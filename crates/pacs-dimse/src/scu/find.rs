//! C-FIND SCU for studies, in Study Root or Patient Root (PS3.4 C.4.1.2, C.6; PS3.7 9.1.2,
//! 9.3.2): C-FIND-RQ, its Pending responses handed over one by one as they arrive, a
//! C-CANCEL-FIND-RQ when the caller has had enough, and the final response.

use std::ops::ControlFlow;
use std::time::Duration;

use dicom_core::header::Header;
use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use super::{ConnectError, associate};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{Command, NO_DATA_SET, PRIORITY_MEDIUM, field, tag};
use crate::message::{DataSet, Incoming, MessageError};
use crate::query::{PATIENT_ROOT_FIND, QueryError, Root, STUDY_ROOT_FIND, StudyQuery};
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
///
/// Both Query/Retrieve Information Models are proposed; Study Root is used when the remote
/// AE accepts it. In Patient Root a study is below its patient, so the search goes in two
/// steps on the one association: the patients that meet Patient ID and Patient's Name, then
/// the studies of each, by its Patient ID (PS3.4 C.6.1.1). A study found that way carries its
/// patient's attributes, as one found in Study Root does.
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
    let syntaxes = vec![EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()];
    let (mut association, accepted) = associate(
        remote,
        calling,
        &[
            (STUDY_ROOT_FIND.to_owned(), syntaxes.clone()),
            (PATIENT_ROOT_FIND.to_owned(), syntaxes),
        ],
        timeout,
        timeout,
    )
    .await?;
    let chosen = [
        (STUDY_ROOT_FIND, Root::Study),
        (PATIENT_ROOT_FIND, Root::Patient),
    ]
    .into_iter()
    .find_map(|(sop_class, root)| {
        accepted
            .iter()
            .find(|context| context.abstract_syntax == sop_class)
            .map(|context| (context.clone(), root))
    });
    let Some((context, root)) = chosen else {
        association.abort().await;
        return Err(ConnectError::NotAccepted(STUDY_ROOT_FIND).into());
    };
    let Some(ts) = TransferSyntaxRegistry.get(&context.transfer_syntax) else {
        association.abort().await;
        return Err(FindError::Identifier(format!(
            "unknown transfer syntax {}",
            context.transfer_syntax
        )));
    };
    let mut outcome = FindOutcome {
        status: Status::SUCCESS,
        matches: 0,
        filtered_out: 0,
        cancelled: false,
        optional_keys_unsupported: false,
    };
    let mut exchange = Exchange {
        association: &mut association,
        context: context.id,
        sop_class: context_sop_class(root),
        ts,
        message_id: 0,
    };
    let result = async {
        // a study found is handed over, unless the SCU's own filtering drops it
        let mut hand_over = |found: InMemDicomObject, outcome: &mut FindOutcome| {
            if !query.accepts(&found) {
                outcome.filtered_out += 1;
                return ControlFlow::Continue(());
            }
            outcome.matches += 1;
            on_match(found)
        };
        match root {
            Root::Study => {
                let status = exchange
                    .find(
                        &query.identifier(),
                        "STUDY",
                        &mut outcome,
                        |found, outcome| hand_over(found, outcome),
                    )
                    .await?;
                outcome.status = status;
            }
            Root::Patient => {
                let mut patients = Vec::new();
                let status = exchange
                    .find(
                        &query.patient_identifier(),
                        "PATIENT",
                        &mut outcome,
                        |found, outcome| {
                            if query.accepts_patient(&found) {
                                patients.push(found);
                            } else {
                                outcome.filtered_out += 1;
                            }
                            ControlFlow::Continue(())
                        },
                    )
                    .await?;
                outcome.status = status;
                for patient in patients {
                    if outcome.cancelled || outcome.status != Status::SUCCESS {
                        break;
                    }
                    // a patient without a Patient ID cannot be named one level down
                    let Some(patient_id) = text_of(&patient, tags::PATIENT_ID) else {
                        continue;
                    };
                    let identifier = query.patient_study_identifier(&patient_id);
                    let status = exchange
                        .find(&identifier, "STUDY", &mut outcome, |mut found, outcome| {
                            for element in patient.iter() {
                                let tag = element.tag();
                                if tag != tags::QUERY_RETRIEVE_LEVEL
                                    && tag != tags::SPECIFIC_CHARACTER_SET
                                    && text_of(&found, tag).is_none()
                                {
                                    found.put(element.clone());
                                }
                            }
                            hand_over(found, outcome)
                        })
                        .await?;
                    outcome.status = status;
                }
            }
        }
        Ok(())
    }
    .await;
    match result {
        Ok(()) => {
            association.release().await;
            Ok(outcome)
        }
        // a final Failure ends the operation, not the association
        Err(e @ FindError::Failed { .. }) => {
            association.release().await;
            Err(e)
        }
        Err(e) => {
            association.abort().await;
            Err(e)
        }
    }
}

fn context_sop_class(root: Root) -> &'static str {
    match root {
        Root::Study => STUDY_ROOT_FIND,
        Root::Patient => PATIENT_ROOT_FIND,
    }
}

/// The value of `tag` in a Response Identifier, trimmed; `None` when absent or empty.
fn text_of(found: &InMemDicomObject, tag: dicom_core::Tag) -> Option<String> {
    let text = found.element(tag).ok()?.to_str().ok()?;
    let text = text.trim_matches([' ', '\0']);
    (!text.is_empty()).then(|| text.to_owned())
}

/// The C-FIND requests of one search, on one association, one after the other.
struct Exchange<'a> {
    association: &'a mut super::Link,
    context: u8,
    sop_class: &'static str,
    ts: &'static dicom_encoding::TransferSyntax,
    message_id: u16,
}

impl Exchange<'_> {
    /// One C-FIND at `level`: each Pending match goes to `on_found`, whose
    /// `ControlFlow::Break` cancels; the final status, short of a Failure.
    async fn find<G>(
        &mut self,
        identifier: &InMemDicomObject,
        level: &str,
        outcome: &mut FindOutcome,
        mut on_found: G,
    ) -> Result<Status, FindError>
    where
        G: FnMut(InMemDicomObject, &mut FindOutcome) -> ControlFlow<()>,
    {
        let mut bytes = Vec::new();
        identifier
            .write_dataset_with_ts(&mut bytes, self.ts)
            .map_err(|e| FindError::Identifier(e.to_string()))?;
        self.message_id += 1;
        let message_id = self.message_id;
        self.association
            .send(
                self.context,
                &c_find_rq_of(self.sop_class, message_id),
                Some(&bytes),
            )
            .await?;
        let mut cancelled = false;
        loop {
            let response = match self.association.receive(MAX_IDENTIFIER_BYTES).await? {
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
                    if cancelled {
                        continue;
                    }
                    outcome.optional_keys_unsupported |= status == Status::PENDING_OPTIONAL_KEYS;
                    let found =
                        super::read_identifier(&bytes, self.ts).map_err(FindError::Identifier)?;
                    if !level_is(&found, level) {
                        return Err(FindError::Identifier(format!(
                            "Query/Retrieve Level is not {level}"
                        )));
                    }
                    if on_found(found, outcome).is_break() {
                        cancelled = true;
                        outcome.cancelled = true;
                        self.association
                            .send(self.context, &c_cancel_rq(message_id), None)
                            .await?;
                    }
                }
                // PS3.4 C.4.1.2.1: Success, Cancel or Failure ends the Pending responses
                StatusClass::Success | StatusClass::Cancel | StatusClass::Warning => {
                    return Ok(status);
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
}

/// A response may leave Query/Retrieve Level out; when present it is the request's
/// (PS3.4 C.4.1.1.3.2).
fn level_is(found: &InMemDicomObject, level: &str) -> bool {
    match found.element(tags::QUERY_RETRIEVE_LEVEL) {
        Err(_) => true,
        Ok(e) => e
            .to_str()
            .map(|s| s.trim_matches([' ', '\0']) == level)
            .unwrap_or(false),
    }
}

/// A C-FIND-RQ of Study Root, PS3.7 Table 9.3-3.
pub fn c_find_rq(message_id: u16) -> Command {
    c_find_rq_of(STUDY_ROOT_FIND, message_id)
}

/// A C-FIND-RQ of the FIND SOP class `sop_class`, PS3.7 Table 9.3-3.
pub fn c_find_rq_of(sop_class: &str, message_id: u16) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
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
