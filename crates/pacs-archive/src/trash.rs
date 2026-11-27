//! Deletion that can be undone: with a trash ([`Archive::with_trash`]), a deletion forgets
//! the resource in the index at once, as without one, but moves its files under `trash/` in
//! the store instead of removing them, and keeps an entry of what went. Restoring indexes
//! the files again, as if received, in their study's project, with the viewer's annotations
//! of a whole study; emptying removes them for good.

use serde_json::{Value, json};

use pacs_index::{Deleted, IntoTrash, Level, TrashFile};

use crate::{Archive, CheckError, Outcome};

/// What a restore did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Restored {
    /// Files indexed again.
    pub restored: usize,
    /// Files left in the trash, with why: an instance stored again since the deletion (the
    /// newer copy stays), or one the archive did not take.
    pub kept: Vec<(String, String)>,
}

impl Archive {
    /// Deletions keep their files `days` days in the trash; 0, the default: deleted at once.
    pub fn with_trash(mut self, days: u64) -> Self {
        self.trash_days = days;
        self
    }

    pub fn trash_days(&self) -> u64 {
        self.trash_days
    }

    /// [`Archive::delete`] with a trash; called with the deletion lock held.
    pub(crate) async fn delete_to_trash(
        &self,
        level: Level,
        public_id: &str,
    ) -> Result<Option<Deleted>, pacs_index::Error> {
        let studies = self.index.audited_by_public_id(level, public_id).await?;
        let mut kept = Vec::new();
        for study in &studies {
            let uid = &study.study_instance_uid;
            kept.push((
                study,
                self.index.project_of_study(uid).await?.flatten(),
                self.index.annotations(uid).await?,
            ));
        }
        // As the entry is written with the deletion, before it is known which studies went
        // whole: with every study's annotations, corrected below.
        let describe = |whole: &dyn Fn(&str) -> bool| -> Value {
            kept.iter()
                .map(|(study, project, annotations)| {
                    json!({
                        "StudyInstanceUID": study.study_instance_uid,
                        "PatientID": study.patient_id,
                        "PatientName": study.patient_name,
                        "project": project,
                        // a study that lost a part keeps its annotations in the index
                        "annotations": if whole(&study.study_instance_uid) {
                            annotations.clone()
                        } else {
                            None
                        },
                    })
                })
                .collect()
        };
        let first = describe(&|_| true);
        let store = self.store.clone();
        let trash_key = move |id: i64, object_key: &str| store.trash_key(id, object_key);
        let mut trash = IntoTrash {
            level: level.name(),
            public_id,
            studies: &first,
            trash_key: &trash_key,
            id: None,
        };
        // The index forgets the resource and notes where each file is to wait, at once: a
        // server that stops from here on leaves files the trash knows, not orphans.
        let Some(deleted) = self
            .index
            .delete_into_trash(level, public_id, &mut trash)
            .await?
        else {
            return Ok(None);
        };
        let Some(trash_id) = trash.id else {
            return Ok(Some(deleted));
        };
        let mut gone = std::collections::HashSet::new();
        for (study, _, _) in &kept {
            let uid = &study.study_instance_uid;
            if self.index.audited_study(uid).await?.is_none() {
                gone.insert(uid.clone());
            }
        }
        let described = describe(&|uid| gone.contains(uid));
        if described != first {
            self.index.set_trash_studies(trash_id, &described).await?;
        }
        for file in self.index.trash_files(trash_id).await? {
            self.move_to_trash(&file).await;
        }
        Ok(Some(deleted))
    }

    /// Move a file the trash lists to where it waits. One that cannot be moved stays where
    /// it was, still the trash's: restoring or emptying its entry finds it there.
    pub(crate) async fn move_to_trash(&self, file: &TrashFile) {
        match self.store.rename(&file.object_key, &file.trash_key).await {
            // not there: moved already
            Ok(()) | Err(pacs_store::Error::NotFound(_)) => {}
            Err(error) => tracing::warn!(
                object_key = %file.object_key,
                %error,
                "a deleted file could not go to the trash; it stays where it was until its \
                 entry is restored or emptied"
            ),
        }
    }

    /// A file of the trash: where it waits, or where it was when its deletion did not get
    /// to move it. `None`: in neither place.
    async fn trashed(
        &self,
        file: &TrashFile,
    ) -> Result<Option<(String, bytes::Bytes)>, CheckError> {
        for key in [&file.trash_key, &file.object_key] {
            match self.store.get(key).await {
                Ok(bytes) => return Ok(Some((key.clone(), bytes))),
                Err(pacs_store::Error::NotFound(_)) => {}
                Err(e) => return Err(CheckError::Store(e)),
            }
        }
        Ok(None)
    }

    /// Put trash entry `id` back: its files indexed again, in their study's project, and the
    /// annotations of a whole study. `None`: no such entry. The entry goes once no file is
    /// left in it.
    pub async fn restore(&self, id: i64) -> Result<Option<Restored>, CheckError> {
        let Some(entry) = self.index.trash_entry(id).await? else {
            return Ok(None);
        };
        let modification = entry.level == "Modification";
        let studies = entry.studies.as_array().cloned().unwrap_or_default();
        let project_of = |study: &str| -> Option<String> {
            studies
                .iter()
                .find(|s| s["StudyInstanceUID"] == study)
                .and_then(|s| s["project"].as_str())
                .map(str::to_owned)
        };
        let mut done = Restored::default();
        for file in self.index.trash_files(id).await? {
            let Some((found_at, bytes)) = self.trashed(&file).await? else {
                self.index.remove_trash_file(&file.trash_key).await?;
                done.kept
                    .push((file.object_key, "gone from the trash".into()));
                continue;
            };
            let study = pacs_core::Instance::from_bytes(&bytes)
                .map(|i| i.key.study_instance_uid)
                .unwrap_or_default();
            let archive = match project_of(&study) {
                Some(project) => self.clone().for_project(&project),
                None => self.clone(),
            };
            // undoing a modification: the file replaces the modified one, which is removed
            let archive = if modification {
                archive.with_overwrite(true)
            } else {
                archive
            };
            match archive.ingest(bytes).await {
                Ok(ingested) if ingested.outcome == Outcome::AlreadyStored => {
                    done.kept.push((
                        file.object_key,
                        "stored again since the deletion: that copy stays".into(),
                    ));
                }
                Ok(_) => {
                    match self.store.delete(&found_at).await {
                        Ok(()) | Err(pacs_store::Error::NotFound(_)) => {}
                        Err(e) => return Err(CheckError::Store(e)),
                    }
                    self.index.remove_trash_file(&file.trash_key).await?;
                    done.restored += 1;
                }
                Err(e) => done.kept.push((file.object_key, format!("not taken: {e}"))),
            }
        }
        for study in &studies {
            let (Some(uid), Some(body)) = (
                study["StudyInstanceUID"].as_str(),
                study["annotations"].as_str(),
            ) else {
                continue;
            };
            if self.index.annotations(uid).await?.is_none() {
                self.index.put_annotations(uid, Some(body)).await?;
            }
        }
        if modification {
            // labels and metadata back to the IDs the old files give
            let back: Vec<(String, String)> = studies
                .iter()
                .filter_map(|s| s["rekeyed"].as_array())
                .flatten()
                .filter_map(|pair| {
                    Some((pair[1].as_str()?.to_owned(), pair[0].as_str()?.to_owned()))
                })
                .collect();
            self.index.rekey(&back).await?;
        }
        if self.index.trash_files(id).await?.is_empty() {
            self.index.remove_trash(id).await?;
        }
        Ok(Some(done))
    }

    /// Remove trash entry `id` and its files for good. `false`: no such entry.
    pub async fn empty_trash(&self, id: i64) -> Result<bool, CheckError> {
        if self.index.trash_entry(id).await?.is_none() {
            return Ok(false);
        }
        for file in self.index.trash_files(id).await? {
            // where it waits, and where it was, had its deletion not got to move it
            for key in [&file.trash_key, &file.object_key] {
                match self.store.delete(key).await {
                    Ok(()) | Err(pacs_store::Error::NotFound(_)) => {}
                    Err(e) => return Err(CheckError::Store(e)),
                }
            }
            self.index.remove_trash_file(&file.trash_key).await?;
        }
        self.index.remove_trash(id).await?;
        Ok(true)
    }

    /// Empty the entries older than the trash keeps them; how many went.
    pub async fn empty_old_trash(&self) -> Result<usize, CheckError> {
        let keep_ms = i64::try_from(self.trash_days.saturating_mul(86_400_000)).unwrap_or(i64::MAX);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let old = self
            .index
            .trash_older_than(now.saturating_sub(keep_ms))
            .await?;
        for id in &old {
            self.empty_trash(*id).await?;
        }
        Ok(old.len())
    }
}
