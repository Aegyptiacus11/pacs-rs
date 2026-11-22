//! C-GET SCU at study level, Study Root or Patient Root (PS3.4 C.4.3.2, C.6; PS3.7 9.1.3,
//! 9.3.3): the remote AE is asked for studies and sends them back on the same association,
//! by C-STORE sub-operations to which pacs-rs is the SCP (PS3.4 C.5.3). Nothing has to know
//! pacs-rs's address, so it works with an archive that refuses to C-MOVE to an AE it does
//! not know, and with pacs-rs not listening for DIMSE.
//!
//! Each storage SOP class pacs-rs is to receive needs a presentation context of its own,
//! proposed with SCP/SCU Role Selection (PS3.7 D.3.3.4), and an association has room for
//! 128 contexts in all (PS3.8 9.3.2.2: odd context IDs from 1 to 255). Two go to the GET
//! models; the others go to the storage SOP classes a hospital archive holds most
//! ([`IMAGES`], [`OBJECTS`]), then the rest of PS3.4 Table B.5-1 in its order until none is
//! left. An instance of a class without a context, or stored in a transfer syntax its
//! context was not proposed with and that the archive does not convert, is not sent, and
//! the remote AE counts it as a failed sub-operation (PS3.4 C.4.3.3.1).

use std::ops::ControlFlow;
use std::time::Duration;

use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;

use super::associate_with_roles;
use super::find::c_cancel_rq;
use super::retrieve::{
    MoveError, MoveOutcome, Progress, check_patient_id, failed_list, retrieve_identifier,
};
use crate::ae::{AeTitle, RemoteAe};
use crate::command::{self, Command, PRIORITY_MEDIUM, field, tag};
use crate::message::{DataSet, Incoming, MessageError};
use crate::query::{PATIENT_ROOT_GET, Root, STUDY_ROOT_GET};
use crate::scp::store::{Destination, store};
use crate::scp::{Observer, Peer};
use crate::status::{Status, StatusClass};
use crate::{EXPLICIT_VR_LE, IMPLICIT_VR_LE};
use pacs_archive::observed::stored_during;

/// Command Data Set Type of a request with an Identifier (PS3.7 Table 9.3-7, note).
const IDENTIFIER: u16 = 0x0102;

/// Presentation contexts an association can carry (PS3.8 9.3.2.2).
const MOST_CONTEXTS: usize = 128;

/// The image storage SOP classes proposed first, each with the compressed transfer
/// syntaxes of [`IMAGES_AS`]: the images a hospital archive holds most (PS3.4 Table B.5-1).
pub const IMAGES: &[&str] = &[
    "1.2.840.10008.5.1.4.1.1.2",      // CT Image
    "1.2.840.10008.5.1.4.1.1.2.1",    // Enhanced CT Image
    "1.2.840.10008.5.1.4.1.1.4",      // MR Image
    "1.2.840.10008.5.1.4.1.1.4.1",    // Enhanced MR Image
    "1.2.840.10008.5.1.4.1.1.128",    // Positron Emission Tomography Image
    "1.2.840.10008.5.1.4.1.1.130",    // Enhanced PET Image
    "1.2.840.10008.5.1.4.1.1.20",     // Nuclear Medicine Image
    "1.2.840.10008.5.1.4.1.1.1",      // Computed Radiography Image
    "1.2.840.10008.5.1.4.1.1.1.1",    // Digital X-Ray Image - For Presentation
    "1.2.840.10008.5.1.4.1.1.1.1.1",  // Digital X-Ray Image - For Processing
    "1.2.840.10008.5.1.4.1.1.1.2",    // Digital Mammography X-Ray Image - For Presentation
    "1.2.840.10008.5.1.4.1.1.1.2.1",  // Digital Mammography X-Ray Image - For Processing
    "1.2.840.10008.5.1.4.1.1.13.1.3", // Breast Tomosynthesis Image
    "1.2.840.10008.5.1.4.1.1.6.1",    // Ultrasound Image
    "1.2.840.10008.5.1.4.1.1.3.1",    // Ultrasound Multi-frame Image
    "1.2.840.10008.5.1.4.1.1.12.1",   // X-Ray Angiographic Image
    "1.2.840.10008.5.1.4.1.1.12.1.1", // Enhanced XA Image
    "1.2.840.10008.5.1.4.1.1.12.2",   // X-Ray Radiofluoroscopic Image
    "1.2.840.10008.5.1.4.1.1.7",      // Secondary Capture Image
    "1.2.840.10008.5.1.4.1.1.7.1",    // Multi-frame Single Bit Secondary Capture Image
    "1.2.840.10008.5.1.4.1.1.7.2",    // Multi-frame Grayscale Byte Secondary Capture Image
    "1.2.840.10008.5.1.4.1.1.7.3",    // Multi-frame Grayscale Word Secondary Capture Image
    "1.2.840.10008.5.1.4.1.1.7.4",    // Multi-frame True Color Secondary Capture Image
    "1.2.840.10008.5.1.4.1.1.481.1",  // RT Image
    "1.2.840.10008.5.1.4.1.1.77.1.4", // VL Photographic Image
    "1.2.840.10008.5.1.4.1.1.77.1.1", // VL Endoscopic Image
    "1.2.840.10008.5.1.4.1.1.77.1.6", // VL Whole Slide Microscopy Image
];

/// The other storage SOP classes proposed before the rest of the table, in the
/// uncompressed transfer syntaxes: the objects that come with images.
pub const OBJECTS: &[&str] = &[
    "1.2.840.10008.5.1.4.1.1.66.4",  // Segmentation
    "1.2.840.10008.5.1.4.1.1.66.5",  // Surface Segmentation
    "1.2.840.10008.5.1.4.1.1.66.1",  // Spatial Registration
    "1.2.840.10008.5.1.4.1.1.66.3",  // Deformable Spatial Registration
    "1.2.840.10008.5.1.4.1.1.30",    // Parametric Map
    "1.2.840.10008.5.1.4.1.1.481.2", // RT Dose
    "1.2.840.10008.5.1.4.1.1.481.3", // RT Structure Set
    "1.2.840.10008.5.1.4.1.1.481.5", // RT Plan
    "1.2.840.10008.5.1.4.1.1.481.8", // RT Ion Plan
    "1.2.840.10008.5.1.4.1.1.88.11", // Basic Text SR
    "1.2.840.10008.5.1.4.1.1.88.22", // Enhanced SR
    "1.2.840.10008.5.1.4.1.1.88.33", // Comprehensive SR
    "1.2.840.10008.5.1.4.1.1.88.34", // Comprehensive 3D SR
    "1.2.840.10008.5.1.4.1.1.88.59", // Key Object Selection Document
    "1.2.840.10008.5.1.4.1.1.88.67", // X-Ray Radiation Dose SR
    "1.2.840.10008.5.1.4.1.1.11.1",  // Grayscale Softcopy Presentation State
    "1.2.840.10008.5.1.4.1.1.11.2",  // Color Softcopy Presentation State
    "1.2.840.10008.5.1.4.1.1.104.1", // Encapsulated PDF
    "1.2.840.10008.5.1.4.1.1.104.3", // Encapsulated STL
    "1.2.840.10008.5.1.4.1.1.66",    // Raw Data
];

/// The transfer syntaxes the classes of [`IMAGES`] are proposed in, those dicom-rs knows:
/// uncompressed, then the compressions an archive keeps images in. An instance is kept in
/// the one it comes in, as the C-STORE SCP keeps them. The other classes are proposed in
/// the two uncompressed ones only, which keeps the A-ASSOCIATE-RQ of 128 contexts under
/// the 32 KB a peer reads it in.
const IMAGES_AS: &[&str] = &[
    EXPLICIT_VR_LE,
    IMPLICIT_VR_LE,
    "1.2.840.10008.1.2.4.80", // JPEG-LS Lossless
    "1.2.840.10008.1.2.4.70", // JPEG Lossless, First-Order Prediction
    "1.2.840.10008.1.2.4.57", // JPEG Lossless
    "1.2.840.10008.1.2.4.90", // JPEG 2000 Lossless Only
    "1.2.840.10008.1.2.5",    // RLE Lossless
    "1.2.840.10008.1.2.4.81", // JPEG-LS Near-Lossless
    "1.2.840.10008.1.2.4.91", // JPEG 2000
    "1.2.840.10008.1.2.4.50", // JPEG Baseline
];

/// The storage SOP classes proposed, in order: [`IMAGES`], [`OBJECTS`], then the rest of
/// Table B.5-1, as many as fit beside the two GET contexts.
pub fn proposed_storage() -> Vec<&'static str> {
    let mut proposed: Vec<&str> = IMAGES
        .iter()
        .chain(OBJECTS)
        .copied()
        .filter(|uid| crate::scp::is_storage(uid))
        .collect();
    for (uid, _) in crate::sop_classes::STORAGE {
        if proposed.len() + 2 >= MOST_CONTEXTS {
            break;
        }
        if !proposed.contains(uid) {
            proposed.push(uid);
        }
    }
    proposed
}

/// Where the instances a C-GET brings go, and who is told of them.
#[derive(Clone, Copy)]
pub struct Receiving<'a> {
    /// Stored there as the C-STORE SCP stores what it is sent: the same checks, statuses
    /// and ingest.
    pub destination: &'a Destination,
    /// A larger data set is read through and refused, as the SCP refuses it.
    pub max_data_set_bytes: usize,
    /// Told what was stored once the association is over, as of an association the SCP
    /// served, the remote AE being the one that sent.
    pub observer: Option<&'a dyn Observer>,
}

/// Ask `remote`, as `calling`, for the studies `study_uids`, and store what it sends as
/// `receiving` says. Each Pending response's counts go to `on_progress`;
/// `ControlFlow::Break` sends C-CANCEL-GET-RQ. `patient_id`, the timeouts and the choice of
/// model are as for [`super::retrieve::move_studies`].
#[allow(clippy::too_many_arguments)]
pub async fn get_studies<F>(
    remote: &RemoteAe,
    calling: &AeTitle,
    patient_id: Option<&str>,
    study_uids: &[String],
    receiving: Receiving<'_>,
    timeout: Duration,
    response_timeout: Duration,
    on_progress: F,
) -> Result<MoveOutcome, MoveError>
where
    F: FnMut(Progress) -> ControlFlow<()>,
{
    let mut address = None;
    let (result, stored) = stored_during(connect_and_get(
        remote,
        calling,
        patient_id,
        study_uids,
        receiving,
        timeout,
        response_timeout,
        on_progress,
        &mut address,
    ))
    .await;
    if let Some(observer) = receiving.observer
        && !stored.is_empty()
    {
        let peer = Peer {
            address: address.unwrap_or_else(|| ([0, 0, 0, 0], remote.port).into()),
            calling: remote.ae_title.to_string(),
            called: calling.to_string(),
        };
        observer.received(&peer, stored);
    }
    result
}

/// [`get_studies`], the remote AE's address written to `address` once connected.
#[allow(clippy::too_many_arguments)]
async fn connect_and_get<F>(
    remote: &RemoteAe,
    calling: &AeTitle,
    patient_id: Option<&str>,
    study_uids: &[String],
    receiving: Receiving<'_>,
    timeout: Duration,
    response_timeout: Duration,
    mut on_progress: F,
    address: &mut Option<std::net::SocketAddr>,
) -> Result<MoveOutcome, MoveError>
where
    F: FnMut(Progress) -> ControlFlow<()>,
{
    if study_uids.is_empty() {
        return Err(MoveError::Request("no Study Instance UID to get".into()));
    }
    if let Some(bad) = study_uids.iter().find(|u| !pacs_core::is_valid_uid(u)) {
        return Err(MoveError::Request(format!("{bad:?} is not a UID")));
    }
    let patient_id = patient_id.map(str::trim).filter(|id| !id.is_empty());
    if let Some(id) = patient_id {
        check_patient_id(id).map_err(MoveError::Request)?;
    }
    let images_as: Vec<String> = IMAGES_AS
        .iter()
        .filter(|ts| TransferSyntaxRegistry.get(ts).is_some())
        .map(|ts| (*ts).to_owned())
        .collect();
    let plain = vec![EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()];
    let in_syntaxes = |uid: &str| {
        if IMAGES.contains(&uid) {
            images_as.clone()
        } else {
            plain.clone()
        }
    };
    let storage: Vec<String> = proposed_storage().into_iter().map(str::to_owned).collect();
    let mut contexts = vec![
        (STUDY_ROOT_GET.to_owned(), plain.clone()),
        (PATIENT_ROOT_GET.to_owned(), plain.clone()),
    ];
    contexts.extend(storage.iter().map(|uid| (uid.clone(), in_syntaxes(uid))));
    let (mut association, accepted) = associate_with_roles(
        remote,
        calling,
        &contexts,
        &storage,
        timeout,
        response_timeout,
    )
    .await?;
    *address = association.peer_address();
    let chosen = [
        (STUDY_ROOT_GET, Root::Study),
        (PATIENT_ROOT_GET, Root::Patient),
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
        return Err(super::ConnectError::NotAccepted(STUDY_ROOT_GET).into());
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
    let mut identifier = Vec::new();
    if let Err(e) =
        retrieve_identifier(named, study_uids).write_dataset_with_ts(&mut identifier, ts)
    {
        association.abort().await;
        return Err(MoveError::Identifier(e.to_string()));
    }
    // the sending AE is the remote one, the receiving one pacs-rs, as on any C-STORE
    let (sender, receiver) = (remote.ae_title.to_string(), calling.to_string());
    let message_id = 1;
    let request = c_get_rq(sop_class, message_id);
    let get_context = context.id;
    let mut last = Progress::default();
    let mut cancelled = false;
    let result = async {
        association
            .send(get_context, &request, Some(&identifier))
            .await?;
        loop {
            let message = match association.receive(receiving.max_data_set_bytes).await? {
                Incoming::Message(message) => message,
                Incoming::Released | Incoming::Aborted => return Err(MoveError::Ended),
            };
            let command = &message.command;
            let field = command.command_field().ok();
            if field == Some(field::C_STORE_RQ) {
                let response = match accepted.iter().find(|c| c.id == message.context_id) {
                    Some(stored) if stored.id != get_context => {
                        let on = crate::scp::Request {
                            context_id: stored.id,
                            abstract_syntax: &stored.abstract_syntax,
                            transfer_syntax: &stored.transfer_syntax,
                            calling: &sender,
                            called: &receiver,
                        };
                        let (status, comment) =
                            store(&on, command, message.data, receiving.destination).await;
                        command::response(command, status, comment.as_deref())
                    }
                    _ => command::response(
                        command,
                        Status::SOP_CLASS_NOT_SUPPORTED,
                        Some("not a storage context of this association"),
                    ),
                };
                association
                    .send(message.context_id, &response, None)
                    .await?;
                continue;
            }
            if field != Some(field::C_GET_RSP)
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
                        .send(get_context, &c_cancel_rq(message_id), None)
                        .await?;
                }
                continue;
            }
            // PS3.4 C.4.3.1.3.2: Warning, Cancel and Failure may carry the Failed SOP
            // Instance UID List
            let failed_instances = match message.data {
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
                    destination: receiver.clone(),
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

/// A C-GET-RQ of the GET SOP class `sop_class`, PS3.7 Table 9.3-7.
pub fn c_get_rq(sop_class: &str, message_id: u16) -> Command {
    Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, sop_class)
        .with_us(tag::COMMAND_FIELD, field::C_GET_RQ)
        .with_us(tag::MESSAGE_ID, message_id)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, IDENTIFIER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_proposed_first_is_a_storage_class() {
        for uid in IMAGES.iter().chain(OBJECTS) {
            assert!(crate::scp::is_storage(uid), "{uid} is not in Table B.5-1");
        }
    }

    #[test]
    fn the_contexts_fit_one_association() {
        let proposed = proposed_storage();
        assert_eq!(proposed.len() + 2, MOST_CONTEXTS);
        let first: Vec<&str> = IMAGES.iter().chain(OBJECTS).copied().collect();
        assert_eq!(&proposed[..first.len()], first);
        let mut unique = proposed.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), proposed.len());
    }
}
