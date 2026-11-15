mod common;

use pacs_core::synthetic::SyntheticSeries;
use pacs_core::{Instance, public_ids};
use pacs_index::{Index, Level, NewInstance, Statistics};

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
    for index in common::indexes().await {
        let first = SyntheticSeries::new(81, 3);
        let second = SyntheticSeries::new(82, 2).in_study(81, 2);
        let size = record(&index, &second).await + record(&index, &first).await;

        let patient = "SYN81";
        let study_id = public_ids::study(patient, &first.study_uid());
        assert_eq!(
            index.study_public_ids().await.unwrap(),
            std::slice::from_ref(&study_id)
        );

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
        // YYYYMMDDTHHMMSS, and just arrived, so not stable yet
        assert_eq!(study.last_update.len(), 15);
        assert_eq!(&study.last_update[8..9], "T");
        assert!(!study.is_stable);

        let series = index.rest_series(&series_ids[0]).await.unwrap().unwrap();
        assert_eq!(series.parent_study, study_id);
        assert_eq!(series.main_tags["Modality"], "CT");
        assert_eq!(series.instances.len(), 3);
        assert_eq!(
            index.series_public_id(&first.series_uid()).await.unwrap(),
            Some(series_ids[0].clone())
        );

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
                studies: 1,
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
}

#[tokio::test]
async fn the_studies_a_resource_lies_in_for_the_audit_trail() {
    for index in common::indexes().await {
        // one patient with two studies, the first of two series; and another patient
        let first = SyntheticSeries::new(71, 3).for_patient(7);
        let second = SyntheticSeries::new(72, 2).in_study(71, 2).for_patient(7);
        let other_study = SyntheticSeries::new(73, 1).for_patient(7);
        let other_patient = SyntheticSeries::new(74, 1);
        for series in [&first, &second, &other_study, &other_patient] {
            record(&index, series).await;
        }
        let ct = "1.2.840.10008.5.1.4.1.1.2".to_owned();

        let study = index
            .audited_study(&first.study_uid())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(study.study_instance_uid, first.study_uid());
        assert_eq!(study.patient_id.as_deref(), Some("SYN7"));
        assert_eq!(study.patient_name.as_deref(), Some("SYNTHETIC^SERIES"));
        // both series' instances, counted by SOP class
        assert_eq!(study.sop_classes, [(ct.clone(), 5)]);
        assert_eq!(index.audited_study("1.2.3").await.unwrap(), None);

        let id = |level: &str, uids: &[&str]| match level {
            "patient" => public_ids::patient(uids[0]),
            "study" => public_ids::study(uids[0], uids[1]),
            "series" => public_ids::series(uids[0], uids[1], uids[2]),
            _ => public_ids::instance(uids[0], uids[1], uids[2], uids[3]),
        };
        let study_uid = first.study_uid();
        let by = async |level: Level, public_id: String| {
            let found = index.audited_by_public_id(level, &public_id).await.unwrap();
            found
                .into_iter()
                .map(|s| (s.study_instance_uid, s.sop_classes))
                .collect::<Vec<_>>()
        };
        // a study, a series of it and an instance of it are all in that one study
        let whole = vec![(study_uid.clone(), vec![(ct.clone(), 5)])];
        assert_eq!(
            by(Level::Study, id("study", &["SYN7", &study_uid])).await,
            whole
        );
        assert_eq!(
            by(
                Level::Series,
                id("series", &["SYN7", &study_uid, &second.series_uid()])
            )
            .await,
            whole
        );
        assert_eq!(
            by(
                Level::Instance,
                id(
                    "instance",
                    &[
                        "SYN7",
                        &study_uid,
                        &first.series_uid(),
                        &first.instance_uid(1)
                    ]
                )
            )
            .await,
            whole
        );
        // a patient: every study of theirs, and none of anyone else's
        let mut of_patient = by(Level::Patient, id("patient", &["SYN7"])).await;
        of_patient.sort();
        let mut expected = vec![
            (study_uid.clone(), vec![(ct.clone(), 5)]),
            (other_study.study_uid(), vec![(ct.clone(), 1)]),
        ];
        expected.sort();
        assert_eq!(of_patient, expected);
        // an ID of another level, or of nothing, is in no study
        assert_eq!(
            by(Level::Series, id("study", &["SYN7", &study_uid])).await,
            []
        );
        assert_eq!(by(Level::Study, "nothing".into()).await, []);
    }
}
