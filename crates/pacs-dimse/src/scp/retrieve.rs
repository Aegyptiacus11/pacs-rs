//! The Retrieve SCP (PS3.4 C.4.2.3, C.4.3.3) on the Study Root Query/Retrieve Information
//! Model (C.6.2). A C-MOVE's instances go to the Move Destination by C-STORE sub-operations
//! on another association, which pacs-rs opens as a Storage SCU ([`crate::scu::store`]); a
//! C-GET's go to the requester on the association of the C-GET itself, on the presentation
//! contexts for which it took the SCP role (PS3.7 D.3.3.4, PS3.4 C.5.3).
//!
//! - **What is sent**: every instance under the Unique Keys of the Identifier
//!   (C.4.2.1.4.1, C.6.2.1.5): studies, the series of one study, or the instances of one
//!   series, a list of UID at the level of the retrieval and one value above it.
//! - **Where**: for a C-MOVE, to a destination of the configuration, by its AE title; any
//!   other is Refused: Move Destination unknown (A801).
//! - **Responses**: a Pending one after each sub-operation with the Remaining, Completed,
//!   Failed and Warning counts, then Success, Warning or Failure; a C-CANCEL-RQ, looked
//!   for between two sub-operations, ends with Cancel. Warning, Failure and Cancel name the
//!   instances that failed (C.4.2.1.4.2).

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::{Association, AsyncAssociation};
use dicom_ul::pdu::PresentationContextResultReason;
use pacs_index::{Index, InstanceEntry, Kind, Match};
use tokio::io::{AsyncRead, AsyncWrite};

use super::find::Level;
use super::operation::{After, Interruption, Refusal, interruption, refusal};
use super::{Destination, Request};
use crate::ae::AeTitle;
use crate::command::{self, Command, DATA_SET_PRESENT, field, tag};
use crate::message::{self, DataSet, Incoming, MessageError};
use crate::query::STUDY_ROOT_GET;
use crate::scu::retrieve::STUDY_ROOT_MOVE;
use crate::scu::store::{Accepted, Originator, SendError, Sender, c_store_rq, encoded, stored};
use crate::status::{Status, StatusClass};
use crate::{Config, RemoteAe};

/// Whether `sop_class` is a Query/Retrieve Information Model pacs-rs answers C-MOVE on.
pub(super) fn is_move(sop_class: &str) -> bool {
    sop_class == STUDY_ROOT_MOVE
}

/// Whether `sop_class` is a Query/Retrieve Information Model pacs-rs answers C-GET on.
pub(super) fn is_get(sop_class: &str) -> bool {
    sop_class == STUDY_ROOT_GET
}

/// What a retrieval asks for: the Unique Keys of its Identifier (PS3.4 C.4.2.1.4.1).
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Selection {
    /// One study, or at the STUDY level any number.
    studies: Vec<String>,
    /// At the SERIES level the series of the one study; at the IMAGE level one series.
    series: Vec<String>,
    /// At the IMAGE level, the instances of the one series.
    instances: Vec<String>,
}

impl Selection {
    pub(super) fn parse(identifier: &InMemDicomObject) -> Result<Selection, Refusal> {
        let mismatch = |comment: String, tag: Tag| {
            refusal(
                Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                comment,
                Some(tag),
            )
        };
        let text = |tag: Tag| {
            identifier
                .get(tag)
                .and_then(|element| element.to_str().ok())
                .map(|value| value.trim_matches([' ', '\0']).to_owned())
                .unwrap_or_default()
        };
        let level = Level::named(&text(tags::QUERY_RETRIEVE_LEVEL)).ok_or_else(|| {
            mismatch(
                "Query/Retrieve Level is STUDY, SERIES or IMAGE in the Study Root model".into(),
                tags::QUERY_RETRIEVE_LEVEL,
            )
        })?;
        // the Unique Key of the level retrieved may be a list of UID; of a level above it
        // is a single value (C.4.2.1.4.1, note)
        let uids = |tag: Tag, name: &str, several: bool| -> Result<Vec<String>, Refusal> {
            match Match::parse(Kind::Uid, &text(tag)) {
                Ok(Some(Match::Is(uid))) => Ok(vec![uid]),
                Ok(Some(Match::AnyOf(uids))) if several => Ok(uids),
                Ok(Some(_)) => Err(mismatch(
                    format!("{} level takes one {name}", level.name()),
                    tag,
                )),
                Ok(None) => Err(mismatch(
                    format!("{} level needs {name}", level.name()),
                    tag,
                )),
                Err(e) => Err(mismatch(format!("{name}: {e}"), tag)),
            }
        };
        let studies = uids(
            tags::STUDY_INSTANCE_UID,
            "Study Instance UID",
            level == Level::Study,
        )?;
        let series = match level {
            Level::Study => Vec::new(),
            _ => uids(
                tags::SERIES_INSTANCE_UID,
                "Series Instance UID",
                level == Level::Series,
            )?,
        };
        let instances = match level {
            Level::Image => uids(tags::SOP_INSTANCE_UID, "SOP Instance UID", true)?,
            _ => Vec::new(),
        };
        Ok(Selection {
            studies,
            series,
            instances,
        })
    }

    /// The instances selected, as `index` shows them: a study it does not show, or a
    /// series or an instance that is not under the keys above it, has none.
    pub(super) async fn instances(
        &self,
        index: &Index,
    ) -> Result<Vec<InstanceEntry>, pacs_index::Error> {
        let mut selected = Vec::new();
        if !self.instances.is_empty() {
            for instance in &self.instances {
                selected.extend(
                    index
                        .instance(&self.studies[0], &self.series[0], instance)
                        .await?,
                );
            }
        } else if !self.series.is_empty() {
            for series in &self.series {
                selected.extend(index.instances(&self.studies[0], series, None).await?);
            }
        } else {
            for study in &self.studies {
                for series in index.series(study, None).await? {
                    selected.extend(
                        index
                            .instances(study, &series.series_instance_uid, None)
                            .await?,
                    );
                }
            }
        }
        Ok(selected)
    }
}

/// The sub-operations of a retrieval, counted (PS3.4 C.4.2.1.6 to 9).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Counts {
    pub remaining: usize,
    pub completed: usize,
    pub failed: usize,
    pub warning: usize,
    /// The SOP Instance UIDs of the failed ones, for the Failed SOP Instance UID List.
    pub failed_instances: Vec<String>,
}

impl Counts {
    pub(super) fn of(instances: usize) -> Counts {
        Counts {
            remaining: instances,
            ..Counts::default()
        }
    }

    /// One sub-operation is over: stored (with a Warning status or without), or not.
    pub(super) fn count(&mut self, instance: &str, stored: Option<Status>) {
        self.remaining -= 1;
        match stored {
            Some(status) if status.class() == StatusClass::Warning => self.warning += 1,
            Some(_) => self.completed += 1,
            None => {
                self.failed += 1;
                self.failed_instances.push(instance.to_owned());
            }
        }
    }

    /// The status once no sub-operation remains (PS3.4 C.4.2.3.1): Success when all were
    /// stored without a warning, Failure when none was stored, Warning in between.
    pub(super) fn final_status(&self) -> Status {
        if self.failed == 0 && self.warning == 0 {
            Status::SUCCESS
        } else if self.completed == 0 && self.warning == 0 {
            Status::UNABLE_TO_PERFORM_SUB_OPERATIONS
        } else {
            Status::SUB_OPERATIONS_WARNING
        }
    }

    /// A response with `status` and these counts. The number of Remaining sub-operations
    /// is in a Pending response and in a Cancel one, and in no other (C.4.2.1.6).
    pub(super) fn response(
        &self,
        request: &Command,
        status: Status,
        comment: Option<&str>,
    ) -> Command {
        // a count is a US: one over 65535 is written as that
        let count = |n: usize| u16::try_from(n).unwrap_or(u16::MAX);
        let mut response = command::response(request, status, comment)
            .with_us(
                tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS,
                count(self.completed),
            )
            .with_us(tag::NUMBER_OF_FAILED_SUB_OPERATIONS, count(self.failed))
            .with_us(tag::NUMBER_OF_WARNING_SUB_OPERATIONS, count(self.warning));
        if matches!(status.class(), StatusClass::Pending | StatusClass::Cancel) {
            response = response.with_us(
                tag::NUMBER_OF_REMAINING_SUB_OPERATIONS,
                count(self.remaining),
            );
        }
        response
    }

    /// The Identifier of a Warning, Failure or Cancel response: the Failed SOP Instance
    /// UID List (C.4.2.1.4.2). `None` when no sub-operation failed: the response then has
    /// no data set.
    pub(super) fn failed_list(
        &self,
        ts: &dicom_encoding::TransferSyntax,
    ) -> Result<Option<Vec<u8>>, MessageError> {
        if self.failed_instances.is_empty() {
            return Ok(None);
        }
        let mut identifier = InMemDicomObject::new_empty();
        identifier.put(DataElement::new(
            tags::FAILED_SOP_INSTANCE_UID_LIST,
            VR::UI,
            PrimitiveValue::Strs(self.failed_instances.iter().cloned().collect()),
        ));
        let mut bytes = Vec::new();
        identifier
            .write_dataset_with_ts(&mut bytes, ts)
            .map_err(|e| {
                MessageError::Protocol(format!("a response Identifier cannot be written: {e}"))
            })?;
        Ok(Some(bytes))
    }
}

/// One instance a destination stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Sent {
    pub study_instance_uid: String,
    pub sop_class_uid: String,
}

/// What a retrieval came to, for whoever accounts for it.
#[derive(Debug)]
pub(super) struct Retrieved {
    /// The final status.
    pub status: Status,
    pub counts: Counts,
    /// The AE title the instances were to go to, as the request named it.
    pub destination: String,
    /// The host of that AE, when it is one this server sends to.
    pub destination_host: Option<String>,
    /// The instances the destination stored, in order.
    pub sent: Vec<Sent>,
}

/// Send the final response of a retrieval: `status` with the counts and, when a
/// sub-operation failed and the status is not Success, the list of the failed instances.
async fn finish<A, S>(
    association: &mut A,
    request: &Request<'_>,
    command: &Command,
    status: Status,
    counts: &Counts,
    comment: Option<&str>,
) -> Result<(), MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let failed = match (
        status.class(),
        TransferSyntaxRegistry.get(request.transfer_syntax),
    ) {
        (StatusClass::Success, _) | (_, None) => None,
        (_, Some(ts)) => counts.failed_list(ts)?,
    };
    let mut response = counts.response(command, status, comment);
    if failed.is_some() {
        response = response.with_us(tag::COMMAND_DATA_SET_TYPE, DATA_SET_PRESENT);
    }
    message::send(
        association,
        request.context_id,
        &response,
        failed.as_deref(),
    )
    .await
}

/// The Unique Keys of the retrieval `operation` (its name, for an error) asks for, or why
/// it is refused: a caller that may not retrieve, an Identifier that cannot be read or is
/// not one of the model.
fn selection(
    request: &Request<'_>,
    data: DataSet,
    authorized: bool,
    operation: &str,
) -> Result<Selection, Refusal> {
    if !authorized {
        // PS3.7 9.1.3.1.6, 9.1.4.1.7: the peer was not authorized to invoke the operation
        return Err(refusal(
            Status::NOT_AUTHORIZED,
            format!(
                "{} is not an AE this server answers retrievals of",
                request.calling
            ),
            None,
        ));
    }
    let bytes = match data {
        DataSet::Received(bytes) => bytes,
        DataSet::None => {
            return Err(refusal(
                Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                format!("{operation}-RQ without an Identifier"),
                None,
            ));
        }
        DataSet::Dropped { size } => {
            return Err(refusal(
                Status::UNABLE_TO_PROCESS,
                format!("an Identifier of {size} bytes is over the limit"),
                None,
            ));
        }
    };
    let unreadable = |reason: String| {
        refusal(
            Status::UNABLE_TO_PROCESS,
            format!("the Identifier cannot be read: {reason}"),
            None,
        )
    };
    let ts = TransferSyntaxRegistry
        .get(request.transfer_syntax)
        .ok_or_else(|| unreadable(format!("transfer syntax {}", request.transfer_syntax)))?;
    let identifier = crate::scu::read_identifier(&bytes, ts).map_err(unreadable)?;
    Selection::parse(&identifier)
}

/// The instances a retrieval sends, as `index` shows them; an index that fails is Refused:
/// Out of resources, unable to calculate number of matches (PS3.4 Tables C.4-2, C.4-3).
async fn selected(
    selection: &Selection,
    index: &Index,
    operation: &str,
) -> Result<Vec<InstanceEntry>, Refusal> {
    selection.instances(index).await.map_err(|e| {
        tracing::error!(error = %e, operation, "the index failed");
        refusal(
            Status::UNABLE_TO_CALCULATE_MATCHES,
            "the index could not be read",
            None,
        )
    })
}

/// What a C-MOVE needs before its first sub-operation, or why it has none: the instances
/// to send and where.
async fn prepare<'c>(
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    index: &Index,
    config: &'c Config,
    authorized: bool,
    retrieved: &mut Retrieved,
) -> Result<(Vec<InstanceEntry>, &'c RemoteAe), Refusal> {
    let selection = selection(request, data, authorized, "C-MOVE")?;
    retrieved.destination = command
        .text(tag::MOVE_DESTINATION)
        .unwrap_or_default()
        .to_owned();
    // PS3.4 Table C.4-2: Refused: Move Destination unknown
    let Some(destination) = config.move_destination(&retrieved.destination) else {
        return Err(refusal(
            Status::MOVE_DESTINATION_UNKNOWN,
            format!(
                "{:?} is not a destination this server sends to",
                retrieved.destination
            ),
            None,
        ));
    };
    retrieved.destination_host = Some(destination.host.clone());
    let instances = selected(&selection, index, "C-MOVE").await?;
    Ok((instances, destination))
}

/// Answer a request that is refused: its Failure status, with what is wrong and where.
async fn refuse<A, S>(
    association: &mut A,
    request: &Request<'_>,
    command: &Command,
    refusal: &Refusal,
) -> Result<(), MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let mut response = command::response(command, refusal.status, Some(&refusal.comment));
    if let Some(tag) = refusal.offending {
        response = response.with_tags(tag::OFFENDING_ELEMENT, &[tag]);
    }
    message::send(association, request.context_id, &response, None).await
}

/// Answer a C-MOVE-RQ on `association`: the sub-operations on an association of their own,
/// every Pending response and the final one.
///
/// `asked` is what the AE title that was called gives: the archive that holds the files,
/// read as that title shows it. `authorized` is whether the caller may retrieve at all. An
/// error is the association's: it cannot go on.
pub(super) async fn c_move<A, S>(
    association: &mut A,
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    asked: &Destination,
    config: &Config,
    authorized: bool,
) -> Result<After<Retrieved>, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let message_id = command.message_id()?;
    let (archive, index) = (asked.archive(), asked.index());
    let mut retrieved = Retrieved {
        status: Status::SUCCESS,
        counts: Counts::default(),
        destination: String::new(),
        destination_host: None,
        sent: Vec::new(),
    };
    let prepared = prepare(
        request,
        command,
        data,
        &index,
        config,
        authorized,
        &mut retrieved,
    )
    .await;
    let (instances, destination) = match prepared {
        Ok(prepared) => prepared,
        Err(refusal) => {
            retrieved.status = refusal.status;
            refuse(association, request, command, &refusal).await?;
            return Ok(After::Open(retrieved));
        }
    };
    retrieved.counts = Counts::of(instances.len());
    if instances.is_empty() {
        // nothing under these keys that the caller may see: no sub-operation, no failure
        finish(
            association,
            request,
            command,
            Status::SUCCESS,
            &retrieved.counts,
            None,
        )
        .await?;
        return Ok(After::Open(retrieved));
    }
    let stored: Vec<(String, String)> = instances
        .iter()
        .map(|entry| {
            (
                entry.sop_class_uid.clone(),
                entry.transfer_syntax_uid.clone(),
            )
        })
        .collect();
    let originator = Originator {
        ae_title: request.calling.to_owned(),
        message_id,
    };
    // the AE that was asked is the one that sends: a project under its own title
    let calling = AeTitle::new(request.called).unwrap_or_else(|_| config.ae_title.clone());
    let opened = Sender::open(
        destination,
        &calling,
        &stored,
        config.timeout,
        Some(originator),
    )
    .await;
    let mut sender = match opened {
        Ok(sender) => sender,
        Err(e) => {
            // PS3.4 Table C.4-2: Refused: Out of resources, unable to perform sub-operations
            tracing::warn!(error = %e, destination = %retrieved.destination, "C-MOVE: no association with the destination");
            for entry in &instances {
                retrieved.counts.count(&entry.sop_instance_uid, None);
            }
            retrieved.status = Status::UNABLE_TO_PERFORM_SUB_OPERATIONS;
            finish(
                association,
                request,
                command,
                retrieved.status,
                &retrieved.counts,
                Some(&e.to_string()),
            )
            .await?;
            return Ok(After::Open(retrieved));
        }
    };
    let mut entries = instances.iter();
    let mut cancelled = false;
    // whether the association with the destination still carries instances
    let mut holds = true;
    while retrieved.counts.remaining > 0 {
        match interruption(association, message_id, "C-MOVE").await? {
            Interruption::Nothing => {}
            Interruption::Cancel => {
                cancelled = true;
                break;
            }
            ended @ (Interruption::Released | Interruption::Aborted) => {
                // PS3.4 C.4.2.3.1, note: with the requester gone, what is left is not sent
                sender.abort().await;
                retrieved.status = Status::CANCEL;
                return Ok(match ended {
                    Interruption::Released => After::Released(retrieved),
                    _ => After::Aborted(retrieved),
                });
            }
        }
        let Some(entry) = entries.next() else { break };
        let outcome = if holds {
            match archive.file(entry).await {
                Ok(file) => sender
                    .send(
                        file,
                        &entry.sop_class_uid,
                        &entry.sop_instance_uid,
                        &entry.transfer_syntax_uid,
                    )
                    .await
                    .map_err(|e| {
                        holds = e.association_holds();
                        e.to_string()
                    }),
                Err(e) => Err(format!("its file cannot be read: {e}")),
            }
        } else {
            Err("the association with the destination is over".into())
        };
        match outcome {
            Ok(status) => {
                retrieved
                    .counts
                    .count(&entry.sop_instance_uid, Some(status));
                retrieved.sent.push(Sent {
                    study_instance_uid: entry.study_instance_uid.clone(),
                    sop_class_uid: entry.sop_class_uid.clone(),
                });
            }
            Err(reason) => {
                tracing::warn!(instance = %entry.sop_instance_uid, destination = %retrieved.destination, reason, "C-MOVE: a sub-operation failed");
                retrieved.counts.count(&entry.sop_instance_uid, None);
            }
        }
        if retrieved.counts.remaining > 0 {
            let pending = retrieved.counts.response(command, Status::PENDING, None);
            message::send(association, request.context_id, &pending, None).await?;
        }
    }
    if holds {
        sender.release().await;
    } else {
        sender.abort().await;
    }
    retrieved.status = if cancelled {
        Status::CANCEL
    } else {
        retrieved.counts.final_status()
    };
    finish(
        association,
        request,
        command,
        retrieved.status,
        &retrieved.counts,
        None,
    )
    .await?;
    Ok(After::Open(retrieved))
}

/// What the requester of a C-GET did with one instance: stored it, with this status, or
/// not; or it ended the association instead of answering.
enum SubOperation {
    Done(Result<Status, String>),
    Released,
    Aborted,
}

/// Send one instance to the requester of a C-GET on its own association, and wait for its
/// C-STORE-RSP. A C-CANCEL-GET-RQ that comes meanwhile sets `cancelled`: the instance on
/// its way is finished all the same.
async fn sub_operation<A, S>(
    association: &mut A,
    accepted: &[Accepted],
    file: bytes::Bytes,
    entry: &InstanceEntry,
    message_id: u16,
    get_id: u16,
    cancelled: &mut bool,
) -> Result<SubOperation, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let contexts = accepted.to_vec();
    let (sop_class, stored_in) = (
        entry.sop_class_uid.clone(),
        entry.transfer_syntax_uid.clone(),
    );
    // decoding pixel data is work for another thread than the one that serves
    let prepared =
        tokio::task::spawn_blocking(move || encoded(&contexts, &file, &sop_class, &stored_in))
            .await
            .map_err(|e| MessageError::Protocol(e.to_string()))?;
    let (context, data_set) = match prepared {
        Ok(prepared) => prepared,
        // PS3.4 C.4.3.3.1: no presentation context for it is a failed sub-operation
        Err(e) => return Ok(SubOperation::Done(Err(e.to_string()))),
    };
    let request = c_store_rq(
        message_id,
        &entry.sop_class_uid,
        &entry.sop_instance_uid,
        None,
    );
    message::send(association, context, &request, Some(&data_set)).await?;
    loop {
        let message = match message::receive(association, 0).await? {
            Incoming::Message(message) => message,
            Incoming::Released => return Ok(SubOperation::Released),
            Incoming::Aborted => return Ok(SubOperation::Aborted),
        };
        if message.command.command_field()? == field::C_CANCEL_RQ {
            // a cancel of another operation has nothing to cancel
            *cancelled |= message.command.us(tag::MESSAGE_ID_BEING_RESPONDED_TO)? == get_id;
            continue;
        }
        return match stored(&message.command, message_id) {
            Ok(status) => Ok(SubOperation::Done(Ok(status))),
            Err(e @ SendError::Refused { .. }) => Ok(SubOperation::Done(Err(e.to_string()))),
            Err(e) => Err(MessageError::Protocol(format!(
                "{e} while a C-GET is being answered"
            ))),
        };
    }
}

/// Answer a C-GET-RQ on `association`: the sub-operations on this same association, every
/// Pending response and the final one (PS3.4 C.4.3.3.1).
///
/// `asked` and `authorized` are as for [`c_move`]. An instance goes on a presentation
/// context of its SOP class for which the requester took the SCP role; with none, it is a
/// failed sub-operation.
pub(super) async fn c_get<A, S>(
    association: &mut A,
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    asked: &Destination,
    authorized: bool,
) -> Result<After<Retrieved>, MessageError>
where
    A: AsyncAssociation<S> + Association + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let get_id = command.message_id()?;
    let (archive, index) = (asked.archive(), asked.index());
    let mut retrieved = Retrieved {
        status: Status::SUCCESS,
        counts: Counts::default(),
        // what is retrieved goes to who asks
        destination: request.calling.to_owned(),
        destination_host: None,
        sent: Vec::new(),
    };
    let instances = match selection(request, data, authorized, "C-GET") {
        Ok(selection) => selected(&selection, &index, "C-GET").await,
        Err(refusal) => Err(refusal),
    };
    let instances = match instances {
        Ok(instances) => instances,
        Err(refusal) => {
            retrieved.status = refusal.status;
            refuse(association, request, command, &refusal).await?;
            return Ok(After::Open(retrieved));
        }
    };
    retrieved.counts = Counts::of(instances.len());
    // PS3.7 D.3.3.4: the contexts on which the requester is the Storage SCP
    let accepted: Vec<Accepted> = association
        .presentation_contexts()
        .iter()
        .filter(|pc| pc.reason == PresentationContextResultReason::Acceptance)
        .map(|pc| Accepted {
            id: pc.id,
            abstract_syntax: pc.abstract_syntax.trim_end_matches('\0').to_owned(),
            transfer_syntax: pc.transfer_syntax.trim_end_matches('\0').to_owned(),
        })
        .filter(|context| {
            super::is_storage(&context.abstract_syntax)
                && association
                    .requestor_roles_for(&context.abstract_syntax)
                    .scp
        })
        .collect();
    let mut cancelled = false;
    // the Message IDs of the sub-operations are not the C-GET's
    let mut message_id = get_id;
    for entry in &instances {
        if !cancelled {
            match interruption(association, get_id, "C-GET").await? {
                Interruption::Nothing => {}
                Interruption::Cancel => cancelled = true,
                Interruption::Released => {
                    retrieved.status = Status::CANCEL;
                    return Ok(After::Released(retrieved));
                }
                Interruption::Aborted => {
                    retrieved.status = Status::CANCEL;
                    return Ok(After::Aborted(retrieved));
                }
            }
        }
        if cancelled {
            break;
        }
        message_id = message_id.checked_add(1).unwrap_or(1);
        if message_id == get_id {
            message_id = message_id.checked_add(1).unwrap_or(1);
        }
        let outcome = match archive.file(entry).await {
            Ok(file) => {
                let done = sub_operation(
                    association,
                    &accepted,
                    file,
                    entry,
                    message_id,
                    get_id,
                    &mut cancelled,
                )
                .await?;
                match done {
                    SubOperation::Done(outcome) => outcome,
                    SubOperation::Released => {
                        retrieved.status = Status::CANCEL;
                        return Ok(After::Released(retrieved));
                    }
                    SubOperation::Aborted => {
                        retrieved.status = Status::CANCEL;
                        return Ok(After::Aborted(retrieved));
                    }
                }
            }
            Err(e) => Err(format!("its file cannot be read: {e}")),
        };
        match outcome {
            Ok(status) => {
                retrieved
                    .counts
                    .count(&entry.sop_instance_uid, Some(status));
                retrieved.sent.push(Sent {
                    study_instance_uid: entry.study_instance_uid.clone(),
                    sop_class_uid: entry.sop_class_uid.clone(),
                });
            }
            Err(reason) => {
                tracing::warn!(instance = %entry.sop_instance_uid, reason, "C-GET: a sub-operation failed");
                retrieved.counts.count(&entry.sop_instance_uid, None);
            }
        }
        if retrieved.counts.remaining > 0 {
            let pending = retrieved.counts.response(command, Status::PENDING, None);
            message::send(association, request.context_id, &pending, None).await?;
        }
    }
    retrieved.status = if cancelled && retrieved.counts.remaining > 0 {
        Status::CANCEL
    } else {
        retrieved.counts.final_status()
    };
    finish(
        association,
        request,
        command,
        retrieved.status,
        &retrieved.counts,
        None,
    )
    .await?;
    Ok(After::Open(retrieved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identifier(level: &str, keys: &[(Tag, &str)]) -> InMemDicomObject {
        let mut identifier = InMemDicomObject::new_empty();
        identifier.put(DataElement::new(
            tags::QUERY_RETRIEVE_LEVEL,
            VR::CS,
            PrimitiveValue::from(level),
        ));
        for (tag, value) in keys {
            identifier.put(DataElement::new(*tag, VR::UI, PrimitiveValue::from(*value)));
        }
        identifier
    }

    #[test]
    fn the_unique_keys_of_a_retrieval() {
        let (study, series, instance) = (
            tags::STUDY_INSTANCE_UID,
            tags::SERIES_INSTANCE_UID,
            tags::SOP_INSTANCE_UID,
        );
        let studies = Selection::parse(&identifier("STUDY", &[(study, "1.2\\1.3\\1.2")])).unwrap();
        assert_eq!(studies.studies, ["1.2", "1.3"]);
        assert!(studies.series.is_empty() && studies.instances.is_empty());
        let series_of = Selection::parse(&identifier(
            "SERIES",
            &[(study, "1.2"), (series, "1.2.1\\1.2.2")],
        ))
        .unwrap();
        assert_eq!(series_of.series, ["1.2.1", "1.2.2"]);
        let images = Selection::parse(&identifier(
            "IMAGE",
            &[
                (study, "1.2"),
                (series, "1.2.1"),
                (instance, "1.2.1.1\\1.2.1.2"),
            ],
        ))
        .unwrap();
        assert_eq!(images.instances, ["1.2.1.1", "1.2.1.2"]);

        for (bad, offending) in [
            (
                identifier("PATIENT", &[(study, "1.2")]),
                tags::QUERY_RETRIEVE_LEVEL,
            ),
            (
                identifier("", &[(study, "1.2")]),
                tags::QUERY_RETRIEVE_LEVEL,
            ),
            (identifier("STUDY", &[]), study),
            (identifier("STUDY", &[(study, "")]), study),
            (identifier("STUDY", &[(study, "1.2.*")]), study),
            (identifier("SERIES", &[(study, "1.2")]), series),
            // one value above the level retrieved
            (
                identifier("SERIES", &[(study, "1.2\\1.3"), (series, "1.2.1")]),
                study,
            ),
            (
                identifier(
                    "IMAGE",
                    &[
                        (study, "1.2"),
                        (series, "1.2.1\\1.2.2"),
                        (instance, "1.2.1.1"),
                    ],
                ),
                series,
            ),
            (
                identifier("IMAGE", &[(study, "1.2"), (series, "1.2.1")]),
                instance,
            ),
        ] {
            let refused = Selection::parse(&bad).unwrap_err();
            assert_eq!(refused.status, Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS);
            assert_eq!(refused.offending, Some(offending), "{}", refused.comment);
        }
    }

    #[test]
    fn the_final_status_follows_the_counts() {
        let run = |outcomes: &[Option<u16>]| {
            let mut counts = Counts::of(outcomes.len());
            for (n, outcome) in outcomes.iter().enumerate() {
                counts.count(&format!("1.2.{n}"), outcome.map(Status));
            }
            assert_eq!(counts.remaining, 0);
            counts
        };
        let (stored, warned, failed) = (Some(0x0000), Some(0xB000), None);
        assert_eq!(run(&[stored, stored]).final_status(), Status::SUCCESS);
        // some stored, some not, or some with a warning
        assert_eq!(
            run(&[stored, failed]).final_status(),
            Status::SUB_OPERATIONS_WARNING
        );
        assert_eq!(
            run(&[stored, warned]).final_status(),
            Status::SUB_OPERATIONS_WARNING
        );
        // all with a warning
        assert_eq!(
            run(&[warned, warned]).final_status(),
            Status::SUB_OPERATIONS_WARNING
        );
        // none stored
        let none = run(&[failed, failed]);
        assert_eq!(
            none.final_status(),
            Status::UNABLE_TO_PERFORM_SUB_OPERATIONS
        );
        assert_eq!(none.failed_instances, ["1.2.0", "1.2.1"]);
        assert_eq!((none.completed, none.failed, none.warning), (0, 2, 0));
        // nothing to send is nothing failed
        assert_eq!(run(&[]).final_status(), Status::SUCCESS);
    }

    #[test]
    fn the_counts_of_a_response_are_those_its_status_has() {
        let request = Command::new()
            .with_us(tag::COMMAND_FIELD, crate::command::field::C_MOVE_RQ)
            .with_us(tag::MESSAGE_ID, 3);
        let counts = Counts {
            remaining: 70_000,
            completed: 2,
            failed: 1,
            warning: 0,
            failed_instances: vec!["1.2.3".into()],
        };
        let remaining = |status| {
            counts
                .response(&request, status, None)
                .us(tag::NUMBER_OF_REMAINING_SUB_OPERATIONS)
                .ok()
        };
        // more than a US holds is written as the most it holds
        assert_eq!(remaining(Status::PENDING), Some(u16::MAX));
        assert_eq!(remaining(Status::CANCEL), Some(u16::MAX));
        for last in [
            Status::SUCCESS,
            Status::SUB_OPERATIONS_WARNING,
            Status::UNABLE_TO_PERFORM_SUB_OPERATIONS,
        ] {
            assert_eq!(remaining(last), None);
            let response = counts.response(&request, last, None);
            assert_eq!(response.us(tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS), Ok(2));
            assert_eq!(response.us(tag::NUMBER_OF_FAILED_SUB_OPERATIONS), Ok(1));
            assert_eq!(response.us(tag::NUMBER_OF_WARNING_SUB_OPERATIONS), Ok(0));
        }
        assert!(
            Counts::of(1)
                .failed_list(TransferSyntaxRegistry.get(crate::IMPLICIT_VR_LE).unwrap())
                .unwrap()
                .is_none()
        );
    }
}
