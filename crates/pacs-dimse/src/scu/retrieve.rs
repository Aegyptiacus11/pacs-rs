//! C-MOVE SCU at study level, Study Root or Patient Root (PS3.4 C.4.2.2, C.6; PS3.7 9.1.4,
//! 9.3.4): the remote AE is asked to send studies to a destination AE, which for pacs-rs is
//! itself, by C-STORE sub-operations on another association; its Pending responses count
//! them as they go, and C-CANCEL-MOVE-RQ stops it.

use std::ops::ControlFlow;
use std::time::Duration;

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use super::find::c_cancel_rq;
use super::{ConnectError, associate};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{Command, PRIORITY_MEDIUM, field, tag};
use crate::message::{DataSet, Incoming, MessageError};
use crate::query::{PATIENT_ROOT_MOVE, Root};
use crate::status::{Status, StatusClass};
use crate::{EXPLICIT_VR_LE, IMPLICIT_VR_LE};

/// Study Root Query/Retrieve Information Model - MOVE (PS3.4 C.6.2.3).
pub const STUDY_ROOT_MOVE: &str = "1.2.840.10008.5.1.4.1.2.2.2";

/// Command Data Set Type of a request with an Identifier (PS3.7 Table 9.3-9, note).
const IDENTIFIER: u16 = 0x0102;

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error("{0}")]
    Request(String),
    #[error(transparent)]
    Connect(#[from] ConnectError),
    #[error(transparent)]
    Message(#[from] MessageError),
    #[error("the remote AE released or aborted the association before its final response")]
    Ended,
    #[error("the response is not a C-MOVE-RSP to message {0}")]
    WrongResponse(u16),
    #[error("a response Identifier cannot be read: {0}")]
    Identifier(String),
    #[error("{}", failure_text(.status, .comment, .destination))]
    Failed {
        status: Status,
        comment: Option<String>,
        destination: String,
        progress: Progress,
        failed_instances: Vec<String>,
    },
}

fn failure_text(status: &Status, comment: &Option<String>, destination: &str) -> String {
    let mut text = if *status == Status::MOVE_DESTINATION_UNKNOWN {
        format!(
            "the remote AE does not know the move destination {destination}; it has to be \
             registered there with pacs-rs's host and port ({status})"
        )
    } else {
        format!("the remote AE answered {status}")
    };
    if let Some(comment) = comment {
        text.push_str(": ");
        text.push_str(comment);
    }
    text
}

/// The sub-operation counts of a C-MOVE response (PS3.4 C.4.2.1.6-9). A count the response
/// leaves out, as the final ones may, is `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub remaining: Option<u16>,
    pub completed: Option<u16>,
    pub failed: Option<u16>,
    pub warning: Option<u16>,
}

impl Progress {
    fn of(command: &Command) -> Self {
        let count = |element| command.us(element).ok();
        Progress {
            remaining: count(tag::NUMBER_OF_REMAINING_SUB_OPERATIONS),
            completed: count(tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS),
            failed: count(tag::NUMBER_OF_FAILED_SUB_OPERATIONS),
            warning: count(tag::NUMBER_OF_WARNING_SUB_OPERATIONS),
        }
    }

    /// The final response's counts, each one it leaves out taken from the last Pending.
    fn or(self, earlier: Progress) -> Progress {
        Progress {
            remaining: self.remaining,
            completed: self.completed.or(earlier.completed),
            failed: self.failed.or(earlier.failed),
            warning: self.warning.or(earlier.warning),
        }
    }
}

/// How a C-MOVE ended, short of a Failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveOutcome {
    /// Success (every sub-operation done), Warning (some failed or warned) or Cancel.
    pub status: Status,
    pub progress: Progress,
    /// Failed SOP Instance UID List (0008,0058), from a Warning or Cancel response.
    pub failed_instances: Vec<String>,
    /// Whether pacs-rs sent C-CANCEL-MOVE-RQ because the caller stopped.
    pub cancelled: bool,
}

/// Ask `remote`, as `calling`, to send the studies `study_uids` to `destination` (List of
/// UID Matching at STUDY level, PS3.4 C.4.2.2.1). Each Pending response's counts go to
/// `on_progress`; `ControlFlow::Break` sends C-CANCEL-MOVE-RQ. `timeout` bounds the
/// connection, `response_timeout` the wait between two responses, which can be long while
/// the remote AE sends a large study.
///
/// Both Query/Retrieve Information Models are proposed, Study Root used when accepted. In
/// Patient Root the studies are named under their patient, so `patient_id`, the Patient ID
/// they all belong to, is needed then; a remote AE that takes Patient Root only is refused
/// without it, before anything is asked.
#[allow(clippy::too_many_arguments)]
pub async fn move_studies<F>(
    remote: &RemoteAe,
    calling: &AeTitle,
    patient_id: Option<&str>,
    study_uids: &[String],
    destination: &AeTitle,
    timeout: Duration,
    response_timeout: Duration,
    mut on_progress: F,
) -> Result<MoveOutcome, MoveError>
where
    F: FnMut(Progress) -> ControlFlow<()>,
{
    if study_uids.is_empty() {
        return Err(MoveError::Request("no Study Instance UID to move".into()));
    }
    if let Some(bad) = study_uids.iter().find(|u| !pacs_core::is_valid_uid(u)) {
        return Err(MoveError::Request(format!("{bad:?} is not a UID")));
    }
    let patient_id = patient_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(id) = patient_id {
        check_patient_id(id).map_err(MoveError::Request)?;
    }
    let syntaxes = vec![EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()];
    let (mut association, accepted) = associate(
        remote,
        calling,
        &[
            (STUDY_ROOT_MOVE.to_owned(), syntaxes.clone()),
            (PATIENT_ROOT_MOVE.to_owned(), syntaxes),
        ],
        timeout,
        response_timeout,
    )
    .await?;
    let chosen = [
        (STUDY_ROOT_MOVE, Root::Study),
        (PATIENT_ROOT_MOVE, Root::Patient),
    ]
    .into_iter()
    .find_map(|(sop_class, root)| {
        accepted
            .iter()
            .find(|context| context.abstract_syntax == sop_class)
            .map(|context| (context.clone(), root, sop_class))
    });
    let Some((context, root, sop_class)) = chosen else {
        association.abort().await;
        return Err(ConnectError::NotAccepted(STUDY_ROOT_MOVE).into());
    };
    let named = match (root, patient_id) {
        (Root::Study, _) => None,
        (Root::Patient, Some(id)) => Some(id),
        (Root::Patient, None) => {
            association.abort().await;
            return Err(MoveError::Request(format!(
                "{} retrieves in Patient Root only, where a study is named under its patient: \
                 give the Patient ID of the studies",
                remote.id
            )));
        }
    };
    let Some(ts) = TransferSyntaxRegistry.get(&context.transfer_syntax) else {
        association.abort().await;
        return Err(MoveError::Identifier(format!(
            "unknown transfer syntax {}",
            context.transfer_syntax
        )));
    };
    let context = context.id;
    let mut identifier = Vec::new();
    if let Err(e) =
        retrieve_identifier(named, study_uids).write_dataset_with_ts(&mut identifier, ts)
    {
        association.abort().await;
        return Err(MoveError::Identifier(e.to_string()));
    }

    let message_id = 1;
    let request = c_move_rq_of(sop_class, message_id, destination);
    let mut last = Progress::default();
    let mut cancelled = false;
    let result = async {
        association
            .send(context, &request, Some(&identifier))
            .await?;
        loop {
            let response = match association.receive(1 << 24).await? {
                Incoming::Message(response) => response,
                Incoming::Released | Incoming::Aborted => return Err(MoveError::Ended),
            };
            let command = &response.command;
            if command.command_field().ok() != Some(field::C_MOVE_RSP)
                || command.us(tag::MESSAGE_ID_BEING_RESPONDED_TO).ok() != Some(message_id)
            {
                return Err(MoveError::WrongResponse(message_id));
            }
            let status = command.status().map_err(MessageError::from)?;
            let progress = Progress::of(command);
            if status.class() == StatusClass::Pending {
                last = progress;
                if !cancelled && on_progress(progress).is_break() {
                    cancelled = true;
                    association
                        .send(context, &c_cancel_rq(message_id), None)
                        .await?;
                }
                continue;
            }
            // PS3.4 C.4.2.1.4.2: Warning, Cancel and Failure may carry the Failed SOP
            // Instance UID List
            let failed_instances = match response.data {
                DataSet::Received(bytes) => failed_list(&bytes, ts)?,
                _ => Vec::new(),
            };
            let progress = progress.or(last);
            return match status.class() {
                StatusClass::Success | StatusClass::Warning | StatusClass::Cancel => {
                    Ok(MoveOutcome {
                        status,
                        progress,
                        failed_instances,
                        cancelled,
                    })
                }
                _ => Err(MoveError::Failed {
                    status,
                    comment: command.text(tag::ERROR_COMMENT).map(str::to_owned),
                    destination: destination.to_string(),
                    progress,
                    failed_instances,
                }),
            };
        }
    }
    .await;
    match &result {
        Ok(_) | Err(MoveError::Failed { .. }) => {
            association.release().await;
        }
        Err(_) => {
            association.abort().await;
        }
    }
    result
}

/// Whether `id` can name the patient of a retrieval: one Patient ID (LO), matched by Single
/// Value Matching (PS3.4 C.4.2.2.1), so no wild card, no backslash, 64 characters at most.
pub fn check_patient_id(id: &str) -> Result<(), String> {
    if id.contains(['*', '?', '\\']) || id.chars().count() > 64 || id.chars().any(char::is_control)
    {
        return Err(format!(
            "{id:?} is not one Patient ID: no wild card, backslash or control character, 64 \
             characters at most"
        ));
    }
    Ok(())
}

/// The Request Identifier (PS3.4 C.4.2.1.4.1): Query/Retrieve Level STUDY and the Study
/// Instance UIDs, one or a list.
pub fn move_identifier(study_uids: &[String]) -> InMemDicomObject {
    retrieve_identifier(None, study_uids)
}

/// The Request Identifier of a retrieval at STUDY level: in Patient Root, `patient_id`, the
/// unique key of the level above (PS3.4 C.4.2.2.1), then the Study Instance UIDs.
pub fn retrieve_identifier(patient_id: Option<&str>, study_uids: &[String]) -> InMemDicomObject {
    let mut identifier = InMemDicomObject::new_empty();
    identifier.put(DataElement::new(
        tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        PrimitiveValue::from("STUDY"),
    ));
    if let Some(patient_id) = patient_id {
        if !patient_id.is_ascii() {
            identifier.put(DataElement::new(
                tags::SPECIFIC_CHARACTER_SET,
                VR::CS,
                PrimitiveValue::from("ISO_IR 192"),
            ));
        }
        identifier.put(DataElement::new(
            tags::PATIENT_ID,
            VR::LO,
            PrimitiveValue::from(patient_id),
        ));
    }
    identifier.put(DataElement::new(
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        PrimitiveValue::Strs(study_uids.iter().cloned().collect()),
    ));
    identifier
}

fn failed_list(
    bytes: &[u8],
    ts: &dicom_encoding::TransferSyntax,
) -> Result<Vec<String>, MoveError> {
    let identifier = super::read_identifier(bytes, ts).map_err(MoveError::Identifier)?;
    Ok(identifier
        .element(tags::FAILED_SOP_INSTANCE_UID_LIST)
        .ok()
        .and_then(|e| e.to_multi_str().ok())
        .map(|uids| {
            uids.iter()
                .map(|u| u.trim_end_matches(['\0', ' ']).to_owned())
                .filter(|u| !u.is_empty())
                .collect()
        })
        .unwrap_or_default())
}

/// A C-MOVE-RQ of Study Root, PS3.7 Table 9.3-9.
pub fn c_move_rq(message_id: u16, destination: &AeTitle) -> Command {
    c_move_rq_of(STUDY_ROOT_MOVE, message_id, destination)
}

/// A C-MOVE-RQ of the MOVE SOP class `sop_class`, PS3.7 Table 9.3-9.
pub fn c_move_rq_of(sop_class: &str, message_id: u16, destination: &AeTitle) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_MOVE_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_text(tag::MOVE_DESTINATION, destination.as_str())
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, IDENTIFIER)
}
