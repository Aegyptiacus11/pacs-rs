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
    pub recorded: Recorded,
}

#[derive(Debug, Clone)]
pub struct Archive {
    store: Store,
    index: Index,
}

impl Archive {
    pub fn new(store: Store, index: Index) -> Self {
        Archive { store, index }
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
        let instance = Instance::from_bytes(&bytes)?;
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
                recorded,
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
