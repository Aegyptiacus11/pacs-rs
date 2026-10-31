//! The Storage SCP (PS3.4 Annex B): a C-STORE's data set made a Part 10 file, checked
//! against its request, and handed to its [`Destination`] with the statuses of PS3.4 Table
//! B.2-1.

use std::sync::Arc;

use bytes::Bytes;
use pacs_archive::{Archive, IngestError};

use super::Request;
use crate::command::{Command, tag};
use crate::message::DataSet;
use crate::project::{Project, ProjectError};
use crate::status::Status;
use crate::{IMPLEMENTATION_CLASS_UID, IMPLEMENTATION_VERSION_NAME};

/// Where the instances an association receives go.
#[derive(Debug, Clone)]
pub enum Destination {
    /// Into the archive, exactly as a REST or STOW-RS upload: compressed, stored, indexed;
    /// an instance sent again is kept as first stored unless overwriting is on.
    Archive(Archive),
    /// De-identified by a project's profile first ([`Project`]); only the result is
    /// archived.
    Project(Arc<Project>),
}

/// Why a destination did not take an instance.
enum Refusal {
    Ingest(IngestError),
    Project(ProjectError),
}

impl Destination {
    async fn store(&self, file: Bytes) -> Result<(), Refusal> {
        match self {
            Destination::Archive(archive) => archive
                .ingest(file)
                .await
                .map(|_| ())
                .map_err(Refusal::Ingest),
            Destination::Project(project) => match project.receive(file).await {
                Ok(_) => Ok(()),
                Err(ProjectError::Ingest(e)) => Err(Refusal::Ingest(e)),
                Err(e) => Err(Refusal::Project(e)),
            },
        }
    }
}

/// A C-STORE: the data set and the request's UIDs checked against each other, then the
/// instance handed to `destination`, with the statuses of PS3.4 Table B.2-1.
pub(super) async fn store(
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    destination: &Destination,
) -> (Status, Option<String>) {
    let data = match data {
        DataSet::Received(data) => data,
        DataSet::None => {
            return (
                Status::CANNOT_UNDERSTAND,
                Some("C-STORE-RQ without a data set".into()),
            );
        }
        DataSet::Dropped { size } => {
            return (
                Status::OUT_OF_RESOURCES_TOO_LARGE,
                Some(format!("data set of {size} bytes is over the limit")),
            );
        }
    };
    let sop_class = request.abstract_syntax;
    let sop_instance = command.ui(tag::AFFECTED_SOP_INSTANCE_UID).unwrap_or("");
    let file = part10(
        &data,
        sop_class,
        sop_instance,
        request.transfer_syntax,
        request.calling,
        request.called,
    );
    let (found_class, found_instance) = match identifiers(&file) {
        Ok(found) => found,
        Err(e) => return (Status::CANNOT_UNDERSTAND, Some(e)),
    };
    // PS3.4 B.2.3: the data set must be an instance of the SOP class it is sent as
    if found_class != sop_class {
        return (
            Status::DATA_SET_DOES_NOT_MATCH_SOP_CLASS,
            Some(format!("SOP Class UID is {found_class}, not {sop_class}")),
        );
    }
    if found_instance != sop_instance {
        return (
            Status::CANNOT_UNDERSTAND_INSTANCE_UID,
            Some(format!(
                "SOP Instance UID is {found_instance}, not {sop_instance}"
            )),
        );
    }
    match destination.store(Bytes::from(file)).await {
        // PS3.4 B.2.2: Success means the instance is stored. One sent again is: the copy
        // kept is the first (or the new one, with overwriting on), as for a REST upload. An
        // instance a project quarantines is stored too, in its quarantine.
        Ok(()) => (Status::SUCCESS, None),
        Err(Refusal::Ingest(IngestError::Invalid(e))) => {
            (Status::CANNOT_UNDERSTAND, Some(e.to_string()))
        }
        Err(Refusal::Ingest(e @ (IngestError::Conflict(_) | IngestError::WrongStudy { .. }))) => {
            (Status::CANNOT_UNDERSTAND_CONFLICT, Some(e.to_string()))
        }
        Err(Refusal::Ingest(IngestError::Store(e))) => {
            tracing::error!(error = %e, sop_instance, "C-STORE: the store refused the file");
            (
                Status::OUT_OF_RESOURCES_STORE,
                Some("the store refused the file".into()),
            )
        }
        Err(Refusal::Ingest(IngestError::Index(e))) => {
            tracing::error!(error = %e, sop_instance, "C-STORE: the index refused the instance");
            (
                Status::OUT_OF_RESOURCES_INDEX,
                Some("the index refused the instance".into()),
            )
        }
        Err(Refusal::Project(e @ ProjectError::Refused(_))) => {
            (Status::CANNOT_UNDERSTAND_QUARANTINED, Some(e.to_string()))
        }
        Err(Refusal::Project(e @ ProjectError::Quarantine(_))) => {
            tracing::error!(error = %e, sop_instance, "C-STORE: the quarantine refused the file");
            (
                Status::OUT_OF_RESOURCES_STORE,
                Some("the quarantine store refused the file".into()),
            )
        }
        Err(Refusal::Project(e)) => (
            Status::CANNOT_UNDERSTAND_DEIDENTIFICATION,
            Some(e.to_string()),
        ),
    }
}

/// SOP Class UID and SOP Instance UID of a Part 10 file, read up to the pixel data.
fn identifiers(file: &[u8]) -> Result<(String, String), String> {
    use dicom_dictionary_std::tags;
    let object = dicom_object::OpenFileOptions::new()
        .read_preamble(dicom_object::file::ReadPreamble::Always)
        .read_until(tags::PIXEL_DATA)
        .from_reader(file)
        .map_err(|e| format!("cannot read the data set: {e}"))?;
    let read = |tag, name| {
        object
            .element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim_end_matches(['\0', ' ']).to_owned())
            .ok_or_else(|| format!("the data set has no {name}"))
    };
    Ok((
        read(tags::SOP_CLASS_UID, "SOP Class UID")?,
        read(tags::SOP_INSTANCE_UID, "SOP Instance UID")?,
    ))
}

/// A DICOM file of a received data set: 128-byte preamble, "DICM", the File Meta
/// Information group in Explicit VR Little Endian (PS3.10 7.1, Table 7.1-1), then the
/// data set exactly as received, in the negotiated transfer syntax.
pub fn part10(
    data_set: &[u8],
    sop_class: &str,
    sop_instance: &str,
    transfer_syntax: &str,
    calling: &str,
    called: &str,
) -> Vec<u8> {
    let mut meta = Vec::new();
    // File Meta Information Version: OB, 00H 01H
    meta.extend_from_slice(&[
        0x02, 0x00, 0x01, 0x00, b'O', b'B', 0, 0, 2, 0, 0, 0, 0x00, 0x01,
    ]);
    short_element(&mut meta, 0x0002, b"UI", &ui(sop_class));
    short_element(&mut meta, 0x0003, b"UI", &ui(sop_instance));
    short_element(&mut meta, 0x0010, b"UI", &ui(transfer_syntax));
    short_element(&mut meta, 0x0012, b"UI", &ui(IMPLEMENTATION_CLASS_UID));
    short_element(&mut meta, 0x0013, b"SH", &text(IMPLEMENTATION_VERSION_NAME));
    // Source: the AE that wrote the file, pacs-rs under the title it was called by;
    // Sending and Receiving: the two ends of the association
    short_element(&mut meta, 0x0016, b"AE", &text(called));
    short_element(&mut meta, 0x0017, b"AE", &text(calling));
    short_element(&mut meta, 0x0018, b"AE", &text(called));
    let mut file = vec![0u8; 128];
    file.extend_from_slice(b"DICM");
    short_element(&mut file, 0x0000, b"UL", &(meta.len() as u32).to_le_bytes());
    file.extend_from_slice(&meta);
    file.extend_from_slice(data_set);
    file
}

fn short_element(out: &mut Vec<u8>, element: u16, vr: &[u8; 2], value: &[u8]) {
    out.extend_from_slice(&0x0002u16.to_le_bytes());
    out.extend_from_slice(&element.to_le_bytes());
    out.extend_from_slice(vr);
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value);
}

fn ui(uid: &str) -> Vec<u8> {
    let mut value = uid.as_bytes().to_vec();
    if value.len() % 2 == 1 {
        value.push(0);
    }
    value
}

fn text(s: &str) -> Vec<u8> {
    let mut value = s.as_bytes().to_vec();
    if value.len() % 2 == 1 {
        value.push(b' ');
    }
    value
}
