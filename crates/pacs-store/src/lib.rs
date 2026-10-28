//! Where pacs-rs keeps DICOM files: a directory on local disk or a bucket in S3-compatible
//! object storage (SeaweedFS, MinIO, R2, AWS S3).
//!
//! Each instance is one object, stored as received, at
//! `<prefix>/<study uid>/<series uid>/<sop instance uid>.dcm`. The index records that
//! key; the store knows nothing about DICOM beyond it.

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
    #[error(transparent)]
    Backend(#[from] object_store::Error),
}

/// An S3-compatible bucket.
#[derive(Debug, Clone)]
pub struct S3Config {
    /// e.g. `http://homeserver:8333`; `None` means AWS itself.
    pub endpoint: Option<String>,
    pub region: String,
    pub bucket: String,
    pub access_key_id: String,
    pub secret_access_key: String,
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
