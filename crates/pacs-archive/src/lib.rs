//! The pacs-rs archive: each instance's file in the store and its entry in the index,
//! kept in step. Every way in (DICOMweb STOW-RS, a plain upload, DIMSE C-STORE later)
//! goes through [`Archive::ingest`], every way out through [`Archive::delete`].

use std::sync::{Arc, Mutex};

mod consistency;
pub mod frames;
pub mod observed;
mod trash;

use bytes::Bytes;
use dicom_dictionary_std::tags;
use pacs_core::codec::{self, JPEG_LS_LOSSLESS};
use pacs_core::{Instance, InstanceKey};
use pacs_index::{Deleted, Index, InstanceEntry, Level, NewInstance, Recorded};
use pacs_store::Store;
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};

/// How much memory reading files may take at once beyond the files themselves, by
/// [`pacs_core::wire::read_cost`], unless set with [`Archive::with_read_limit`].
pub const DEFAULT_READ_LIMIT: u64 = 512 << 20;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    /// Not DICOM, or missing the UIDs an instance must have.
    #[error(transparent)]
    Invalid(#[from] pacs_core::Error),
    /// Reading it would take more memory than the archive allows any file.
    #[error(
        "reading the file would take about {estimate} bytes of memory, over the limit of {limit}"
    )]
    TooCostly { estimate: u64, limit: u64 },
    /// Sent through an archive value limited to an account ([`Archive::limited_to`]).
    #[error("an account limited to projects cannot store: what it sends would be in no project")]
    Limited,
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

/// How native (uncompressed) pixel data is stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Compression {
    /// As received.
    Off,
    /// JPEG-LS lossless, each frame decoded back and compared before the file is kept;
    /// an instance the codec does not handle, or one that does not come back exact, is
    /// stored as received.
    #[default]
    JpegLs,
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
    compression: Compression,
    /// The project whose instances this archive value takes in; none for the archive itself.
    project: Option<String>,
    /// Uploads share it, a deletion holds it alone: a file's storage key comes from its
    /// UIDs, so an upload of an instance being deleted would otherwise have its new file
    /// removed by the deletion. One server process; several on one index would need more.
    writes: Arc<RwLock<()>>,
    /// Refuse a file whose [`pacs_core::wire::read_cost`] is over this.
    read_limit: u64,
    /// The same amount in KiB, shared by the files being read, so that a few at once cannot
    /// add up to what one is refused for.
    reading: Arc<Semaphore>,
    /// Frames of a file at least this large are read on their own ([`frames`]).
    frames_alone_from: u64,
    /// Where the frames of files are, once found.
    layouts: Arc<Mutex<frames::Layouts>>,
    /// Days a deletion keeps its files in the trash; 0: deleted at once ([`trash`]).
    trash_days: u64,
}

/// Room in the archive's read limit, for as long as it is held.
#[derive(Debug)]
pub struct Admitted(#[allow(dead_code)] OwnedSemaphorePermit);

impl Archive {
    /// An archive that keeps the first copy of an instance sent twice, like the
    /// reference server's default; see [`Archive::with_overwrite`].
    pub fn new(store: Store, index: Index) -> Self {
        Archive {
            store,
            index,
            overwrite: false,
            compression: Compression::default(),
            project: None,
            writes: Arc::default(),
            read_limit: DEFAULT_READ_LIMIT,
            reading: Arc::new(Semaphore::new(read_permits(DEFAULT_READ_LIMIT))),
            frames_alone_from: frames::DEFAULT_FRAMES_ALONE_FROM,
            layouts: Arc::default(),
            trash_days: 0,
        }
    }

    /// How much memory reading files may take at once beyond the files themselves; a file
    /// that would take more alone is refused.
    pub fn with_read_limit(mut self, bytes: u64) -> Self {
        self.read_limit = bytes;
        self.reading = Arc::new(Semaphore::new(read_permits(bytes)));
        self
    }

    /// Wait for room to read `file` and hold on to it, or refuse a file over the read limit or
    /// with lengths that lie. To hold while nothing else of this archive is waited for: two
    /// holders waiting for more could wait for ever.
    pub async fn admit(&self, file: &[u8]) -> Result<Admitted, IngestError> {
        let estimate = pacs_core::wire::read_cost(file)?;
        if estimate > self.read_limit {
            return Err(IngestError::TooCostly {
                estimate,
                limit: self.read_limit,
            });
        }
        Ok(self.room_for(estimate + file.len() as u64).await)
    }

    /// Wait for room to read a file the archive has already taken, however large.
    pub async fn admit_stored(&self, file: &[u8]) -> Admitted {
        // A stored file was read once, so an error here is for the read itself to report.
        let estimate = pacs_core::wire::read_cost(file).unwrap_or(0);
        self.room_for(estimate + file.len() as u64).await
    }

    async fn room_for(&self, bytes: u64) -> Admitted {
        // One bigger than the whole limit (a large native file) waits to have all of it.
        let wanted = ((bytes >> 10) + 1).min(read_permits(self.read_limit) as u64) as u32;
        Admitted(
            self.reading
                .clone()
                .acquire_many_owned(wanted)
                .await
                .expect("the semaphore is never closed"),
        )
    }

    /// Whether an instance sent again replaces the stored one.
    pub fn with_overwrite(mut self, overwrite: bool) -> Self {
        self.overwrite = overwrite;
        self
    }

    /// Whether an instance sent again replaces the stored one, so that what is served
    /// under its UIDs can change.
    pub fn overwrites(&self) -> bool {
        self.overwrite
    }

    /// The same archive for what comes in through `project`: its studies are recorded as
    /// that project's, and an instance for a study of another project, or of none, is
    /// refused.
    pub fn for_project(mut self, project: &str) -> Self {
        self.project = Some(project.to_owned());
        self
    }

    /// The same archive as `account` sees it ([`Index::limited_to`]): reads and deletions
    /// stay within the account's projects. Nothing is stored through such a value: what it
    /// took in would be in no project, so out of its own sight.
    pub fn limited_to(&self, account: &str) -> Self {
        Archive {
            index: self.index.limited_to(account),
            ..self.clone()
        }
    }

    /// How native pixel data is stored; JPEG-LS lossless unless set otherwise.
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    /// Store a DICOM Part 10 file and index it: native pixel data compressed as
    /// [`Archive::with_compression`] says, anything else as received.
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
        if self.index.account().is_some() {
            return Err(IngestError::Limited);
        }
        // Before the lock: a deletion waiting for it must not hold up those who have room.
        let _reading = self.admit(&bytes).await?;
        let _writing = self.writes.read().await;
        let instance = Instance::from_bytes(&bytes)?;
        // whose it is, for who accounts for what was stored (see `observed`)
        let patient = (
            instance.string(tags::PATIENT_ID),
            instance.string(tags::PATIENT_NAME),
        );
        let noted = |key: &InstanceKey, outcome: Outcome| {
            observed::note(|| observed::Stored {
                study_instance_uid: key.study_instance_uid.clone(),
                sop_class_uid: key.sop_class_uid.clone(),
                patient_id: patient.0.clone(),
                patient_name: patient.1.clone(),
                outcome,
            })
        };
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
            noted(&instance.key, Outcome::AlreadyStored);
            return Ok(Ingested {
                key: instance.key,
                outcome: Outcome::AlreadyStored,
            });
        }
        let (instance, bytes) = self.compress(instance, bytes).await;
        let size = bytes.len();
        let hash = sha256_hex(&bytes);
        let object_key = self.store.put(&instance.key, bytes).await?;
        let new = NewInstance::new(&instance, object_key.clone(), size)
            .map(|new| new.with_sha256(hash).in_project(self.project.as_deref()));
        let new = match new {
            Ok(new) => new,
            Err(e) => {
                self.store.delete(&object_key).await?;
                return Err(e.into());
            }
        };
        match self.index.record(&new).await {
            Ok(recorded) => {
                let outcome = match recorded {
                    Recorded::Added => Outcome::Added,
                    Recorded::Replaced => Outcome::Replaced,
                };
                noted(&instance.key, outcome);
                Ok(Ingested {
                    key: instance.key,
                    outcome,
                })
            }
            Err(e) => {
                if matches!(e, pacs_index::Error::Conflict(_)) {
                    self.store.delete(&object_key).await?;
                }
                Err(e.into())
            }
        }
    }

    /// `instance` as it is to be stored: compressed when it is native and compression is
    /// on, else unchanged.
    async fn compress(&self, instance: Instance, bytes: Bytes) -> (Instance, Bytes) {
        if self.compression == Compression::Off
            || !codec::is_native(&instance.key.transfer_syntax_uid)
        {
            return (instance, bytes);
        }
        let compressed = tokio::task::spawn_blocking(move || {
            let result = instance
                .transcoded(JPEG_LS_LOSSLESS, true)
                .map_err(|e| e.to_string())
                .and_then(|file| {
                    let parsed = Instance::from_bytes(&file).map_err(|e| e.to_string())?;
                    Ok((parsed, Bytes::from(file)))
                });
            (instance, result)
        })
        .await;
        match compressed {
            Ok((_, Ok(compressed))) => compressed,
            Ok((instance, Err(reason))) => {
                tracing::debug!(
                    sop_instance_uid = %instance.key.sop_instance_uid,
                    %reason,
                    "stored as received, not compressed"
                );
                (instance, bytes)
            }
            Err(e) => std::panic::resume_unwind(e.into_panic()),
        }
    }

    /// Delete a resource and everything under it, index entries first, then files, so the
    /// index never names a file that is gone. `None` when there is no such resource.
    ///
    /// A file that cannot be removed stays in the store, unindexed, and is logged: the
    /// resource is deleted all the same.
    pub async fn delete(
        &self,
        level: Level,
        public_id: &str,
    ) -> Result<Option<Deleted>, pacs_index::Error> {
        let _deleting = self.writes.write().await;
        if self.trash_days > 0 {
            return self.delete_to_trash(level, public_id).await;
        }
        let Some(deleted) = self.index.delete(level, public_id).await? else {
            return Ok(None);
        };
        let failed = self.store.delete_many(deleted.object_keys.clone()).await;
        if !failed.is_empty() {
            tracing::warn!(
                level = level.name(),
                id = public_id,
                left = failed.len(),
                first = %failed[0],
                "deleted from the index, but some files could not be removed"
            );
        }
        Ok(Some(deleted))
    }

    /// The stored file of an indexed instance.
    pub async fn file(&self, entry: &InstanceEntry) -> Result<Bytes, pacs_store::Error> {
        self.file_at(&entry.object_key).await
    }

    /// The stored file under an index entry's `object_key`.
    pub async fn file_at(&self, object_key: &str) -> Result<Bytes, pacs_store::Error> {
        self.store.get(object_key).await
    }
}

/// The limit in KiB, at least one: a semaphore of none would never let anything through.
fn read_permits(bytes: u64) -> usize {
    ((bytes >> 10).max(1) as usize)
        .min(Semaphore::MAX_PERMITS)
        .min(u32::MAX as usize)
}

pub use consistency::{Inconsistencies, LostRepair, OrphanRepair};
pub use trash::Restored;

/// The SHA-256 of `bytes`, lower-case hex: how the index keeps a stored file's.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Why a stored file failed its check.
pub fn file_failure(file: &pacs_index::StoredFile, bytes: &[u8]) -> (String, Option<String>) {
    let hash = sha256_hex(bytes);
    let failure = if bytes.len() as i64 != file.size_bytes {
        Some(format!(
            "{} bytes in the store, {} in the index",
            bytes.len(),
            file.size_bytes
        ))
    } else {
        match &file.sha256 {
            Some(kept) if *kept != hash => {
                Some(format!("changed: SHA-256 {hash}, {kept} when stored"))
            }
            _ => None,
        }
    };
    (hash, failure)
}

impl Archive {
    /// Check one stored file against the index: there, of its size, with its SHA-256 (taken
    /// now for a file stored before hashes were kept). What was wrong, or `None`. Recorded
    /// in the index either way. A store that cannot be read is an error, not a bad file.
    pub async fn check_file(
        &self,
        file: &pacs_index::StoredFile,
    ) -> Result<Option<String>, CheckError> {
        let failure = match self.store.get(&file.object_key).await {
            Ok(bytes) => {
                let (hash, failure) = file_failure(file, &bytes);
                self.index
                    .record_check(&file.sop_instance_uid, Some(&hash), failure.as_deref())
                    .await?;
                failure
            }
            Err(pacs_store::Error::NotFound(_)) => {
                let failure = "missing from the store".to_owned();
                self.index
                    .record_check(&file.sop_instance_uid, None, Some(&failure))
                    .await?;
                Some(failure)
            }
            Err(e) => return Err(CheckError::Store(e)),
        };
        if let Some(failure) = &failure {
            tracing::error!(
                sop_instance_uid = %file.sop_instance_uid,
                object_key = %file.object_key,
                %failure,
                "a stored file failed its check"
            );
        }
        Ok(failure)
    }
}

/// A check that could not be made.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("the store: {0}")]
    Store(pacs_store::Error),
    #[error("the index: {0}")]
    Index(#[from] pacs_index::Error),
}
