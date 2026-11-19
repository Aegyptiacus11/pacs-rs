//! C-STORE SCU (PS3.4 Annex B; PS3.7 9.1.1, 9.3.1): instances of the archive sent to another
//! AE, as the sub-operations of a C-MOVE or of a C-GET are.
//!
//! An instance goes in the transfer syntax it is stored in when the peer accepts that one,
//! and otherwise decoded to an uncompressed one the peer accepts: every AE takes Implicit VR
//! Little Endian (PS3.5 10.1). With no presentation context accepted for its SOP class it
//! is not sent, which a retrieval counts as a failed sub-operation (PS3.4 C.4.2.3.1).

use std::time::Duration;

use bytes::Bytes;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::OpenFileOptions;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use super::{ConnectError, Link, associate};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{Command, DATA_SET_PRESENT, PRIORITY_MEDIUM, field, tag};
use crate::message::{Incoming, MessageError};
use crate::status::{Status, StatusClass};
use crate::{EXPLICIT_VR_LE, IMPLICIT_VR_LE};

/// A presentation context the peer accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Accepted {
    pub id: u8,
    pub abstract_syntax: String,
    pub transfer_syntax: String,
}

/// The C-MOVE whose sub-operation a C-STORE is: its requester's AE title and the Message ID
/// of its request (PS3.7 9.1.1.1.5-6), which the C-STORE-RQ then carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Originator {
    pub ae_title: String,
    pub message_id: u16,
}

/// Why an instance was not stored by the peer.
#[derive(Debug, thiserror::Error)]
pub enum SendError {
    #[error("the peer accepted no presentation context for SOP Class {0}")]
    NoContext(String),
    #[error("the instance cannot be written in a transfer syntax the peer accepted: {0}")]
    Encode(String),
    #[error("the peer answered {status}{}", comment.as_deref().map(|c| format!(": {c}")).unwrap_or_default())]
    Refused {
        status: Status,
        comment: Option<String>,
    },
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error("the peer released or aborted the association instead of answering")]
    Ended,
    #[error("the response is not a C-STORE-RSP to message {0}")]
    WrongResponse(u16),
}

impl SendError {
    /// Whether the association can carry the next instance: it can after an instance that
    /// was not sent or that the peer refused, not once the exchange itself has broken.
    pub fn association_holds(&self) -> bool {
        matches!(
            self,
            SendError::NoContext(_) | SendError::Encode(_) | SendError::Refused { .. }
        )
    }
}

/// The presentation contexts to propose for instances of these (SOP Class, stored transfer
/// syntax) pairs: for each SOP class, one context per stored transfer syntax that is not an
/// uncompressed little endian one, and one with Explicit and Implicit VR Little Endian. A
/// context is answered with a single transfer syntax (PS3.8 9.3.3.2), so the stored one
/// gets a context of its own for the peer to take or leave.
///
/// An association has at most 128 contexts (PS3.8 9.3.2.2): what does not fit is left out,
/// compressed alternatives first, and an instance of a SOP class left out is then not sent.
pub fn proposal(stored: &[(String, String)]) -> Vec<(String, Vec<String>)> {
    const MOST: usize = 128;
    let mut classes: Vec<&str> = Vec::new();
    for (sop_class, _) in stored {
        if !classes.contains(&sop_class.as_str()) {
            classes.push(sop_class);
        }
    }
    let mut contexts: Vec<(String, Vec<String>)> = classes
        .iter()
        .take(MOST)
        .map(|sop_class| {
            (
                (*sop_class).to_owned(),
                vec![EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()],
            )
        })
        .collect();
    for (sop_class, transfer_syntax) in stored {
        if contexts.len() == MOST {
            break;
        }
        let context = (sop_class.clone(), vec![transfer_syntax.clone()]);
        if !pacs_core::codec::is_native(transfer_syntax)
            && classes.iter().take(MOST).any(|class| class == sop_class)
            && !contexts.contains(&context)
        {
            contexts.push(context);
        }
    }
    contexts
}

/// Where the data set of a Part 10 file starts: after the preamble, "DICM" and the File
/// Meta Information, whose first element gives its length (PS3.10 7.1).
fn data_set_of(file: &[u8]) -> Result<&[u8], SendError> {
    let not_a_file = || SendError::Encode("the stored file is not a DICOM Part 10 file".into());
    if file.get(128..132) != Some(b"DICM") || file.get(132..136) != Some(&[2, 0, 0, 0]) {
        return Err(not_a_file());
    }
    let length = file
        .get(140..144)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
        .ok_or_else(not_a_file)?;
    file.get(144 + length..).ok_or_else(not_a_file)
}

/// The data set of a Part 10 file, written again in the uncompressed transfer syntax
/// `target`.
fn rewritten(file: &[u8], target: &str) -> Result<Vec<u8>, SendError> {
    let encode = |e: String| SendError::Encode(e);
    let ts = TransferSyntaxRegistry
        .get(target)
        .ok_or_else(|| encode(format!("unknown transfer syntax {target}")))?;
    let object = OpenFileOptions::new()
        .read_preamble(dicom_object::file::ReadPreamble::Always)
        .from_reader(&*pacs_core::wire::checked(file).map_err(|e| encode(e.to_string()))?)
        .map_err(|e| encode(e.to_string()))?;
    let mut data_set = Vec::new();
    object
        .write_dataset_with_ts(&mut data_set, ts)
        .map_err(|e| encode(e.to_string()))?;
    Ok(data_set)
}

/// The presentation context to send a stored instance on, and its data set written for it.
///
/// The context of the stored transfer syntax first: the file goes as it is. Else an
/// uncompressed one, Explicit before Implicit VR Little Endian, with the pixel data decoded
/// when it is stored compressed. Else a compressed one pacs-rs can encode.
pub fn encoded(
    accepted: &[Accepted],
    file: &[u8],
    sop_class: &str,
    stored: &str,
) -> Result<(u8, Vec<u8>), SendError> {
    let of_class: Vec<&Accepted> = accepted
        .iter()
        .filter(|context| context.abstract_syntax == sop_class)
        .collect();
    if of_class.is_empty() {
        return Err(SendError::NoContext(sop_class.to_owned()));
    }
    let taking = |transfer_syntax: &str| {
        of_class
            .iter()
            .find(|context| context.transfer_syntax == transfer_syntax)
    };
    if let Some(context) = taking(stored) {
        return Ok((context.id, data_set_of(file)?.to_vec()));
    }
    let target = taking(EXPLICIT_VR_LE)
        .or_else(|| taking(IMPLICIT_VR_LE))
        .or_else(|| {
            of_class
                .iter()
                .find(|context| pacs_core::codec::can_encode(&context.transfer_syntax))
        })
        .ok_or_else(|| {
            SendError::Encode(format!(
                "stored in {stored}, and the peer accepted only {}",
                of_class
                    .iter()
                    .map(|context| context.transfer_syntax.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
    let wanted = target.transfer_syntax.as_str();
    if pacs_core::codec::is_native(stored) {
        // no pixel data to decode: the same values under another encoding
        return Ok((target.id, rewritten(file, wanted)?));
    }
    // what the codecs write natively is Explicit VR Little Endian
    let through = if pacs_core::codec::can_encode(wanted) {
        wanted
    } else {
        EXPLICIT_VR_LE
    };
    let transcoded = pacs_core::Instance::from_bytes(file)
        .map_err(|e| SendError::Encode(e.to_string()))?
        .transcoded(through, false)
        .map_err(|e| SendError::Encode(e.to_string()))?;
    let data_set = if through == wanted {
        data_set_of(&transcoded)?.to_vec()
    } else {
        rewritten(&transcoded, wanted)?
    };
    Ok((target.id, data_set))
}

/// A C-STORE-RQ, PS3.7 Table 9.3-1.
pub fn c_store_rq(
    message_id: u16,
    sop_class: &str,
    sop_instance: &str,
    originator: Option<&Originator>,
) -> Command {
    let mut request = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, DATA_SET_PRESENT)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, sop_instance);
    if let Some(originator) = originator {
        request = request
            .with_text(tag::MOVE_ORIGINATOR_AE_TITLE, &originator.ae_title)
            .with_us(tag::MOVE_ORIGINATOR_MESSAGE_ID, originator.message_id);
    }
    request
}

/// What a C-STORE-RSP to message `message_id` says: its status when the peer stored the
/// instance (Success, or a Warning of PS3.4 Table B.2-1), an error when it did not.
pub fn stored(response: &Command, message_id: u16) -> Result<Status, SendError> {
    if response.command_field().ok() != Some(field::C_STORE_RSP)
        || response.us(tag::MESSAGE_ID_BEING_RESPONDED_TO).ok() != Some(message_id)
    {
        return Err(SendError::WrongResponse(message_id));
    }
    let status = response.status().map_err(MessageError::from)?;
    match status.class() {
        StatusClass::Success | StatusClass::Warning => Ok(status),
        _ => Err(SendError::Refused {
            status,
            comment: response.text(tag::ERROR_COMMENT).map(str::to_owned),
        }),
    }
}

/// An association pacs-rs opened to send instances on.
pub struct Sender {
    link: Link,
    accepted: Vec<Accepted>,
    next_id: u16,
    originator: Option<Originator>,
}

impl Sender {
    /// Associate with `remote` as `calling`, proposing what [`proposal`] makes of
    /// `stored`, the (SOP Class, stored transfer syntax) of every instance to send.
    /// `timeout` bounds the connection and each wait for a response.
    pub async fn open(
        remote: &RemoteAe,
        calling: &AeTitle,
        stored: &[(String, String)],
        timeout: Duration,
        originator: Option<Originator>,
    ) -> Result<Sender, ConnectError> {
        let (link, accepted) =
            associate(remote, calling, &proposal(stored), timeout, timeout).await?;
        Ok(Sender {
            link,
            accepted,
            next_id: 1,
            originator,
        })
    }

    /// The presentation contexts the peer accepted.
    pub fn accepted(&self) -> &[Accepted] {
        &self.accepted
    }

    /// Send one instance: `file` is its Part 10 file as stored, in transfer syntax
    /// `stored`. The status is Success, or a Warning the peer stored it with.
    pub async fn send(
        &mut self,
        file: Bytes,
        sop_class: &str,
        sop_instance: &str,
        stored_in: &str,
    ) -> Result<Status, SendError> {
        let accepted = self.accepted.clone();
        let (class, syntax) = (sop_class.to_owned(), stored_in.to_owned());
        // decoding pixel data is work for another thread than the one that serves
        let (context, data_set) =
            tokio::task::spawn_blocking(move || encoded(&accepted, &file, &class, &syntax))
                .await
                .map_err(|e| SendError::Encode(e.to_string()))??;
        let message_id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        let request = c_store_rq(
            message_id,
            sop_class,
            sop_instance,
            self.originator.as_ref(),
        );
        self.link.send(context, &request, Some(&data_set)).await?;
        match self.link.receive(0).await? {
            Incoming::Message(response) => stored(&response.command, message_id),
            Incoming::Released | Incoming::Aborted => Err(SendError::Ended),
        }
    }

    /// Release the association.
    pub async fn release(self) {
        self.link.release().await;
    }

    /// Abort it: after an error that leaves it unusable.
    pub async fn abort(self) {
        self.link.abort().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
    const MR: &str = "1.2.840.10008.5.1.4.1.1.4";
    const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

    fn pair(sop_class: &str, transfer_syntax: &str) -> (String, String) {
        (sop_class.to_owned(), transfer_syntax.to_owned())
    }

    #[test]
    fn a_proposal_has_the_stored_syntax_and_the_ones_every_ae_takes() {
        let uncompressed = vec![EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()];
        let contexts = proposal(&[
            pair(CT, JPEG_LS),
            pair(CT, JPEG_LS),
            pair(MR, EXPLICIT_VR_LE),
            pair(CT, IMPLICIT_VR_LE),
        ]);
        assert_eq!(
            contexts,
            [
                (CT.to_owned(), uncompressed.clone()),
                (MR.to_owned(), uncompressed.clone()),
                (CT.to_owned(), vec![JPEG_LS.to_owned()]),
            ]
        );
        assert!(proposal(&[]).is_empty());
    }

    #[test]
    fn a_proposal_stops_at_the_128_contexts_an_association_has() {
        let stored: Vec<(String, String)> = (0..200)
            .map(|n| pair(&format!("1.2.3.{n}"), JPEG_LS))
            .collect();
        let contexts = proposal(&stored);
        assert_eq!(contexts.len(), 128);
        // every class that fits can be sent uncompressed; none has its compressed context
        assert!(contexts.iter().all(|(_, syntaxes)| syntaxes.len() == 2));
        // room for some compressed contexts: the first classes get them
        let contexts = proposal(&stored[..100]);
        assert_eq!(contexts.len(), 128);
        assert_eq!(
            contexts[100],
            ("1.2.3.0".to_owned(), vec![JPEG_LS.to_owned()])
        );
    }

    #[test]
    fn a_file_that_is_not_part_10_is_not_sent() {
        let accepted = [Accepted {
            id: 1,
            abstract_syntax: CT.into(),
            transfer_syntax: EXPLICIT_VR_LE.into(),
        }];
        for file in [&b"not a file"[..], &[0u8; 140][..]] {
            let error = encoded(&accepted, file, CT, EXPLICIT_VR_LE).unwrap_err();
            assert!(matches!(error, SendError::Encode(_)), "{error}");
            assert!(error.association_holds());
        }
        let error = encoded(&accepted, b"", MR, EXPLICIT_VR_LE).unwrap_err();
        assert!(matches!(error, SendError::NoContext(_)), "{error}");
    }
}
