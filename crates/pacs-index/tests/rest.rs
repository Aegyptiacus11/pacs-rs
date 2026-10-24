use pacs_core::synthetic::SyntheticSeries;
use pacs_core::{Instance, public_ids};
use pacs_index::{Index, NewInstance, Statistics};

async fn record(index: &Index, series: &SyntheticSeries) -> i64 {
    let mut total = 0;
    for bytes in series.all_bytes() {
        let instance = Instance::from_bytes(&bytes).unwrap();
        let new = NewInstance::new(&instance, "k".into(), bytes.len()).unwrap();
        index.record(&new).await.unwrap();
        total += bytes.len() as i64;
    }
    total
}

#[tokio::test]
async fn studies_series_and_instances_by_public_id() {
    let index = Index::in_memory().await.unwrap();
    let first = SyntheticSeries::new(81, 3);
    let second = SyntheticSeries::new(82, 2).in_study(81, 2);
    let size = record(&index, &second).await + record(&index, &first).await;

    let patient = "SYN81";
    let study_id = public_ids::study(patient, &first.study_uid());
    assert_eq!(index.study_public_ids().await.unwrap(), std::slice::from_ref(&study_id));

    let study = index.rest_study(&study_id).await.unwrap().unwrap();
    assert_eq!(study.study_instance_uid, first.study_uid());
    assert_eq!(study.patient_public_id, public_ids::patient(patient));
    assert_eq!(study.main_tags["AccessionNumber"], "ACC81");
    assert_eq!(study.patient_main_tags["PatientSex"], "O");
    let series_ids = [
        public_ids::series(patient, &first.study_uid(), &first.series_uid()),
        public_ids::series(patient, &first.study_uid(), &second.series_uid()),
    ];
    assert_eq!(study.series, series_ids);
    assert!(study.last_update.ends_with('Z'));

    let series = index.rest_series(&series_ids[0]).await.unwrap().unwrap();
    assert_eq!(series.parent_study, study_id);
    assert_eq!(series.main_tags["Modality"], "CT");
    assert_eq!(series.instances.len(), 3);

    let instance = index
        .rest_instance(&series.instances[2])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(instance.sop_instance_uid, first.instance_uid(2));
    assert_eq!(instance.parent_series, series_ids[0]);
    assert_eq!(instance.parent_study, study_id);
    assert_eq!(instance.instance_number, Some(3));
    let by_uid = index
        .rest_instance_by_uid(&first.instance_uid(2))
        .await
        .unwrap();
    assert_eq!(by_uid.as_ref(), Some(&instance));

    let all = index.rest_study_instances(&study_id).await.unwrap();
    assert_eq!(all.len(), 5);
    assert_eq!(all[0].sop_instance_uid, first.instance_uid(0));
    assert_eq!(all[4].sop_instance_uid, second.instance_uid(1));

    assert_eq!(
        index.study_statistics(&study_id).await.unwrap(),
        Some(Statistics {
            series: 2,
            instances: 5,
            size_bytes: size
        })
    );
    assert_eq!(index.study_statistics("nope").await.unwrap(), None);
    assert!(index.rest_study("nope").await.unwrap().is_none());
    assert!(index.rest_series("nope").await.unwrap().is_none());
    assert!(index.rest_instance("nope").await.unwrap().is_none());
}
