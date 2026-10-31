//! A project: instances sent to its AE title are de-identified by its profile on the way in
//! (PS3.15 Annex E, through `pacs-deid`), and only the de-identified copy reaches the
//! archive. The identified instance is held in memory while that happens and never stored;
//! one the profile quarantines goes to the quarantine store when the deployment has one,
//! and is refused otherwise.

use bytes::Bytes;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject, OpenFileOptions};
use pacs_archive::{Archive, IngestError};
use pacs_deid::{Keys, Outcome, Profile};
use pacs_store::Store;

use crate::ae::AeTitle;
use crate::{IMPLEMENTATION_CLASS_UID, IMPLEMENTATION_VERSION_NAME};

#[derive(Debug, Clone)]
pub struct Project {
    /// The called AE title its instances arrive on.
    pub ae_title: AeTitle,
    pub profile: Profile,
    pub keys: Keys,
    pub archive: Archive,
    /// Where quarantined instances are kept, identified, never indexed or served; `None`
    /// refuses them.
    pub quarantine: Option<Store>,
}

/// What became of an instance sent to a project.
#[derive(Debug, PartialEq, Eq)]
pub enum Received {
    Stored,
    /// Kept in the quarantine store, for this reason.
    Quarantined(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("cannot be read for de-identification: {0}")]
    Read(String),
    /// The profile quarantines it and the deployment keeps no quarantine.
    #[error("refused, not de-identified: {0}")]
    Refused(String),
    #[error("the de-identified instance cannot be written: {0}")]
    Write(String),
    #[error(transparent)]
    Ingest(#[from] IngestError),
    #[error("the quarantine store did not take the instance: {0}")]
    Quarantine(#[from] pacs_store::Error),
}

impl Project {
    /// De-identify a received Part 10 file and ingest the result; or quarantine it.
    pub async fn receive(&self, file: Bytes) -> Result<Received, ProjectError> {
        let profile = self.profile.clone();
        let keys = self.keys.clone();
        let identified = file.clone();
        let done =
            tokio::task::spawn_blocking(move || deidentify_file(&identified, &profile, &keys))
                .await
                .map_err(|e| ProjectError::Read(e.to_string()))??;
        match done {
            Done::Deidentified(bytes) => {
                self.archive.ingest(Bytes::from(bytes)).await?;
                Ok(Received::Stored)
            }
            Done::Quarantined(reason) => {
                let Some(store) = &self.quarantine else {
                    return Err(ProjectError::Refused(reason));
                };
                let key = pacs_core::Instance::from_bytes(&file)
                    .map_err(|e| ProjectError::Read(e.to_string()))?
                    .key;
                store.put(&key, file).await?;
                tracing::info!(project = %self.profile.name, sop_instance = %key.sop_instance_uid, %reason, "quarantined");
                Ok(Received::Quarantined(reason))
            }
        }
    }
}

enum Done {
    Deidentified(Vec<u8>),
    Quarantined(String),
}

/// The de-identified file: the data set by the profile, in the transfer syntax it came in,
/// behind File Meta Information written anew (PS3.15 E.1.1: the original's, AE titles and
/// all, must not leak) with the new SOP Class and Instance UIDs and pacs-rs's implementation.
fn deidentify_file(file: &[u8], profile: &Profile, keys: &Keys) -> Result<Done, ProjectError> {
    let object = OpenFileOptions::new()
        .read_preamble(dicom_object::file::ReadPreamble::Auto)
        .from_reader(file)
        .map_err(|e| ProjectError::Read(e.to_string()))?;
    let transfer_syntax = object
        .meta()
        .transfer_syntax()
        .trim_end_matches('\0')
        .to_owned();
    let dataset: &InMemDicomObject = &object;
    let deidentified = match pacs_deid::deidentify(dataset, profile, keys) {
        Outcome::Quarantined(reason) => return Ok(Done::Quarantined(reason)),
        Outcome::Deidentified(object) => *object,
    };
    let uid = |tag| {
        deidentified
            .element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim_end_matches(['\0', ' ']).to_owned())
            .unwrap_or_default()
    };
    let meta = FileMetaTableBuilder::new()
        .transfer_syntax(transfer_syntax)
        .media_storage_sop_class_uid(uid(dicom_dictionary_std::tags::SOP_CLASS_UID))
        .media_storage_sop_instance_uid(uid(dicom_dictionary_std::tags::SOP_INSTANCE_UID))
        .implementation_class_uid(IMPLEMENTATION_CLASS_UID)
        .implementation_version_name(IMPLEMENTATION_VERSION_NAME);
    let mut out = Vec::new();
    deidentified
        .with_meta(meta)
        .map_err(|e| ProjectError::Write(e.to_string()))?
        .write_all(&mut out)
        .map_err(|e| ProjectError::Write(e.to_string()))?;
    Ok(Done::Deidentified(out))
}
