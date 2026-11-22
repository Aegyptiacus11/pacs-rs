use bytes::Bytes;
use pacs_archive::{Archive, Compression, IngestError, Outcome};
use pacs_core::Instance;
use pacs_core::codec::{EXPLICIT_VR_LE, JPEG_LS_LOSSLESS};
use pacs_core::real::RealSeries;
use pacs_index::{Index, Level};
use pacs_store::Store;

async fn archive() -> Archive {
    Archive::new(Store::memory(), Index::in_memory().await.unwrap())
}

/// One that stores files exactly as received.
async fn uncompressed() -> Archive {
    archive().await.with_compression(Compression::Off)
}

#[tokio::test]
async fn ingest_then_read_the_same_bytes_back() {
    let archive = uncompressed().await;
    let series = RealSeries::new(51, 2);
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
async fn native_pixels_are_stored_as_jpeg_ls_and_decode_back_exactly() {
    let archive = archive().await;
    let series = RealSeries::new(57, 1);
    let sent = series.multiframe_bytes(0);
    archive.ingest(Bytes::from(sent.clone())).await.unwrap();
    let entry = archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(0),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.transfer_syntax_uid, JPEG_LS_LOSSLESS);
    let stored = archive.file(&entry).await.unwrap();
    assert_eq!(entry.size_bytes as usize, stored.len());
    assert!(stored.len() < sent.len());

    let (sent, stored) = (
        Instance::from_bytes(&sent).unwrap(),
        Instance::from_bytes(&stored).unwrap(),
    );
    assert_eq!(stored.key.transfer_syntax_uid, JPEG_LS_LOSSLESS);
    let shape = sent.shape().unwrap();
    for frame in 1..=RealSeries::FRAMES {
        assert_eq!(
            stored.native_frame(frame, shape).unwrap(),
            sent.native_frame(frame, shape).unwrap()
        );
    }
    assert_eq!(
        stored.transcoded(EXPLICIT_VR_LE, true).unwrap().len(),
        sent.transcoded(EXPLICIT_VR_LE, true).unwrap().len()
    );
}

#[tokio::test]
async fn what_the_codecs_do_not_handle_is_stored_as_received() {
    let archive = archive().await;
    let series = RealSeries::new(58, 1);
    // already compressed, in a syntax there is no codec for: a real image in lossy JPEG
    let jpeg = series.jpeg_lossy_bytes(0);
    archive.ingest(Bytes::from(jpeg.clone())).await.unwrap();
    let entry = archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(0),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.file(&entry).await.unwrap(), jpeg);
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
    let first = RealSeries::new(52, 1);
    archive.ingest(first.slice_bytes(0).into()).await.unwrap();

    // same series UID, another study: a conflict
    let mut clash = RealSeries::new(52, 1).in_study(53, 1);
    clash.seed = 52;
    let bytes = clash.slice_bytes(0);
    let err = archive
        .ingest(Bytes::from(bytes.clone()))
        .await
        .unwrap_err();
    assert!(matches!(err, IngestError::Conflict(_)));
    // its file went: the store holds the first one's alone, which the index names
    let objects = store.list().await.unwrap();
    assert_eq!(objects.len(), 1, "{objects:?}");
    let original = pacs_core::Instance::from_bytes(&first.slice_bytes(0))
        .unwrap()
        .key;
    let (_, named) = archive
        .index()
        .instance_file(&original.sop_instance_uid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(objects[0].key, named);
    assert!(store.get(&named).await.is_ok());
}

#[tokio::test]
async fn an_instance_sent_to_another_study_is_refused() {
    let archive = archive().await;
    let series = RealSeries::new(54, 1);
    let other = RealSeries::new(55, 1).study_uid();
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
    let series = RealSeries::new(56, 1);
    let first = series.slice_bytes(0);
    // the same instance, different bytes: another patient name
    let second = series
        .clone()
        .with_patient("OTHER^NAME", None)
        .slice_bytes(0);
    let key = pacs_core::Instance::from_bytes(&first).unwrap().key;

    let archive = Archive::new(store.clone(), Index::in_memory().await.unwrap())
        .with_compression(Compression::Off);
    // the file the index names, and how many files the store holds
    let named = async |archive: &Archive| {
        let (_, object_key) = archive
            .index()
            .instance_file(&key.sop_instance_uid)
            .await
            .unwrap()
            .unwrap();
        (
            store.get(&object_key).await.unwrap(),
            store.list().await.unwrap().len(),
        )
    };
    archive.ingest(first.clone().into()).await.unwrap();
    let again = archive.ingest(second.clone().into()).await.unwrap();
    assert_eq!(again.outcome, Outcome::AlreadyStored);
    assert_eq!(named(&archive).await, (first.clone().into(), 1));

    // replaced: the index names the new file, and the old one is gone
    let archive = archive.with_overwrite(true);
    let again = archive.ingest(second.clone().into()).await.unwrap();
    assert_eq!(again.outcome, Outcome::Replaced);
    assert_eq!(named(&archive).await, (second.clone().into(), 1));
}

#[tokio::test]
async fn deleting_removes_files_and_entries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::local(dir.path(), "").unwrap();
    let archive = Archive::new(store, Index::in_memory().await.unwrap());
    let series = RealSeries::new(55, 3);
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

    let study = pacs_core::public_ids::study("NEMA-55", &series.study_uid());
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

/// Native pixel data goes to JPEG-LS as any other, and the stored file has the elements under
/// their VR, since it is written from what was read.
#[tokio::test]
async fn native_file_with_elements_in_un_is_compressed_with_them_reverted() {
    let archive = archive().await;
    let bytes = pacs_core::real::AlteredFile::new(EXPLICIT_VR_LE)
        .identifiers(95, b"UN")
        .short((0x0028, 0x0002), b"US", &1u16.to_le_bytes())
        .short((0x0028, 0x0004), b"CS", b"MONOCHROME2 ")
        .long((0x0028, 0x0010), b"UN", &2u16.to_le_bytes())
        .long((0x0028, 0x0011), b"UN", &2u16.to_le_bytes())
        .short((0x0028, 0x0100), b"US", &16u16.to_le_bytes())
        .short((0x0028, 0x0101), b"US", &16u16.to_le_bytes())
        .short((0x0028, 0x0102), b"US", &15u16.to_le_bytes())
        .short((0x0028, 0x0103), b"US", &0u16.to_le_bytes())
        .long((0x7FE0, 0x0010), b"OW", &[1, 0, 2, 0, 3, 0, 4, 0])
        .bytes();
    let ingested = archive.ingest(Bytes::from(bytes)).await.unwrap();
    assert_eq!(ingested.key.study_instance_uid, "2.25.951");
    let entry = archive
        .index()
        .instance("2.25.951", "2.25.952", "2.25.953")
        .await
        .unwrap()
        .unwrap();
    let stored = Instance::from_bytes(&archive.file(&entry).await.unwrap()).unwrap();
    assert_eq!(stored.key.transfer_syntax_uid, JPEG_LS_LOSSLESS);
    assert_eq!(stored.int(dicom_dictionary_std::tags::ROWS), Some(2));
}

fn costly_file(seed: u32, values: usize) -> Vec<u8> {
    pacs_core::real::AlteredFile::new(EXPLICIT_VR_LE)
        .identifiers(seed, b"UI")
        .long((0x0043, 0x1001), b"UC", &vec![b'\\'; values])
        .bytes()
}

/// A well-formed file that would take more memory than the limit is refused without being read;
/// the limit is a setting, and one just under it passes.
#[tokio::test]
async fn a_file_over_the_read_limit_is_refused() {
    let file = costly_file(96, 100_000);
    let cost = pacs_core::wire::read_cost(&file).unwrap();
    let archive = uncompressed().await.with_read_limit(cost - 1);
    let refused = archive.ingest(Bytes::from(file.clone())).await.unwrap_err();
    assert!(
        matches!(refused, IngestError::TooCostly { estimate, limit } if estimate == cost && limit == cost - 1),
        "{refused:?}"
    );
    let archive = uncompressed().await.with_read_limit(cost);
    archive.ingest(Bytes::from(file)).await.unwrap();
}

/// A file of lengths that lie is still reported as unreadable, not as costly.
#[tokio::test]
async fn admitting_a_lying_file_says_it_is_invalid() {
    let lying = pacs_core::real::AlteredFile::new(EXPLICIT_VR_LE)
        .identifiers(97, b"UI")
        .long_declared((0x0043, 0x1001), b"OB", 0xC000_0000, b"short")
        .bytes();
    let refused = uncompressed()
        .await
        .ingest(Bytes::from(lying))
        .await
        .unwrap_err();
    assert!(matches!(refused, IngestError::Invalid(_)), "{refused:?}");
}

/// Files over what is free wait for it and then go through, smaller or larger than the limit;
/// with a limit of nothing, and with a pixel file far over it. Nothing hangs.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_read_limit_makes_files_wait_not_fail() {
    let archive = uncompressed().await.with_read_limit(4 << 20);
    let held = archive.admit(&costly_file(98, 20_000)).await.unwrap();
    let waiting = {
        let archive = archive.clone();
        tokio::spawn(async move { archive.ingest(Bytes::from(costly_file(99, 20_000))).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!waiting.is_finished(), "it should wait for room");
    drop(held);
    tokio::time::timeout(std::time::Duration::from_secs(20), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    let native_over_the_limit = pacs_core::real::AlteredFile::new(EXPLICIT_VR_LE)
        .identifiers(100, b"UI")
        .long((0x7FE0, 0x0010), b"OW", &vec![0; 8 << 20])
        .bytes();
    archive
        .ingest(Bytes::from(native_over_the_limit))
        .await
        .unwrap();

    let none = uncompressed().await.with_read_limit(0);
    assert!(matches!(
        none.ingest(Bytes::from(costly_file(101, 1))).await,
        Err(IngestError::TooCostly { .. })
    ));
}

/// A deletion waiting for its lock while files wait for room: all of it finishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_read_limit_and_deletions_do_not_wait_on_each_other() {
    let archive = uncompressed().await.with_read_limit(2 << 20);
    let series = RealSeries::new(102, 1);
    archive
        .ingest(Bytes::from(series.slice_bytes(0)))
        .await
        .unwrap();
    let id = pacs_core::public_ids::study(&series.patient_id(), &series.study_uid());
    let mut tasks = Vec::new();
    for seed in 0..12 {
        let archive = archive.clone();
        tasks.push(tokio::spawn(async move {
            archive
                .ingest(Bytes::from(costly_file(200 + seed, 10_000)))
                .await
                .map(|_| ())
                .unwrap();
        }));
    }
    let deleting = {
        let archive = archive.clone();
        tokio::spawn(async move { archive.delete(Level::Study, &id).await.unwrap() })
    };
    tasks.push(tokio::spawn(async move {
        deleting.await.unwrap();
    }));
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        for task in tasks {
            task.await.unwrap();
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn what_a_request_stored_is_handed_to_who_asks() {
    use pacs_archive::observed::{Stored, stored_during};
    let archive = archive().await;
    let series = RealSeries::new(31, 2).for_patient(3);
    let other = RealSeries::new(32, 1);
    let noted = |series: &RealSeries, outcome| Stored {
        study_instance_uid: series.study_uid(),
        sop_class_uid: "1.2.840.10008.5.1.4.1.1.2".into(),
        patient_id: Some(series.patient_id()),
        patient_name: Some("CompressedSamples^CT1".into()),
        outcome,
    };

    // every instance a request ingests, in order, with whose it is and what became of it
    let (count, stored) = stored_during(async {
        let mut count = 0;
        for bytes in series.all_bytes().into_iter().chain(other.all_bytes()) {
            archive.ingest(Bytes::from(bytes)).await.unwrap();
            count += 1;
        }
        count
    })
    .await;
    assert_eq!(count, 3);
    assert_eq!(
        stored,
        [
            noted(&series, Outcome::Added),
            noted(&series, Outcome::Added),
            noted(&other, Outcome::Added)
        ]
    );

    // a copy already held is noted as such; what is refused is not noted at all
    let (results, stored) = stored_during(async {
        let again = archive.ingest(Bytes::from(series.slice_bytes(0))).await;
        let garbage = archive.ingest(Bytes::from_static(b"not DICOM")).await;
        let elsewhere = archive
            .ingest_in_study(Bytes::from(RealSeries::new(33, 1).slice_bytes(0)), "1.2.3")
            .await;
        (again.is_ok(), garbage.is_err(), elsewhere.is_err())
    })
    .await;
    assert_eq!(results, (true, true, true));
    assert_eq!(stored, [noted(&series, Outcome::AlreadyStored)]);

    // a request that stores nothing has nothing; and outside one, ingest works the same
    let ((), stored) = stored_during(async {}).await;
    assert_eq!(stored, []);
    let alone = archive
        .ingest(Bytes::from(RealSeries::new(34, 1).slice_bytes(0)))
        .await;
    assert_eq!(alone.unwrap().outcome, Outcome::Added);
    // two requests at once keep their own
    let (first, second) = tokio::join!(
        stored_during(archive.ingest(Bytes::from(RealSeries::new(35, 1).slice_bytes(0)))),
        stored_during(archive.ingest(Bytes::from(RealSeries::new(36, 1).slice_bytes(0)))),
    );
    assert_eq!(first.1.len(), 1);
    assert_eq!(second.1.len(), 1);
    assert_ne!(
        first.1[0].study_instance_uid,
        second.1[0].study_instance_uid
    );
}

#[tokio::test]
async fn a_project_s_archive_records_its_studies_and_takes_no_other_s() {
    let archive = uncompressed().await;
    let liver = archive.clone().for_project("liver");
    let series = RealSeries::new(71, 2);
    let study = series.study_uid();

    liver
        .ingest(Bytes::from(series.slice_bytes(0)))
        .await
        .unwrap();
    let project = archive.index().study_project(&study).await.unwrap();
    assert_eq!(project, Some(Some("liver".into())));

    // the rest of the study sent to the archive itself, or to another project: refused,
    // and the file it stored first is removed again
    for other in [archive.clone(), archive.clone().for_project("lung")] {
        let err = other
            .ingest(Bytes::from(series.slice_bytes(1)))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, IngestError::Conflict(said) if said.contains("is in project liver")),
            "{err:?}"
        );
    }
    let stored = archive
        .index()
        .instances(&study, &series.series_uid(), None)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        archive
            .index()
            .study_statistics(&pacs_core::public_ids::study("NEMA-71", &study))
            .await
            .unwrap()
            .unwrap()
            .instances,
        1
    );

    // the instance already there, sent another way, is kept as first stored
    let again = archive
        .ingest(Bytes::from(series.slice_bytes(0)))
        .await
        .unwrap();
    assert_eq!(again.outcome, Outcome::AlreadyStored);
    assert_eq!(
        archive.index().study_project(&study).await.unwrap(),
        Some(Some("liver".into()))
    );

    // once the study is deleted, it can come in the other way
    let id = pacs_core::public_ids::study("NEMA-71", &study);
    archive.delete(Level::Study, &id).await.unwrap().unwrap();
    archive
        .ingest(Bytes::from(series.slice_bytes(1)))
        .await
        .unwrap();
    assert_eq!(
        archive.index().study_project(&study).await.unwrap(),
        Some(None)
    );
}
