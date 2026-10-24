use bytes::Bytes;
use pacs_archive::{Archive, IngestError};
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Index, Recorded};
use pacs_store::Store;

async fn archive() -> Archive {
    Archive::new(Store::memory(), Index::in_memory().await.unwrap())
}

#[tokio::test]
async fn ingest_then_read_the_same_bytes_back() {
    let archive = archive().await;
    let series = SyntheticSeries::new(51, 2);
    for bytes in series.all_bytes() {
        let ingested = archive.ingest(Bytes::from(bytes)).await.unwrap();
        assert_eq!(ingested.recorded, Recorded::Added);
    }
    let entry = archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(1),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.file(&entry).await.unwrap(), series.slice_bytes(1));
    assert_eq!(entry.size_bytes as usize, series.slice_bytes(1).len());
}

#[tokio::test]
async fn garbage_is_invalid_and_leaves_nothing() {
    let archive = archive().await;
    let err = archive
        .ingest(Bytes::from_static(b"nope"))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Invalid(_)));
    let studies = archive.index().studies(&Default::default()).await.unwrap();
    assert!(studies.is_empty());
}

#[tokio::test]
async fn a_refused_instance_leaves_no_file_behind() {
    let store = Store::memory();
    let archive = Archive::new(store.clone(), Index::in_memory().await.unwrap());
    let first = SyntheticSeries::new(52, 1);
    archive.ingest(first.slice_bytes(0).into()).await.unwrap();

    // same series UID, another study: a conflict
    let mut clash = SyntheticSeries::new(52, 1).in_study(53, 1);
    clash.seed = 52;
    let bytes = clash.slice_bytes(0);
    let err = archive
        .ingest(Bytes::from(bytes.clone()))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Conflict(_)));
    let key = pacs_core::Instance::from_bytes(&bytes).unwrap().key;
    let object_key = store.key_for(&key).unwrap();
    assert!(matches!(
        store.get(&object_key).await,
        Err(pacs_store::Error::NotFound(_))
    ));
    // the first one is untouched
    let original = pacs_core::Instance::from_bytes(&first.slice_bytes(0))
        .unwrap()
        .key;
    assert!(store.get(&store.key_for(&original).unwrap()).await.is_ok());
}
