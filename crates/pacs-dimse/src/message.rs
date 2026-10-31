//! DIMSE messages over an association: a Command Set, then a Data Set when Command Data
//! Set Type says one follows (PS3.7 6.3), each carried in Presentation Data Values whose
//! Message Control Header marks command or data and the last fragment (PS3.8 E.2).

use dicom_ul::Pdu;
use dicom_ul::association::AsyncAssociation;
use dicom_ul::pdu::{PDataValue, PDataValueType};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::command::{Command, CommandError};

/// A whole message as received.
#[derive(Debug)]
pub struct Message {
    /// The presentation context it came on: every fragment of a message uses the same one.
    pub context_id: u8,
    pub command: Command,
    pub data: DataSet,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DataSet {
    /// Command Data Set Type said none follows.
    None,
    Received(Vec<u8>),
    /// Larger than the limit: read to its last fragment and dropped, `size` bytes in all.
    Dropped {
        size: usize,
    },
}

/// What the peer sent next.
#[derive(Debug)]
pub enum Incoming {
    Message(Message),
    /// An A-RELEASE-RQ, already answered with A-RELEASE-RP.
    Released,
    /// An A-ABORT.
    Aborted,
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error(transparent)]
    Ul(#[from] dicom_ul::association::Error),
    #[error("command set: {0}")]
    Command(#[from] CommandError),
    #[error("protocol: {0}")]
    Protocol(String),
}

fn protocol(message: impl Into<String>) -> MessageError {
    MessageError::Protocol(message.into())
}

/// Read the next message, keeping at most `max_data` bytes of its data set.
///
/// PS3.8 E.2 and PS3.7 6.3: the command's fragments come first and the last of them is
/// marked; the data set's fragments follow, on the same presentation context, until one
/// marked last. Anything else is a protocol error, which the caller answers with A-ABORT.
pub async fn receive<A, S>(association: &mut A, max_data: usize) -> Result<Incoming, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut context_id = None;
    let mut command_bytes = Vec::new();
    let mut command: Option<Command> = None;
    let mut data = Vec::new();
    let mut size = 0usize;
    loop {
        let values = match AsyncAssociation::receive(association).await? {
            Pdu::PData { data } => data,
            Pdu::ReleaseRQ if context_id.is_none() => {
                AsyncAssociation::send(association, &Pdu::ReleaseRP).await?;
                return Ok(Incoming::Released);
            }
            Pdu::AbortRQ { .. } => return Ok(Incoming::Aborted),
            other => return Err(protocol(format!("unexpected {}", pdu_name(&other)))),
        };
        for PDataValue {
            presentation_context_id,
            value_type,
            is_last,
            data: fragment,
        } in values
        {
            match context_id {
                None => context_id = Some(presentation_context_id),
                Some(id) if id != presentation_context_id => {
                    return Err(protocol(format!(
                        "a message began on presentation context {id} and went on on {presentation_context_id}"
                    )));
                }
                Some(_) => {}
            }
            match (value_type, &command) {
                (PDataValueType::Command, None) => {
                    command_bytes.extend_from_slice(&fragment);
                    if is_last {
                        let parsed = Command::decode(&command_bytes)?;
                        if !parsed.has_data_set()? {
                            return Ok(Incoming::Message(Message {
                                context_id: presentation_context_id,
                                command: parsed,
                                data: DataSet::None,
                            }));
                        }
                        command = Some(parsed);
                    }
                }
                (PDataValueType::Command, Some(_)) => {
                    return Err(protocol("a command fragment after the command's last one"));
                }
                (PDataValueType::Data, None) => {
                    return Err(protocol("a data fragment before the command's last one"));
                }
                (PDataValueType::Data, Some(_)) => {
                    size += fragment.len();
                    if size <= max_data {
                        data.extend_from_slice(&fragment);
                    } else if !data.is_empty() {
                        data = Vec::new();
                    }
                    if is_last {
                        let data = if size <= max_data {
                            DataSet::Received(data)
                        } else {
                            DataSet::Dropped { size }
                        };
                        return Ok(Incoming::Message(Message {
                            context_id: presentation_context_id,
                            command: command.take().expect("matched Some"),
                            data,
                        }));
                    }
                }
            }
        }
    }
}

/// Send a message: the command in one fragment, then the data set, if any, in fragments
/// that fit the peer's Maximum Length Received (PS3.8 D.1: a PDV item adds 6 bytes).
pub async fn send<A, S>(
    association: &mut A,
    context_id: u8,
    command: &Command,
    data: Option<&[u8]>,
) -> Result<(), MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    AsyncAssociation::send(
        association,
        &Pdu::PData {
            data: vec![PDataValue {
                presentation_context_id: context_id,
                value_type: PDataValueType::Command,
                is_last: true,
                data: command.encode(),
            }],
        },
    )
    .await?;
    if let Some(data) = data {
        let max = association.peer_max_pdu_length() as usize;
        let chunk = if max > 6 { max - 6 } else { 16384 - 6 };
        let mut fragments = data.chunks(chunk).peekable();
        if fragments.peek().is_none() {
            return Err(protocol("an empty data set cannot be sent"));
        }
        while let Some(fragment) = fragments.next() {
            AsyncAssociation::send(
                association,
                &Pdu::PData {
                    data: vec![PDataValue {
                        presentation_context_id: context_id,
                        value_type: PDataValueType::Data,
                        is_last: fragments.peek().is_none(),
                        data: fragment.to_vec(),
                    }],
                },
            )
            .await?;
        }
    }
    Ok(())
}

fn pdu_name(pdu: &Pdu) -> &'static str {
    match pdu {
        Pdu::Unknown { .. } => "unknown PDU",
        Pdu::AssociationRQ(_) => "A-ASSOCIATE-RQ",
        Pdu::AssociationAC(_) => "A-ASSOCIATE-AC",
        Pdu::AssociationRJ(_) => "A-ASSOCIATE-RJ",
        Pdu::PData { .. } => "P-DATA-TF",
        Pdu::ReleaseRQ => "A-RELEASE-RQ in the middle of a message",
        Pdu::ReleaseRP => "A-RELEASE-RP",
        Pdu::AbortRQ { .. } => "A-ABORT",
    }
}
