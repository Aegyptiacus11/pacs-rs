//! Modifying a patient: their name, ID, birth date or sex changed in every stored file, or
//! two patients merged by giving one the other's. New files are written first, the index then
//! names them in one transaction (labels and metadata following the public IDs, which change
//! with the Patient ID), and the files they replace go to the trash, as a deletion's do: a
//! restore of that entry undoes the change. A server that stops part way leaves either the
//! old files named, or the new: the index never names a file that is not there.

use serde_json::json;

use pacs_core::Instance;
use pacs_core::patient::PatientChange;
use pacs_index::{IntoTrash, Level, Moved, NewInstance};

use crate::{Archive, sha256_hex};

/// Why a patient was not modified.
#[derive(Debug, thiserror::Error)]
pub enum ModifyError {
    /// The values asked do not fit DICOM.
    #[error("{0}")]
    Invalid(String),
    /// A study of theirs is a project's de-identified copy: its patient is a pseudonym.
    #[error(
        "study {0} is a de-identified copy in project {1}: its patient is a pseudonym, which is \
         not modified"
    )]
    Project(String, String),
    /// A stored file that could not be read or written again.
    #[error("instance {0}: {1}")]
    File(String, String),
    #[error("the store: {0}")]
    Store(#[from] pacs_store::Error),
    #[error("the index: {0}")]
    Index(#[from] pacs_index::Error),
}

/// What a modification did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modified {
    /// The patient's public ID now.
    pub patient: String,
    /// Instances rewritten.
    pub instances: usize,
    /// The trash entry holding the files replaced: restoring it undoes the change. `None`
    /// when the archive keeps no trash, and the old files are gone.
    pub trash: Option<i64>,
    /// Public IDs that changed, with the labels and metadata moved from each.
    pub rekeyed: Vec<Moved>,
}

impl Archive {
    /// Change the attributes of `change` in every file of patient `public_id`. `None`: no such
    /// patient (or none the account sees).
    pub async fn modify_patient(
        &self,
        public_id: &str,
        change: &PatientChange,
    ) -> Result<Option<Modified>, ModifyError> {
        if let Some(why) = change.invalid() {
            return Err(ModifyError::Invalid(why));
        }
        let _modifying = self.writes.write().await;
        let studies = self
            .index
            .audited_by_public_id(Level::Patient, public_id)
            .await?;
        if studies.is_empty() {
            return Ok(None);
        }
        // the values before, kept in the trash entry for the trail and an undo: name and ID
        // are on each study already
        let before = self
            .index
            .patient_main_tags(public_id)
            .await?
            .unwrap_or_default();
        let was = |key: &str| before.get(key).cloned().unwrap_or(json!(null));
        let mut files = Vec::new();
        for study in &studies {
            let uid = &study.study_instance_uid;
            if let Some(Some(project)) = self.index.project_of_study(uid).await? {
                return Err(ModifyError::Project(uid.clone(), project));
            }
            files.extend(self.index.files_of_study(uid).await?);
        }

        // the new files, beside the old ones
        let mut news: Vec<NewInstance> = Vec::with_capacity(files.len());
        let mut written: Vec<String> = Vec::with_capacity(files.len());
        let mut patient = String::new();
        let outcome = async {
            for file in &files {
                let bytes = self.store.get(&file.object_key).await?;
                let failed = |e: &dyn std::fmt::Display| {
                    ModifyError::File(file.sop_instance_uid.clone(), e.to_string())
                };
                let rewritten = Instance::from_bytes(&bytes)
                    .and_then(|i| i.with_patient(change))
                    .map_err(|e| failed(&e))?;
                let instance = Instance::from_bytes(&rewritten).map_err(|e| failed(&e))?;
                patient = pacs_core::public_ids::patient(
                    &instance
                        .string(dicom_dictionary_std::tags::PATIENT_ID)
                        .unwrap_or_default(),
                );
                let (size, hash) = (rewritten.len(), sha256_hex(&rewritten));
                let key = self.store.put_new(&instance.key, rewritten.into()).await?;
                written.push(key.clone());
                news.push(
                    NewInstance::new(&instance, key, size)
                        .map_err(|e| failed(&e))?
                        .with_sha256(hash),
                );
            }
            Ok::<_, ModifyError>(())
        }
        .await;
        if let Err(error) = outcome {
            self.forget(&written).await;
            return Err(error);
        }

        // as a deletion's entry lists its studies, each with the values it was given
        let changed: serde_json::Map<String, serde_json::Value> = change
            .values()
            .map(|(k, v)| (k.to_owned(), json!(v)))
            .collect();
        let studies_json = json!(
            studies
                .iter()
                .map(|s| {
                    json!({
                        "StudyInstanceUID": s.study_instance_uid,
                        "PatientID": s.patient_id,
                        "PatientName": s.patient_name,
                        "project": null,
                        "PatientBirthDate": was("PatientBirthDate"),
                        "PatientSex": was("PatientSex"),
                        "changed": changed,
                    })
                })
                .collect::<Vec<_>>()
        );
        let store = self.store.clone();
        let trash_key = move |id: i64, object_key: &str| store.trash_key(id, object_key);
        let mut trash = IntoTrash {
            level: "Modification",
            public_id,
            studies: &studies_json,
            trash_key: &trash_key,
            id: None,
        };
        let rekeyed = match self.index.replace_instances(&news, &mut trash).await {
            Ok(rekeyed) => rekeyed,
            Err(error) => {
                self.forget(&written).await;
                return Err(error.into());
            }
        };
        let trash_id = trash.id.expect("replace_instances keeps a trash entry");
        // the old files to where the entry says they wait, or gone at once without a trash
        for file in self.index.trash_files(trash_id).await? {
            self.move_to_trash(&file).await;
        }
        let trash = if self.trash_days > 0 {
            Some(trash_id)
        } else {
            self.empty_trash(trash_id).await.map_err(|e| match e {
                crate::CheckError::Store(e) => ModifyError::Store(e),
                crate::CheckError::Index(e) => ModifyError::Index(e),
            })?;
            None
        };
        Ok(Some(Modified {
            patient,
            instances: news.len(),
            trash,
            rekeyed,
        }))
    }

    /// Merge patient `from` into patient `into`: `from`'s files get `into`'s name, ID, birth
    /// date and sex, and with them `into`'s public ID. `None`: either is not there.
    pub async fn merge_patients(
        &self,
        from: &str,
        into: &str,
    ) -> Result<Option<Modified>, ModifyError> {
        if from == into {
            return Err(ModifyError::Invalid(
                "a patient cannot be merged into themselves".into(),
            ));
        }
        let Some(target) = self.index.patient_main_tags(into).await? else {
            return Ok(None);
        };
        let get = |key: &str| {
            target
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_owned()
        };
        let change = PatientChange {
            name: Some(get("PatientName")),
            id: Some(get("PatientID")),
            birth_date: Some(get("PatientBirthDate")),
            sex: Some(get("PatientSex")),
        };
        self.modify_patient(from, &change).await
    }

    /// Files the index does not name: removed, or left for the consistency check. Not an
    /// error: the index is right either way, and the check reports and removes what is left.
    pub(crate) async fn forget(&self, keys: &[String]) {
        let failed = self.store.delete_many(keys.to_vec()).await;
        if !failed.is_empty() {
            tracing::warn!(
                left = failed.len(),
                first = %failed[0],
                "a patient modification or its undo left files the index does not name"
            );
        }
    }
}
