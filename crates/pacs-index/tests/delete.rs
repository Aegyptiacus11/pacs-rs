mod common;

use pacs_core::real::RealSeries;
use pacs_core::{Instance, public_ids};
use pacs_index::{Deleted, Index, IntoTrash, Level, NewInstance, Statistics};

/// Record a series, each instance's storage key its SOP Instance UID.
async fn record(index: &Index, series: &RealSeries) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = series.instance_uid(slice as u32);
        let new = NewInstance::new(&instance, key, bytes.len()).unwrap();
        index.record(&new).await.unwrap();
    }
}

/// Patient NEMA-71 with two studies: study 71 (series 71 of 2 slices, series 72 of 1) and
/// study 73 (series 73 of 1).
async fn patient(index: &Index) -> [RealSeries; 3] {
    let a = RealSeries::new(71, 2);
    let b = RealSeries::new(72, 1).in_study(71, 2).for_patient(71);
    let c = RealSeries::new(73, 1).for_patient(71);
    for s in [&a, &b, &c] {
        record(index, s).await;
    }
    [a, b, c]
}

fn ids(s: &RealSeries) -> (String, String, String) {
    let p = s.patient_id();
    (
        public_ids::patient(&p),
        public_ids::study(&p, &s.study_uid()),
        public_ids::series(&p, &s.study_uid(), &s.series_uid()),
    )
}

fn instance_id(s: &RealSeries, slice: u32) -> String {
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
        record(&index, &RealSeries::new(74, 1)).await;
        let (patient, study_a, _) = ids(&a);
        let (_, study_c, _) = ids(&c);

        let all = index.patient_public_ids(None).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.contains(&patient));
        assert_eq!(
            index.patient_public_ids(Some("NEMA-71")).await.unwrap(),
            std::slice::from_ref(&patient)
        );

        let p = index.rest_patient(&patient).await.unwrap().unwrap();
        assert_eq!(p.main_tags["PatientID"], "NEMA-71");
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
        let other = RealSeries::new(75, 2);
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

/// A deletion takes the labels and metadata of what it removed, at every level below it,
/// and leaves those of what is still there.
#[tokio::test]
async fn a_deletion_takes_the_labels_and_metadata_of_what_it_removed() {
    for index in common::indexes().await {
        let [a, b, c] = patient(&index).await;
        let (patient_id, study, series) = ids(&a);
        let (_, kept_study, _) = ids(&c);
        let resources = [
            (Level::Patient, patient_id.clone()),
            (Level::Study, study.clone()),
            (Level::Series, series.clone()),
            (Level::Series, ids(&b).2),
            (Level::Instance, instance_id(&a, 0)),
            (Level::Study, kept_study.clone()),
        ];
        for (level, id) in &resources {
            index.add_label(*level, id, "cohort").await.unwrap();
            index.set_metadata(*level, id, "note", "x").await.unwrap();
        }
        let count = async |table: &str| -> i64 {
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(index.pool())
                .await
                .unwrap()
        };
        assert_eq!((count("labels").await, count("metadata").await), (6, 6));
        index.delete(Level::Study, &study).await.unwrap().unwrap();
        // the study, its two series and the instance went; the patient and the other study stay
        assert_eq!((count("labels").await, count("metadata").await), (2, 2));
        assert_eq!(index.labels(&kept_study).await.unwrap(), ["cohort"]);
        assert_eq!(index.labels(&patient_id).await.unwrap(), ["cohort"]);
        index
            .delete(Level::Patient, &patient_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!((count("labels").await, count("metadata").await), (0, 0));
    }
}

fn into_trash<'a>(
    public_id: &'a str,
    studies: &'a serde_json::Value,
    trash_key: &'a (dyn Fn(i64, &str) -> String + Send + Sync),
) -> IntoTrash<'a> {
    IntoTrash {
        level: "Study",
        public_id,
        studies,
        trash_key,
        id: None,
    }
}

#[tokio::test]
async fn a_deletion_into_the_trash_lists_its_files_at_once() {
    for index in common::indexes().await {
        let [a, b, _] = patient(&index).await;
        let (_, study, _) = ids(&a);
        let studies = serde_json::json!([{ "StudyInstanceUID": a.study_uid() }]);
        let key = |id: i64, object_key: &str| format!("trash/{id}/{object_key}");
        let mut trash = into_trash(&study, &studies, &key);
        let deleted = index
            .delete_into_trash(Level::Study, &study, &mut trash)
            .await
            .unwrap()
            .unwrap();
        // with nothing else done, as when the server stops here: the entry and every file
        let id = trash.id.expect("an entry");
        let entry = index.trash_entry(id).await.unwrap().unwrap();
        assert_eq!((entry.level.as_str(), entry.files), ("Study", 3));
        assert_eq!(entry.studies, studies);
        let mut listed: Vec<String> = index
            .trash_files(id)
            .await
            .unwrap()
            .into_iter()
            .map(|file| {
                assert_eq!(file.trash_key, format!("trash/{id}/{}", file.object_key));
                file.object_key
            })
            .collect();
        listed.sort();
        let mut expected = vec![a.instance_uid(0), a.instance_uid(1), b.instance_uid(0)];
        expected.sort();
        assert_eq!(listed, expected);
        let mut taken = deleted.object_keys.clone();
        taken.sort();
        assert_eq!(taken, expected);
        let mut waiting = index.trash_object_keys().await.unwrap();
        waiting.sort();
        assert_eq!(waiting, expected);
    }
}

#[tokio::test]
async fn deleting_nothing_puts_nothing_in_the_trash() {
    for index in common::indexes().await {
        patient(&index).await;
        let studies = serde_json::json!([]);
        let key = |id: i64, object_key: &str| format!("trash/{id}/{object_key}");
        let mut trash = into_trash("no-such-study", &studies, &key);
        let deleted = index
            .delete_into_trash(Level::Study, "no-such-study", &mut trash)
            .await
            .unwrap();
        assert_eq!(deleted, None);
        assert_eq!(trash.id, None);
        assert!(index.trash().await.unwrap().is_empty());
        assert!(index.trash_object_keys().await.unwrap().is_empty());
    }
}
