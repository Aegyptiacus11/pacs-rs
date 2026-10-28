use bytes::Bytes;
use pacs_archive::{Archive, IngestError, Outcome};
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Index, Level};
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
        assert_eq!(ingested.outcome, Outcome::Added);
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

#[tokio::test]
async fn an_instance_sent_to_another_study_is_refused() {
    let archive = archive().await;
    let series = SyntheticSeries::new(54, 1);
    let other = SyntheticSeries::new(55, 1).study_uid();
    let err = archive
        .ingest_in_study(series.slice_bytes(0).into(), &other)
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::WrongStudy { .. }));
    let ok = archive
        .ingest_in_study(series.slice_bytes(0).into(), &series.study_uid())
        .await
        .unwrap();
    assert_eq!(ok.outcome, Outcome::Added);
}

#[tokio::test]
async fn the_first_copy_is_kept_unless_overwriting() {
    let store = Store::memory();
    let series = SyntheticSeries::new(56, 1);
    let first = series.slice_bytes(0);
    // the same instance, different bytes: another patient name
    let second = series
        .clone()
        .with_patient("OTHER^NAME", None)
        .slice_bytes(0);
    let key = pacs_core::Instance::from_bytes(&first).unwrap().key;

    let archive = Archive::new(store.clone(), Index::in_memory().await.unwrap());
    archive.ingest(first.clone().into()).await.unwrap();
    let again = archive.ingest(second.clone().into()).await.unwrap();
    assert_eq!(again.outcome, Outcome::AlreadyStored);
    assert_eq!(
        store.get(&store.key_for(&key).unwrap()).await.unwrap(),
        first
    );

    let archive = archive.with_overwrite(true);
    let again = archive.ingest(second.clone().into()).await.unwrap();
    assert_eq!(again.outcome, Outcome::Replaced);
    assert_eq!(
        store.get(&store.key_for(&key).unwrap()).await.unwrap(),
        second
    );
}

#[tokio::test]
async fn deleting_removes_files_and_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::local(dir.path(), "").unwrap();
    let archive = Archive::new(store, Index::in_memory().await.unwrap());
    let series = SyntheticSeries::new(55, 3);
    for bytes in series.all_bytes() {
        archive.ingest(Bytes::from(bytes)).await.unwrap();
    }
    let files = || {
        walk(dir.path())
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "dcm"))
            .count()
    };
    assert_eq!(files(), 3);

    let study = pacs_core::public_ids::study("SYN55", &series.study_uid());
    let deleted = archive.delete(Level::Study, &study).await.unwrap().unwrap();
    assert_eq!(deleted.object_keys.len(), 3);
    assert_eq!(deleted.remaining_ancestor, None);
    assert_eq!(files(), 0);
    assert!(archive.index().rest_study(&study).await.unwrap().is_none());
    assert!(
        archive
            .delete(Level::Study, &study)
            .await
            .unwrap()
            .is_none()
    );

    // and it can be sent again
    let again = archive.ingest(series.slice_bytes(0).into()).await.unwrap();
    assert_eq!(again.outcome, Outcome::Added);
    assert_eq!(files(), 1);
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(walk(&path));
        } else {
            found.push(path);
        }
    }
    found
}
