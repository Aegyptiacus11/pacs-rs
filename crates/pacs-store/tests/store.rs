use bytes::Bytes;
use pacs_core::Instance;
use pacs_core::synthetic::SyntheticSeries;
use pacs_store::{Error, S3Config, Store};

/// Put a series, read every file back byte for byte, delete one, then the rest at once.
async fn round_trip(store: &Store, seed: u32) {
    let series = SyntheticSeries::new(seed, 3);
    let mut keys = Vec::new();
    for bytes in series.all_bytes() {
        let instance = Instance::from_bytes(&bytes).unwrap();
        let key = store
            .put(&instance.key, Bytes::from(bytes.clone()))
            .await
            .unwrap();
        assert!(key.ends_with(&format!(
            "{}/{}/{}.dcm",
            series.study_uid(),
            series.series_uid(),
            instance.key.sop_instance_uid
        )));
        assert_eq!(store.get(&key).await.unwrap(), bytes);
        keys.push(key);
    }
    store.delete(&keys[0]).await.unwrap();
    assert!(matches!(store.get(&keys[0]).await, Err(Error::NotFound(_))));
    // deleting twice is fine
    store.delete(&keys[0]).await.unwrap();
    assert_eq!(store.get(&keys[1]).await.unwrap(), series.slice_bytes(1));
    // all of them, the one already gone included
    let failed = store.delete_many(keys.clone()).await;
    assert!(failed.is_empty(), "{failed:?}");
    for key in &keys {
        assert!(matches!(store.get(key).await, Err(Error::NotFound(_))));
    }
}

#[tokio::test]
async fn memory_round_trip() {
    round_trip(&Store::memory(), 11).await;
}

#[tokio::test]
async fn local_disk_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::local(dir.path(), "dicom").unwrap();
    let series = SyntheticSeries::new(12, 1);
    let instance = Instance::from_bytes(&series.slice_bytes(0)).unwrap();
    let key = store
        .put(&instance.key, series.slice_bytes(0).into())
        .await
        .unwrap();
    let on_disk = dir
        .path()
        .join("dicom")
        .join(series.study_uid())
        .join(series.series_uid())
        .join(format!("{}.dcm", series.instance_uid(0)));
    assert!(on_disk.is_file(), "{on_disk:?}");
    assert!(store.delete_many(vec![key]).await.is_empty());
    assert!(!on_disk.exists(), "{on_disk:?}");
    round_trip(&store, 15).await;
}

#[tokio::test]
async fn a_key_is_never_built_from_a_bad_uid() {
    let mut key = Instance::from_bytes(&SyntheticSeries::new(13, 1).slice_bytes(0))
        .unwrap()
        .key;
    key.sop_instance_uid = "../../escape".into();
    let store = Store::memory();
    assert!(matches!(store.key_for(&key), Err(Error::InvalidUid(_))));
    assert!(store.put(&key, Bytes::new()).await.is_err());
}

/// Runs against a real S3-compatible server when `PACS_TEST_S3_ENDPOINT` is set, e.g.
/// `docker run --rm -p 8333:8333 chrislusf/seaweedfs:4.48 server -s3` and a bucket made
/// beforehand; see docs/building.md.
#[tokio::test]
async fn s3_round_trip() {
    let Ok(endpoint) = std::env::var("PACS_TEST_S3_ENDPOINT") else {
        eprintln!("PACS_TEST_S3_ENDPOINT not set; skipping");
        return;
    };
    let env = |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    let config = S3Config {
        endpoint: Some(endpoint),
        region: env("PACS_TEST_S3_REGION", "us-east-1"),
        bucket: env("PACS_TEST_S3_BUCKET", "pacs-test"),
        access_key_id: env("PACS_TEST_S3_ACCESS_KEY_ID", "test"),
        secret_access_key: env("PACS_TEST_S3_SECRET_ACCESS_KEY", "test"),
    };
    let store = Store::s3(&config, "dicom").unwrap();
    round_trip(&store, 14).await;
}
