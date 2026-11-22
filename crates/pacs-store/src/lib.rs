//! Where pacs-rs keeps DICOM files: a directory on local disk or a bucket in S3-compatible
//! object storage (SeaweedFS, MinIO, R2, AWS S3).
//!
//! Each instance is one object, stored as received, at
//! `<prefix>/<study uid>/<series uid>/<sop instance uid>.dcm`. The index records that
//! key; the store knows nothing about DICOM beyond it.

use std::ops::Range;
use std::sync::Arc;

use bytes::Bytes;
use futures::StreamExt;
use object_store::aws::AmazonS3Builder;
use object_store::local::LocalFileSystem;
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use pacs_core::{InstanceKey, is_valid_uid};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a valid UID for a storage key: {0:?}")]
    InvalidUid(String),
    #[error("no object at {0}")]
    NotFound(String),
    #[error("the object at {object_key} ends before byte {end}")]
    PastEnd { object_key: String, end: u64 },
    #[error(transparent)]
    Backend(#[from] object_store::Error),
    #[error("no random bytes for a storage key: {0}")]
    Random(String),
    #[error("reading the file to store: {0}")]
    Io(std::io::Error),
}

/// An S3-compatible bucket.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// e.g. `http://localhost:8333`; `None` means AWS itself.
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
}

/// An object in the store, as a listing gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub key: String,
    pub size: u64,
    pub modified: std::time::SystemTime,
}

#[derive(Debug, Clone)]
pub struct Store {
    backend: Arc<dyn ObjectStore>,
    prefix: Path,
}

impl Store {
    /// Files under `dir`, which must exist.
    pub fn local(dir: impl AsRef<std::path::Path>, prefix: &str) -> Result<Self, Error> {
        let backend = LocalFileSystem::new_with_prefix(dir)?.with_automatic_cleanup(true);
        Ok(Self::new(Arc::new(backend), prefix))
    }

    /// Objects in an S3-compatible bucket. A custom endpoint uses path-style requests,
    /// which SeaweedFS and MinIO expect, and may be plain HTTP.
    pub fn s3(config: &S3Config, prefix: &str) -> Result<Self, Error> {
        let mut builder = AmazonS3Builder::new()
            .with_region(&config.region)
            .with_bucket_name(&config.bucket)
            .with_access_key_id(&config.access_key_id)
            .with_secret_access_key(&config.secret_access_key);
        if let Some(endpoint) = &config.endpoint {
            builder = builder
                .with_endpoint(endpoint)
                .with_virtual_hosted_style_request(false)
                .with_allow_http(endpoint.starts_with("http://"));
        }
        Ok(Self::new(Arc::new(builder.build()?), prefix))
    }

    /// Objects in memory, for tests.
    pub fn memory() -> Self {
        Self::new(Arc::new(InMemory::new()), "")
    }

    fn new(backend: Arc<dyn ObjectStore>, prefix: &str) -> Self {
        Store {
            backend,
            prefix: Path::from(prefix),
        }
    }

    /// The key an instance is stored under.
    pub fn key_for(&self, key: &InstanceKey) -> Result<String, Error> {
        for uid in [
            &key.study_instance_uid,
            &key.series_instance_uid,
            &key.sop_instance_uid,
        ] {
            if !is_valid_uid(uid) {
                return Err(Error::InvalidUid(uid.clone()));
            }
        }
        let path = self
            .prefix
            .clone()
            .join(key.study_instance_uid.as_str())
            .join(key.series_instance_uid.as_str())
            .join(format!("{}.dcm", key.sop_instance_uid));
        Ok(path.to_string())
    }

    /// Whether the store answers: one request for an object that is not there, which an
    /// answering store says is not found.
    pub async fn ping(&self) -> Result<(), Error> {
        let probe = self.prefix.clone().join("pacs-rs-ready-probe");
        match self.backend.head(&probe).await {
            Ok(_) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Where a file waits in trash entry `trash_id`: under `trash/<id>/` beside the
    /// instances, which are under their Study Instance UID and so never named `trash`.
    pub fn trash_key(&self, trash_id: i64, object_key: &str) -> String {
        let prefix = self.prefix.as_ref();
        let own = object_key
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start_matches('/'))
            .unwrap_or(object_key);
        self.prefix
            .clone()
            .join("trash")
            .join(trash_id.to_string())
            .to_string()
            + "/"
            + own
    }

    /// Whether a key is in the trash.
    pub fn is_trash(&self, key: &str) -> bool {
        let trash = self.prefix.clone().join("trash").to_string();
        key.strip_prefix(&trash)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Move an object: within one store, a rename (on S3 a copy, then a deletion).
    pub async fn rename(&self, from: &str, to: &str) -> Result<(), Error> {
        match self
            .backend
            .rename(&Path::from(from), &Path::from(to))
            .await
        {
            Ok(()) => Ok(()),
            Err(object_store::Error::NotFound { .. }) => Err(Error::NotFound(from.to_owned())),
            Err(e) => Err(e.into()),
        }
    }

    /// Every object under this store's prefix: its key, size and when it was last written.
    pub async fn list(&self) -> Result<Vec<StoredObject>, Error> {
        use futures::TryStreamExt;
        let prefix = (!self.prefix.as_ref().is_empty()).then_some(&self.prefix);
        let objects: Vec<_> = self.backend.list(prefix).try_collect().await?;
        Ok(objects
            .into_iter()
            .map(|meta| StoredObject {
                key: meta.location.to_string(),
                size: meta.size,
                modified: meta.last_modified.into(),
            })
            .collect())
    }

    /// Store an instance's file under a key of its own: its UIDs and a random suffix
    /// (`<study>/<series>/<instance>.<16 hex>.dcm`). Two servers storing the same instance,
    /// or one storing it while another deletes an earlier copy, never touch the same object:
    /// a deletion removes only the files it took out of the index. Returns the key.
    pub async fn put_new(&self, key: &InstanceKey, bytes: Bytes) -> Result<String, Error> {
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).map_err(|e| Error::Random(e.to_string()))?;
        let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
        let fixed = self.key_for(key)?;
        let object_key = format!("{}.{suffix}.dcm", fixed.trim_end_matches(".dcm"));
        self.backend
            .put(&Path::from(object_key.as_str()), bytes.into())
            .await?;
        Ok(object_key)
    }

    /// [`Store::put_new`] of the file at `path`, read and sent a part at a time so that
    /// memory does not grow with its size; each part goes to `seen` as it is read (to hash
    /// it, say). Returns the key. A part that cannot be sent abandons the upload.
    pub async fn put_new_file(
        &self,
        key: &InstanceKey,
        path: &std::path::Path,
        mut seen: impl FnMut(&[u8]),
    ) -> Result<String, Error> {
        use tokio::io::AsyncReadExt;
        const PART: usize = 8 << 20;
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).map_err(|e| Error::Random(e.to_string()))?;
        let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
        let fixed = self.key_for(key)?;
        let object_key = format!("{}.{suffix}.dcm", fixed.trim_end_matches(".dcm"));
        let mut file = tokio::fs::File::open(path).await.map_err(Error::Io)?;
        let upload = self
            .backend
            .put_multipart(&Path::from(object_key.as_str()))
            .await?;
        let mut writer = object_store::WriteMultipart::new_with_chunk_size(upload, PART);
        let mut buffer = vec![0u8; PART];
        loop {
            let read = match file.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => read,
                Err(e) => {
                    writer.abort().await?;
                    return Err(Error::Io(e));
                }
            };
            seen(&buffer[..read]);
            // at most two parts on their way at once: memory stays a few parts' worth
            if let Err(e) = writer.wait_for_capacity(2).await {
                return Err(e.into());
            }
            writer.write(&buffer[..read]);
        }
        writer.finish().await?;
        Ok(object_key)
    }

    /// Store an instance's file, replacing any earlier copy. Returns its key.
    pub async fn put(&self, key: &InstanceKey, bytes: Bytes) -> Result<String, Error> {
        let object_key = self.key_for(key)?;
        self.backend
            .put(&Path::from(object_key.as_str()), bytes.into())
            .await?;
        Ok(object_key)
    }

    pub async fn get(&self, object_key: &str) -> Result<Bytes, Error> {
        let path = Path::from(object_key);
        match self.backend.get(&path).await {
            Ok(result) => Ok(result.bytes().await?),
            Err(object_store::Error::NotFound { .. }) => {
                Err(Error::NotFound(object_key.to_owned()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The bytes of an object in each of `ranges`, in their order, without reading the rest
    /// of it: ranges close to each other come in one request. A range of nothing gives
    /// nothing; one that runs past the end of the object is an error ([`Error::PastEnd`] where
    /// the backend would give what there is of it).
    pub async fn get_ranges(
        &self,
        object_key: &str,
        ranges: &[Range<u64>],
    ) -> Result<Vec<Bytes>, Error> {
        let path = Path::from(object_key);
        // Backends refuse a range of nothing, so those are not asked for.
        let asked: Vec<Range<u64>> = ranges.iter().filter(|r| r.start < r.end).cloned().collect();
        let mut read = match self.backend.get_ranges(&path, &asked).await {
            Ok(read) => read.into_iter(),
            Err(object_store::Error::NotFound { .. }) => {
                return Err(Error::NotFound(object_key.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        ranges
            .iter()
            .map(|range| {
                if range.start >= range.end {
                    return Ok(Bytes::new());
                }
                let bytes = read.next().expect("one answer for each range asked");
                if bytes.len() as u64 != range.end - range.start {
                    return Err(Error::PastEnd {
                        object_key: object_key.to_owned(),
                        end: range.end,
                    });
                }
                Ok(bytes)
            })
            .collect()
    }

    /// The bytes of an object in `range`; see [`Store::get_ranges`].
    pub async fn get_range(&self, object_key: &str, range: Range<u64>) -> Result<Bytes, Error> {
        let mut read = self.get_ranges(object_key, &[range]).await?;
        Ok(read.pop().expect("one answer for the one range"))
    }

    /// Remove an object; removing one that is not there is not an error.
    pub async fn delete(&self, object_key: &str) -> Result<(), Error> {
        match self.backend.delete(&Path::from(object_key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Remove many objects, in batches where the backend has a bulk delete (S3's
    /// DeleteObjects). Objects already gone are not errors; the errors returned are the
    /// objects that could not be removed.
    pub async fn delete_many(&self, object_keys: Vec<String>) -> Vec<Error> {
        let paths = futures::stream::iter(object_keys.into_iter().map(|k| Ok(Path::from(k))));
        self.backend
            .delete_stream(paths.boxed())
            .filter_map(|result| async move {
                match result {
                    Ok(_) | Err(object_store::Error::NotFound { .. }) => None,
                    Err(e) => Some(e.into()),
                }
            })
            .collect()
            .await
    }
}
