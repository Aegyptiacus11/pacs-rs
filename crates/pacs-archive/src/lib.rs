//! The pacs-rs archive: each instance's file in the store and its entry in the index,
//! kept in step. Every way in (DICOMweb STOW-RS, a plain upload, DIMSE C-STORE later)
//! goes through [`Archive::ingest`].

use bytes::Bytes;
use pacs_core::{Instance, InstanceKey};
use pacs_index::{Index, InstanceEntry, NewInstance, Recorded};
use pacs_store::Store;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// Not DICOM, or missing the UIDs an instance must have.
    #[error(transparent)]
    Invalid(#[from] pacs_core::Error),
    /// The UIDs contradict what the archive already holds.
    #[error("{0}")]
    Conflict(String),
    /// Sent to one study but belongs to another.
    #[error("instance {sop_instance_uid} is in study {actual}, not {expected}")]
    WrongStudy {
        sop_instance_uid: String,
        expected: String,
        actual: String,
    },
    #[error(transparent)]
    Store(#[from] pacs_store::Error),
    #[error(transparent)]
    Index(pacs_index::Error),
}

impl From<pacs_index::Error> for IngestError {
    fn from(e: pacs_index::Error) -> Self {
        match e {
            pacs_index::Error::Conflict(message) => IngestError::Conflict(message),
            other => IngestError::Index(other),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingested {
    pub key: InstanceKey,
    pub outcome: Outcome,
}

/// What became of an ingested instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Added,
    /// It was there already and the new file took its place (overwriting is on).
    Replaced,
    /// It was there already and was kept as it was (overwriting is off).
    AlreadyStored,
}

#[derive(Debug, Clone)]
pub struct Archive {
    store: Store,
    index: Index,
    overwrite: bool,
}

impl Archive {
    /// An archive that keeps the first copy of an instance sent twice, like the
    /// reference server's default; see [`Archive::with_overwrite`].
    pub fn new(store: Store, index: Index) -> Self {
        Archive {
            store,
            index,
            overwrite: false,
        }
    }

    /// Whether an instance sent again replaces the stored one.
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Store a DICOM Part 10 file as received and index it.
    ///
    /// The file goes to the store first, so an indexed instance always has its file. If
    /// the index then refuses the instance, the file just written is removed again: its
    /// key comes from the refused UIDs, so it is never the file of an indexed instance.
    pub async fn ingest(&self, bytes: Bytes) -> Result<Ingested, IngestError> {
        self.ingest_checked(bytes, None).await
    }

    /// [`Archive::ingest`], refusing an instance that is not in study `study_instance_uid`.
    pub async fn ingest_in_study(
        &self,
        bytes: Bytes,
        study_instance_uid: &str,
    ) -> Result<Ingested, IngestError> {
        self.ingest_checked(bytes, Some(study_instance_uid)).await
    }

    async fn ingest_checked(
        &self,
        bytes: Bytes,
        expected_study: Option<&str>,
    ) -> Result<Ingested, IngestError> {
        let instance = Instance::from_bytes(&bytes)?;
        if let Some(expected) = expected_study
            && expected != instance.key.study_instance_uid
        {
            return Err(IngestError::WrongStudy {
                sop_instance_uid: instance.key.sop_instance_uid.clone(),
                expected: expected.to_owned(),
                actual: instance.key.study_instance_uid.clone(),
            });
        }
        if !self.overwrite
            && let Some(existing) = self
                .index
                .rest_instance_by_uid(&instance.key.sop_instance_uid)
                .await?
            && existing.series_instance_uid == instance.key.series_instance_uid
            && existing.study_instance_uid == instance.key.study_instance_uid
        {
            return Ok(Ingested {
                key: instance.key,
                outcome: Outcome::AlreadyStored,
            });
        }
        let size = bytes.len();
        let object_key = self.store.put(&instance.key, bytes).await?;
        let new = match NewInstance::new(&instance, object_key.clone(), size) {
            Ok(new) => new,
            Err(e) => {
                self.store.delete(&object_key).await?;
                return Err(e.into());
            }
        };
        match self.index.record(&new).await {
            Ok(recorded) => Ok(Ingested {
                key: instance.key,
                outcome: match recorded {
                    Recorded::Added => Outcome::Added,
                    Recorded::Replaced => Outcome::Replaced,
                },
            }),
            Err(e) => {
                if matches!(e, pacs_index::Error::Conflict(_)) {
                    self.store.delete(&object_key).await?;
                }
                Err(e.into())
            }
        }
    }

    /// The stored file of an indexed instance.
    pub async fn file(&self, entry: &InstanceEntry) -> Result<Bytes, pacs_store::Error> {
        self.store.get(&entry.object_key).await
    }
}
