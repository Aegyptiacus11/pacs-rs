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
    /// Larger than the spool's bound: written to a file as it arrived, after the Part 10
    /// header [`Spool::header`] gave, so that memory did not grow with it. The file goes
    /// when this does.
    Spooled(SpooledFile),
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

/// A data set written to disk: the file, removed when this is dropped, and its length.
#[derive(Debug)]
pub struct SpooledFile {
    pub path: tempfile::TempPath,
    pub size: u64,
}

impl PartialEq for SpooledFile {
    fn eq(&self, other: &Self) -> bool {
        *self.path == *other.path
    }
}

impl Eq for SpooledFile {}

/// Where data sets larger than `from_bytes` are written as they arrive, and the Part 10
/// header that goes before one: `header` makes it from the presentation context and the
/// command, and says `None` for a message whose data set is not to be spooled.
pub struct Spool<'a> {
    pub dir: &'a std::path::Path,
    pub from_bytes: usize,
    pub header: &'a (dyn Fn(u8, &Command) -> Option<Vec<u8>> + Send + Sync),
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
    receive_spooling(association, max_data, None).await
}

/// [`receive`], a data set that grows past `spool`'s bound written to a file of it from then
/// on ([`DataSet::Spooled`]). A file that cannot be written is an error, which ends the
/// association, never a data set kept in memory after all.
pub async fn receive_spooling<A, S>(
    association: &mut A,
    max_data: usize,
    spool: Option<&Spool<'_>>,
) -> Result<Incoming, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    use tokio::io::AsyncWriteExt;
    let spool_error = |e: std::io::Error| protocol(format!("spooling a data set to disk: {e}"));
    let mut spooled: Option<(tokio::fs::File, tempfile::TempPath, u64)> = None;
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
                (PDataValueType::Data, Some(parsed)) => {
                    size += fragment.len();
                    if size > max_data {
                        // read through and dropped, the spool's file with it
                        data = Vec::new();
                        spooled = None;
                    } else if let Some((file, _, written)) = &mut spooled {
                        file.write_all(&fragment).await.map_err(spool_error)?;
                        *written += fragment.len() as u64;
                    } else {
                        data.extend_from_slice(&fragment);
                        let header = spool
                            .filter(|spool| data.len() > spool.from_bytes)
                            .and_then(|spool| {
                                (spool.header)(presentation_context_id, parsed)
                                    .map(|header| (spool, header))
                            });
                        if let Some((spool, header)) = header {
                            let (file, path) = tempfile::Builder::new()
                                .prefix("pacs-spool-")
                                .tempfile_in(spool.dir)
                                .map_err(spool_error)?
                                .into_parts();
                            let mut file = tokio::fs::File::from_std(file);
                            file.write_all(&header).await.map_err(spool_error)?;
                            file.write_all(&data).await.map_err(spool_error)?;
                            let written = (header.len() + data.len()) as u64;
                            data = Vec::new();
                            spooled = Some((file, path, written));
                        }
                    }
                    if is_last {
                        let data = if size > max_data {
                            DataSet::Dropped { size }
                        } else if let Some((mut file, path, written)) = spooled.take() {
                            file.flush().await.map_err(spool_error)?;
                            DataSet::Spooled(SpooledFile {
                                path,
                                size: written,
                            })
                        } else {
                            DataSet::Received(data)
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
