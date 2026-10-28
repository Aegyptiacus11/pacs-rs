mod common;

use pacs_core::synthetic::SyntheticSeries;
use pacs_core::{Instance, public_ids};
use pacs_index::{Deleted, Index, Level, NewInstance, Statistics};

/// Record a series, each instance's storage key its SOP Instance UID.
async fn record(index: &Index, series: &SyntheticSeries) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = series.instance_uid(slice as u32);
        let new = NewInstance::new(&instance, key, bytes.len()).unwrap();
        index.record(&new).await.unwrap();
    }
}

/// Patient SYN71 with two studies: study 71 (series 71 of 2 slices, series 72 of 1) and
/// study 73 (series 73 of 1).
async fn patient(index: &Index) -> [SyntheticSeries; 3] {
    let a = SyntheticSeries::new(71, 2);
    let b = SyntheticSeries::new(72, 1).in_study(71, 2).for_patient(71);
    let c = SyntheticSeries::new(73, 1).for_patient(71);
    for s in [&a, &b, &c] {
        record(index, s).await;
    }
    [a, b, c]
}

fn ids(s: &SyntheticSeries) -> (String, String, String) {
    let p = s.patient_id();
    (
        public_ids::patient(&p),
        public_ids::study(&p, &s.study_uid()),
        public_ids::series(&p, &s.study_uid(), &s.series_uid()),
    )
}

fn instance_id(s: &SyntheticSeries, slice: u32) -> String {
    public_ids::instance(
        &s.patient_id(),
        &s.study_uid(),
        &s.series_uid(),
        &s.instance_uid(slice),
    )
}

#[tokio::test]
async fn a_patient_is_its_studies() {
    for index in common::indexes().await {
        let [a, _, c] = patient(&index).await;
        record(&index, &SyntheticSeries::new(74, 1)).await;
        let (patient, study_a, _) = ids(&a);
        let (_, study_c, _) = ids(&c);

        let all = index.patient_public_ids(None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&patient));
        assert_eq!(
            index.patient_public_ids(Some("SYN71")).await.unwrap(),
            std::slice::from_ref(&patient)
        );

        let p = index.rest_patient(&patient).await.unwrap().unwrap();
        assert_eq!(p.main_tags["PatientID"], "SYN71");
        let mut studies = p.studies.clone();
        studies.sort();
        let mut expected = vec![study_a, study_c];
        expected.sort();
        assert_eq!(studies, expected);
        assert!(!p.is_stable);

        assert_eq!(index.patient_series_ids(&patient).await.unwrap().len(), 3);
        assert_eq!(
            index.rest_patient_instances(&patient).await.unwrap().len(),
            4
        );
        let stats = index.patient_statistics(&patient).await.unwrap().unwrap();
        assert_eq!((stats.studies, stats.series, stats.instances), (2, 3, 4));
        assert!(index.rest_patient("nope").await.unwrap().is_none());
        assert_eq!(index.patient_statistics("nope").await.unwrap(), None);
    }
}

#[tokio::test]
async fn deleting_takes_the_ancestors_it_empties() {
    for index in common::indexes().await {
        let [a, b, c] = patient(&index).await;
        let (patient, study_a, series_a) = ids(&a);
        let (_, _, series_b) = ids(&b);
        let (_, study_c, _) = ids(&c);

        // one of two instances: the series remains
        let deleted = index
            .delete(Level::Instance, &instance_id(&a, 0))
            .await
            .unwrap();
        assert_eq!(
            deleted,
            Some(Deleted {
                object_keys: vec![a.instance_uid(0)],
                remaining_ancestor: Some((Level::Series, series_a.clone())),
            })
        );
        // the series' last instance: the study, with another series, remains
        let deleted = index
            .delete(Level::Instance, &instance_id(&a, 1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            deleted.remaining_ancestor,
            Some((Level::Study, study_a.clone()))
        );
        assert!(index.rest_series(&series_a).await.unwrap().is_none());
        // the study's last series: the patient, with another study, remains
        let deleted = index
            .delete(Level::Series, &series_b)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deleted.object_keys, [b.instance_uid(0)]);
        assert_eq!(
            deleted.remaining_ancestor,
            Some((Level::Patient, patient.clone()))
        );
        assert!(index.rest_study(&study_a).await.unwrap().is_none());
        let p = index.rest_patient(&patient).await.unwrap().unwrap();
        assert_eq!(p.studies, std::slice::from_ref(&study_c));
        // the patient's last study: nothing remains
        let deleted = index.delete(Level::Study, &study_c).await.unwrap().unwrap();
        assert_eq!(deleted.remaining_ancestor, None);
        assert!(index.rest_patient(&patient).await.unwrap().is_none());
        assert!(index.patient_public_ids(None).await.unwrap().is_empty());
        // gone is gone
        assert_eq!(index.delete(Level::Study, &study_c).await.unwrap(), None);
    }
}

#[tokio::test]
async fn deleting_a_patient_takes_everything_and_nothing_else() {
    for index in common::indexes().await {
        let [a, b, c] = patient(&index).await;
        let other = SyntheticSeries::new(75, 2);
        record(&index, &other).await;
        let (patient, _, _) = ids(&a);

        let deleted = index
            .delete(Level::Patient, &patient)
            .await
            .unwrap()
            .unwrap();
        let mut keys = deleted.object_keys;
        keys.sort();
        let mut expected = vec![
            a.instance_uid(0),
            a.instance_uid(1),
            b.instance_uid(0),
            c.instance_uid(0),
        ];
        expected.sort();
        assert_eq!(keys, expected);
        assert_eq!(deleted.remaining_ancestor, None);
        assert_eq!(index.delete(Level::Patient, &patient).await.unwrap(), None);
        assert!(
            index
                .rest_patient_instances(&patient)
                .await
                .unwrap()
                .is_empty()
        );

        let (other_patient, other_study, _) = ids(&other);
        assert_eq!(
            index.patient_public_ids(None).await.unwrap(),
            [other_patient]
        );
        assert_eq!(
            index.study_statistics(&other_study).await.unwrap(),
            Some(Statistics {
                studies: 1,
                series: 1,
                instances: 2,
                size_bytes: other.all_bytes().iter().map(|b| b.len() as i64).sum(),
            })
        );
        // the deleted instances can come back
        record(&index, &a).await;
        assert_eq!(
            index.rest_patient_instances(&patient).await.unwrap().len(),
            2
        );
    }
}
