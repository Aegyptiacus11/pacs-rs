//! C-FIND SCU at study level, Study Root (PS3.4 C.4.1.2, C.6.2; PS3.7 9.1.2, 9.3.2): one
//! C-FIND-RQ, its Pending responses handed over one by one as they arrive, a C-CANCEL-FIND-RQ
//! when the caller has had enough, and the final response.

use std::ops::ControlFlow;
use std::time::Duration;

use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use super::{ConnectError, Connected, connect};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{Command, NO_DATA_SET, PRIORITY_MEDIUM, field, tag};
use crate::message::{self, DataSet, Incoming, MessageError};
use crate::query::{QueryError, STUDY_ROOT_FIND, StudyQuery};
use crate::status::{Status, StatusClass};
use crate::{EXPLICIT_VR_LE, IMPLICIT_VR_LE};

/// Command Data Set Type of a request with an Identifier: 0102H, which PS3.7 Table 9.3-3
/// asks of implementations compatible with earlier editions, and any non-0101H value means
/// a data set follows.
const IDENTIFIER: u16 = 0x0102;

/// The largest Response Identifier accepted.
const MAX_IDENTIFIER_BYTES: usize = 1 << 24;

#[derive(Debug, thiserror::Error)]
pub enum FindError {
    #[error(transparent)]
    Query(#[from] QueryError),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error("the remote AE released or aborted the association before its final response")]
    Ended,
    #[error("the response is not a C-FIND-RSP to message {0}")]
    WrongResponse(u16),
    #[error("a Pending response without an Identifier (PS3.7 9.1.2.1.5)")]
    PendingWithoutIdentifier,
    #[error("a Response Identifier cannot be read: {0}")]
    Identifier(String),
    #[error("the remote AE answered {status}{}", comment.as_deref().map(|c| format!(": {c}")).unwrap_or_default())]
    Failed {
        status: Status,
        comment: Option<String>,
    },
}

/// How a C-FIND ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindOutcome {
    /// The final status: Success, Cancel, or a Warning (matching ended early, PS3.7
    /// 9.1.2.1.6, so the matches may be incomplete).
    pub status: Status,
    /// Matches handed to the caller.
    pub matches: usize,
    /// Pending responses that did not meet the query, dropped by the SCU's own filtering.
    pub filtered_out: usize,
    /// Whether pacs-rs sent C-CANCEL-FIND-RQ because the caller stopped.
    pub cancelled: bool,
    /// Whether any Pending response was FF01: Optional Keys not supported for matching.
    pub optional_keys_unsupported: bool,
}

impl FindOutcome {
    /// Every match was seen: Success, without a cancel.
    pub fn complete(&self) -> bool {
        self.status == Status::SUCCESS && !self.cancelled
    }
}

/// Search `remote`'s studies as `calling`. Each match is handed to `on_match` as it
/// arrives; `ControlFlow::Break` sends C-CANCEL-FIND-RQ, and responses still in flight are
/// read and dropped until the final one (PS3.4 C.4.1.2.1). `timeout` bounds the connection
/// and every wait for the next response.
pub async fn find_studies<F>(
    remote: &RemoteAe,
    calling: &AeTitle,
    query: &StudyQuery,
    timeout: Duration,
    mut on_match: F,
) -> Result<FindOutcome, FindError>
where
    F: FnMut(InMemDicomObject) -> ControlFlow<()>,
{
    let query = query.clone().checked()?;
    let Connected {
        mut association,
        context,
        transfer_syntax,
    } = connect(
        remote,
        calling,
        STUDY_ROOT_FIND,
        &[EXPLICIT_VR_LE, IMPLICIT_VR_LE],
        timeout,
        timeout,
    )
    .await?;
    let ts = TransferSyntaxRegistry
        .get(&transfer_syntax)
        .ok_or_else(|| {
            FindError::Identifier(format!("unknown transfer syntax {transfer_syntax}"))
        })?;
    let mut identifier = Vec::new();
    query
        .identifier()
        .write_dataset_with_ts(&mut identifier, ts)
        .map_err(|e| FindError::Identifier(e.to_string()))?;

    let message_id = 1;
    let request = c_find_rq(message_id);
    let mut outcome = FindOutcome {
        status: Status::SUCCESS,
        matches: 0,
        filtered_out: 0,
        cancelled: false,
        optional_keys_unsupported: false,
    };
    let result = async {
        message::send(&mut association, context, &request, Some(&identifier)).await?;
        loop {
            let response = match message::receive(&mut association, MAX_IDENTIFIER_BYTES).await? {
                Incoming::Message(response) => response,
                Incoming::Released | Incoming::Aborted => return Err(FindError::Ended),
            };
            let command = &response.command;
            if command.command_field().ok() != Some(field::C_FIND_RSP)
                || command.us(tag::MESSAGE_ID_BEING_RESPONDED_TO).ok() != Some(message_id)
            {
                return Err(FindError::WrongResponse(message_id));
            }
            let status = command.status().map_err(MessageError::from)?;
            match status.class() {
                StatusClass::Pending => {
                    let DataSet::Received(bytes) = response.data else {
                        return Err(FindError::PendingWithoutIdentifier);
                    };
                    if outcome.cancelled {
                        continue;
                    }
                    outcome.optional_keys_unsupported |= status == Status::PENDING_OPTIONAL_KEYS;
                    let found =
                        super::read_identifier(&bytes, ts).map_err(FindError::Identifier)?;
                    if !level_is_study(&found) {
                        return Err(FindError::Identifier(
                            "Query/Retrieve Level is not STUDY".into(),
                        ));
                    }
                    if !query.accepts(&found) {
                        outcome.filtered_out += 1;
                        continue;
                    }
                    outcome.matches += 1;
                    if on_match(found).is_break() {
                        outcome.cancelled = true;
                        message::send(&mut association, context, &c_cancel_rq(message_id), None)
                            .await?;
                    }
                }
                // PS3.4 C.4.1.2.1: Success, Cancel or Failure ends the Pending responses
                StatusClass::Success | StatusClass::Cancel | StatusClass::Warning => {
                    outcome.status = status;
                    return Ok(());
                }
                StatusClass::Failure | StatusClass::Unknown => {
                    return Err(FindError::Failed {
                        status,
                        comment: command.text(tag::ERROR_COMMENT).map(str::to_owned),
                    });
                }
            }
        }
    }
    .await;
    match result {
        Ok(()) => {
            let _ = association.release().await;
            Ok(outcome)
        }
        // a final Failure ends the operation, not the association
        Err(e @ FindError::Failed { .. }) => {
            let _ = association.release().await;
            Err(e)
        }
        Err(e) => {
            let _ = association.abort().await;
            Err(e)
        }
    }
}

/// A response may leave Query/Retrieve Level out; when present it is the request's
/// (PS3.4 C.4.1.1.3.2).
fn level_is_study(found: &InMemDicomObject) -> bool {
    match found.element(tags::QUERY_RETRIEVE_LEVEL) {
        Err(_) => true,
        Ok(e) => e
            .to_str()
            .map(|s| s.trim_matches([' ', '\0']) == "STUDY")
            .unwrap_or(false),
    }
}

/// A C-FIND-RQ, PS3.7 Table 9.3-3.
pub fn c_find_rq(message_id: u16) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, STUDY_ROOT_FIND)
        .with_us(tag::COMMAND_FIELD, field::C_FIND_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, IDENTIFIER)
}

/// A C-CANCEL-FIND-RQ, PS3.7 Table 9.3-5.
pub fn c_cancel_rq(message_id: u16) -> Command {
    Command::new()
        .with_us(tag::COMMAND_FIELD, field::C_CANCEL_RQ)
        .with_us(tag::MESSAGE_ID_BEING_RESPONDED_TO, message_id)
        .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
}
