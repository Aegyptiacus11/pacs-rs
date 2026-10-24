use pacs_core::Instance;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Error, Index, NewInstance, Recorded, StudyFilter};

async fn record_series(index: &Index, series: &SyntheticSeries) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = format!("dicom/{slice}.dcm");
        let new = NewInstance::new(&instance, key, bytes.len()).unwrap();
        assert_eq!(index.record(&new).await.unwrap(), Recorded::Added);
    }
}

#[tokio::test]
async fn a_study_with_two_series_has_its_counts() {
    let index = Index::in_memory().await.unwrap();
    let first = SyntheticSeries::new(21, 3);
    let second = SyntheticSeries::new(22, 2).in_study(21, 2);
    record_series(&index, &second).await;
    record_series(&index, &first).await;

    let study = index.study(&first.study_uid()).await.unwrap().unwrap();
    assert_eq!(study.series_count, 2);
    assert_eq!(study.instance_count, 5);
    assert_eq!(study.modalities, ["CT"]);
    assert_eq!(study.attributes["00100020"]["Value"][0], "SYN21");

    // by series number, whatever the order received
    let series = index.series(&first.study_uid(), None).await.unwrap();
    let uids: Vec<_> = series
        .iter()
        .map(|s| s.series_instance_uid.as_str())
        .collect();
    assert_eq!(uids, [first.series_uid(), second.series_uid()]);
    assert_eq!(series[0].instance_count, 3);
    assert_eq!(series[1].instance_count, 2);
    assert_eq!(series[1].attributes["00200011"]["Value"][0], 2);

    let one = index
        .series(&first.study_uid(), Some(&second.series_uid()))
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
}

#[tokio::test]
async fn instances_and_metadata_come_in_instance_number_order() {
    let index = Index::in_memory().await.unwrap();
    let series = SyntheticSeries::new(23, 4);
    // receive the slices backwards
    for bytes in series.all_bytes().iter().rev() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let new = NewInstance::new(&instance, "k".into(), bytes.len()).unwrap();
        index.record(&new).await.unwrap();
    }
    let instances = index
        .instances(&series.study_uid(), &series.series_uid(), None)
        .await
        .unwrap();
    let uids: Vec<_> = instances
        .iter()
        .map(|i| i.sop_instance_uid.clone())
        .collect();
    let expected: Vec<_> = (0..4).map(|s| series.instance_uid(s)).collect();
    assert_eq!(uids, expected);
    assert_eq!(instances[0].transfer_syntax_uid, "1.2.840.10008.1.2.1");

    let metadata = index
        .metadata(&series.study_uid(), &series.series_uid(), None)
        .await
        .unwrap();
    assert_eq!(metadata.len(), 4);
    assert_eq!(metadata[0]["00080018"]["Value"][0], series.instance_uid(0));
    assert_eq!(metadata[3]["00200013"]["Value"][0], 4);
    assert!(metadata[0].get("7FE00010").is_none());

    let one = index
        .metadata(
            &series.study_uid(),
            &series.series_uid(),
            Some(&series.instance_uid(2)),
        )
        .await
        .unwrap();
    assert_eq!(one.len(), 1);
    assert_eq!(one[0]["00200013"]["Value"][0], 3);
}

#[tokio::test]
async fn an_instance_is_found_only_under_its_own_study_and_series() {
    let index = Index::in_memory().await.unwrap();
    let series = SyntheticSeries::new(24, 1);
    record_series(&index, &series).await;
    let sop = series.instance_uid(0);
    let found = index
        .instance(&series.study_uid(), &series.series_uid(), &sop)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.object_key, "dicom/0.dcm");
    let other = SyntheticSeries::new(25, 1);
    assert!(
        index
            .instance(&other.study_uid(), &series.series_uid(), &sop)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        index
            .instance(&series.study_uid(), &other.series_uid(), &sop)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn resending_replaces_and_moving_between_studies_is_refused() {
    let index = Index::in_memory().await.unwrap();
    let series = SyntheticSeries::new(26, 1);
    record_series(&index, &series).await;

    let bytes = series.slice_bytes(0);
    let instance = Instance::from_bytes(&bytes).unwrap();
    let again = NewInstance::new(&instance, "dicom/new.dcm".into(), bytes.len()).unwrap();
    assert_eq!(index.record(&again).await.unwrap(), Recorded::Replaced);
    let study = index.study(&series.study_uid()).await.unwrap().unwrap();
    assert_eq!(study.instance_count, 1);

    // the same series UID claimed by another study
    let mut moved = again.clone();
    moved.study_instance_uid = SyntheticSeries::new(27, 1).study_uid();
    assert!(matches!(
        index.record(&moved).await,
        Err(Error::Conflict(_))
    ));
    // nothing half-written
    assert!(
        index
            .study(&moved.study_instance_uid)
            .await
            .unwrap()
            .is_none()
    );

    // the same instance UID claimed by another series
    let mut moved = again.clone();
    moved.series_instance_uid = SyntheticSeries::new(28, 1).series_uid();
    assert!(matches!(
        index.record(&moved).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        index.series(&series.study_uid(), None).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn studies_filter_by_patient_and_paginate() {
    let index = Index::in_memory().await.unwrap();
    for seed in [31, 32, 33] {
        record_series(&index, &SyntheticSeries::new(seed, 1)).await;
    }
    let all = index.studies(&StudyFilter::default()).await.unwrap();
    assert_eq!(all.len(), 3);
    let by_patient = StudyFilter {
        patient_id: Some("SYN32".into()),
        ..StudyFilter::default()
    };
    let found = index.studies(&by_patient).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].study_instance_uid,
        SyntheticSeries::new(32, 1).study_uid()
    );
    let page = StudyFilter {
        limit: Some(2),
        offset: Some(2),
        ..StudyFilter::default()
    };
    assert_eq!(index.studies(&page).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_file_index_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index.sqlite");
    let series = SyntheticSeries::new(34, 2);
    {
        let index = Index::open(&path).await.unwrap();
        record_series(&index, &series).await;
    }
    let index = Index::open(&path).await.unwrap();
    let study = index.study(&series.study_uid()).await.unwrap().unwrap();
    assert_eq!(study.instance_count, 2);
}
