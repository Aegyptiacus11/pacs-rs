//! A project: instances sent to its AE title are de-identified by its profile on the way in
//! (PS3.15 Annex E, through `pacs-deid`), and only the de-identified copy reaches the
//! archive. The identified instance is held in memory while that happens and never stored;
//! one the profile quarantines goes to the quarantine store when the deployment has one,
//! and is refused otherwise.
//!
//! What each copy replaced is recorded in the index ([`pacs_index::OriginalStudy`] and the
//! UID pairs), so that a result made on the copy can be sent back to the original study.

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
    #[error("the way back to the originals cannot be read or recorded: {0}")]
    Originals(String),
    #[error("the quarantine store did not take the instance: {0}")]
    Quarantine(#[from] pacs_store::Error),
}

impl Project {
    /// De-identify a received Part 10 file and ingest the result; or quarantine it.
    pub async fn receive(&self, file: Bytes) -> Result<Received, ProjectError> {
        if self.is_result(&file).await? {
            self.archive.clone().as_received().ingest(file).await?;
            return Ok(Received::Stored);
        }
        let profile = self.profile.clone();
        let keys = self.keys.clone();
        let identified = file.clone();
        let reading = self.archive.admit(&file).await?;
        let done =
            tokio::task::spawn_blocking(move || deidentify_file(&identified, &profile, &keys))
                .await
                .map_err(|e| ProjectError::Read(e.to_string()))??;
        drop(reading); // the ingest takes its own: held here, both could wait on each other.
        match done {
            Done::Deidentified(bytes, originals) => {
                // before the copy: once a copy is there, a result may be made on it
                self.archive
                    .index()
                    .record_originals(
                        &self.profile.name,
                        &originals.uids,
                        &originals.replaced_study,
                        &originals.study,
                    )
                    .await
                    .map_err(|e| ProjectError::Originals(e.to_string()))?;
                self.archive.ingest(Bytes::from(bytes)).await?;
                Ok(Received::Stored)
            }
            Done::Quarantined(reason) => {
                let Some(store) = &self.quarantine else {
                    return Err(ProjectError::Refused(reason));
                };
                let reading = self.archive.admit(&file).await?;
                let key = pacs_core::Instance::from_bytes(&file)
                    .map_err(|e| ProjectError::Read(e.to_string()))?
                    .key;
                drop(reading);
                store.put(&key, file).await?;
                tracing::info!(project = %self.profile.name, sop_instance = %key.sop_instance_uid, %reason, "quarantined");
                Ok(Received::Quarantined(reason))
            }
        }
    }
}

impl Project {
    /// Whether `file` is a result made on one of this project's de-identified studies (a
    /// segmentation, a report): it refers to the replaced UIDs already, and is stored as it
    /// comes. A result for another project's study is refused.
    async fn is_result(&self, file: &Bytes) -> Result<bool, ProjectError> {
        let study = {
            let _reading = self.archive.admit(file).await?;
            pacs_core::Instance::from_bytes(file)
                .map_err(|e| ProjectError::Read(e.to_string()))?
                .key
                .study_instance_uid
        };
        let index = self.archive.index();
        let Some(Some(project)) = index
            .study_project(&study)
            .await
            .map_err(|e| ProjectError::Originals(e.to_string()))?
        else {
            return Ok(false);
        };
        let copy = index
            .original_study(&project, &study)
            .await
            .map_err(|e| ProjectError::Originals(e.to_string()))?
            .is_some();
        if copy && project != self.profile.name {
            return Err(ProjectError::Refused(format!(
                "a result for a study of project {project}, sent to {}",
                self.profile.name
            )));
        }
        Ok(copy)
    }
}

enum Done {
    Deidentified(Vec<u8>, Originals),
    Quarantined(String),
}

/// What one de-identified instance replaced.
struct Originals {
    uids: Vec<(String, String)>,
    replaced_study: String,
    study: pacs_index::OriginalStudy,
}

/// The de-identified file: the data set by the profile, in the transfer syntax it came in,
/// behind File Meta Information written anew (PS3.15 E.1.1: the original's, AE titles and
/// all, must not leak) with the new SOP Class and Instance UIDs and pacs-rs's implementation.
fn deidentify_file(file: &[u8], profile: &Profile, keys: &Keys) -> Result<Done, ProjectError> {
    let object = OpenFileOptions::new()
        .read_preamble(dicom_object::file::ReadPreamble::Auto)
        .from_reader(
            &*pacs_core::wire::checked(file).map_err(|e| ProjectError::Read(e.to_string()))?,
        )
        .map_err(|e| ProjectError::Read(e.to_string()))?;
    let transfer_syntax = object
        .meta()
        .transfer_syntax()
        .trim_end_matches('\0')
        .to_owned();
    let original = pacs_core::Instance {
        key: pacs_core::InstanceKey {
            study_instance_uid: String::new(),
            series_instance_uid: String::new(),
            sop_instance_uid: String::new(),
            sop_class_uid: String::new(),
            transfer_syntax_uid: transfer_syntax.clone(),
        },
        object,
    };
    let dataset: &InMemDicomObject = &original.object;
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
    let originals = Originals {
        uids: pacs_deid::replaced_uids(dataset, &deidentified, keys),
        replaced_study: uid(dicom_dictionary_std::tags::STUDY_INSTANCE_UID),
        study: pacs_index::OriginalStudy {
            patient: original.main_tags(pacs_core::main_tags::PATIENT),
            study: original.main_tags(pacs_core::main_tags::STUDY),
        },
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
    Ok(Done::Deidentified(out, originals))
}

/// A result's file under the originals: the UIDs `project` replaced, and the patient and
/// study of `original`, in the transfer syntax it is stored in (see
/// [`pacs_deid::reidentify`]).
pub async fn reidentified(
    index: &pacs_index::Index,
    project: &str,
    original: &pacs_index::OriginalStudy,
    file: Bytes,
) -> Result<Bytes, String> {
    let instance = pacs_core::Instance::from_bytes(&file).map_err(|e| e.to_string())?;
    let uids = pacs_deid::uids_in(&instance.object);
    let originals = index
        .original_uids(project, &uids)
        .await
        .map_err(|e| format!("the index: {e}"))?;
    let back = pacs_deid::reidentify(
        &instance.object,
        &originals,
        &original.patient,
        &original.study,
    )
    .map_err(|e| e.to_string())?;
    let uid = |tag| {
        back.element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim_end_matches(['\0', ' ']).to_owned())
            .unwrap_or_default()
    };
    let meta = FileMetaTableBuilder::new()
        .transfer_syntax(instance.key.transfer_syntax_uid.as_str())
        .media_storage_sop_class_uid(uid(dicom_dictionary_std::tags::SOP_CLASS_UID))
        .media_storage_sop_instance_uid(uid(dicom_dictionary_std::tags::SOP_INSTANCE_UID))
        .implementation_class_uid(IMPLEMENTATION_CLASS_UID)
        .implementation_version_name(IMPLEMENTATION_VERSION_NAME);
    let mut out = Vec::new();
    back.with_meta(meta)
        .map_err(|e| e.to_string())?
        .write_all(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out.into())
}
