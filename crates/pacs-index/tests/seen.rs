//! Rights per project: a limited index value reads only its account's projects.

mod common;

use pacs_core::Instance;
use pacs_core::public_ids;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Index, Level, NewInstance, PatientQuery, StudyFilter, WorklistQuery};

const READER: &str = "user:reader";

async fn record(index: &Index, series: &SyntheticSeries, project: Option<&str>) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = format!("dicom/{}/{slice}.dcm", series.seed);
        let new = NewInstance::new(&instance, key, bytes.len())
            .unwrap()
            .in_project(project);
        index.record(&new).await.unwrap();
    }
}

/// One patient with a study in `liver` and one in `lung`, and another patient whose study
/// is in no project.
async fn three_studies(index: &Index) -> [SyntheticSeries; 3] {
    let liver = SyntheticSeries::new(81, 2).for_patient(80);
    let lung = SyntheticSeries::new(82, 2).for_patient(80);
    let received = SyntheticSeries::new(83, 2);
    record(index, &liver, Some("liver")).await;
    record(index, &lung, Some("lung")).await;
    record(index, &received, None).await;
    [liver, lung, received]
}

fn all_studies() -> WorklistQuery {
    WorklistQuery {
        limit: 100,
        ..WorklistQuery::default()
    }
}

fn all_patients() -> PatientQuery {
    PatientQuery {
        limit: 100,
        ..PatientQuery::default()
    }
}

fn projects(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

struct Ids {
    patient: String,
    study: String,
    series: String,
    instance: String,
}

fn ids(s: &SyntheticSeries) -> Ids {
    let (p, st, se) = (s.patient_id(), s.study_uid(), s.series_uid());
    Ids {
        patient: public_ids::patient(&p),
        study: public_ids::study(&p, &st),
        series: public_ids::series(&p, &st, &se),
        instance: public_ids::instance(&p, &st, &se, &s.instance_uid(0)),
    }
}

#[tokio::test]
async fn an_account_is_unlimited_until_it_is_given_a_list() {
    for index in common::indexes().await {
        assert_eq!(index.account_projects(READER).await.unwrap(), None);
        let projects = ["lung".to_owned(), "liver".to_owned(), "liver".to_owned()];
        index
            .set_account_projects(READER, Some(&projects))
            .await
            .unwrap();
        assert_eq!(
            index.account_projects(READER).await.unwrap(),
            Some(vec!["liver".to_owned(), "lung".to_owned()])
        );
        assert_eq!(index.account_projects("user:other").await.unwrap(), None);
        // an empty list is a limit: no project at all
        index.set_account_projects(READER, Some(&[])).await.unwrap();
        assert_eq!(index.account_projects(READER).await.unwrap(), Some(vec![]));
        index.set_account_projects(READER, None).await.unwrap();
        assert_eq!(index.account_projects(READER).await.unwrap(), None);
    }
}

/// Every read of the hierarchy, as the account sees it: `seen` are the series it must
/// find, `hidden` those it must not.
async fn check(index: &Index, seen: &[&SyntheticSeries], hidden: &[&SyntheticSeries]) {
    let studies = index.studies(&StudyFilter::default()).await.unwrap();
    assert_eq!(studies.len(), seen.len());
    assert_eq!(index.study_public_ids().await.unwrap().len(), seen.len());
    let (listed, total) = index.worklist(&all_studies()).await.unwrap();
    assert_eq!((listed.len(), total), (seen.len(), seen.len() as i64));
    let summary = index
        .worklist_summary(&WorklistQuery::default())
        .await
        .unwrap();
    assert_eq!(summary.studies, seen.len() as i64);
    assert_eq!(summary.instances, 2 * seen.len() as i64);
    let facets = index.worklist_facets(None).await.unwrap();
    let dated: i64 = facets.dates.iter().map(|(_, n)| n).sum();
    assert_eq!(dated, seen.len() as i64);

    for s in seen {
        let (id, uid) = (ids(s), s.study_uid());
        assert!(index.study(&uid).await.unwrap().is_some());
        assert_eq!(index.series(&uid, None).await.unwrap().len(), 1);
        assert_eq!(
            index
                .instances(&uid, &s.series_uid(), None)
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            index
                .metadata(&uid, &s.series_uid(), None)
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(index.study_project(&uid).await.unwrap().is_some());
        assert!(index.rest_study(&id.study).await.unwrap().is_some());
        assert!(index.rest_series(&id.series).await.unwrap().is_some());
        assert!(index.rest_instance(&id.instance).await.unwrap().is_some());
        assert!(
            index
                .rest_instance_by_uid(&s.instance_uid(0))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            index
                .series_public_id(&s.series_uid())
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            index.rest_study_instances(&id.study).await.unwrap().len(),
            2
        );
        assert!(index.study_statistics(&id.study).await.unwrap().is_some());
        assert!(index.audited_study(&uid).await.unwrap().is_some());
        for (level, id) in [
            (Level::Study, &id.study),
            (Level::Series, &id.series),
            (Level::Instance, &id.instance),
        ] {
            assert_eq!(
                index.audited_by_public_id(level, id).await.unwrap().len(),
                1
            );
        }
        assert!(index.put_annotations(&uid, Some("[]")).await.unwrap());
        assert_eq!(
            index.annotations(&uid).await.unwrap().as_deref(),
            Some("[]")
        );
    }
    for s in hidden {
        let (id, uid) = (ids(s), s.study_uid());
        assert!(index.study(&uid).await.unwrap().is_none());
        assert!(index.series(&uid, None).await.unwrap().is_empty());
        assert!(
            index
                .instances(&uid, &s.series_uid(), None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            index
                .metadata(&uid, &s.series_uid(), None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            index
                .metadata_text(&uid, &s.series_uid(), None)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(index.study_project(&uid).await.unwrap(), None);
        assert!(index.rest_study(&id.study).await.unwrap().is_none());
        assert!(index.rest_series(&id.series).await.unwrap().is_none());
        assert!(index.rest_instance(&id.instance).await.unwrap().is_none());
        assert!(
            index
                .rest_instance_by_uid(&s.instance_uid(0))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            index
                .series_public_id(&s.series_uid())
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            index
                .rest_study_instances(&id.study)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(index.study_statistics(&id.study).await.unwrap().is_none());
        assert!(index.audited_study(&uid).await.unwrap().is_none());
        for (level, id) in [
            (Level::Study, &id.study),
            (Level::Series, &id.series),
            (Level::Instance, &id.instance),
        ] {
            assert!(
                index
                    .audited_by_public_id(level, id)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        // measurements of a study it does not see: not read, not written, as for no study
        assert!(index.annotations(&uid).await.unwrap().is_none());
        assert!(!index.put_annotations(&uid, Some("[1]")).await.unwrap());
        let filter = StudyFilter {
            study_instance_uid: Some(uid),
            ..StudyFilter::default()
        };
        assert!(index.studies(&filter).await.unwrap().is_empty());
        let by_patient = WorklistQuery {
            patient: Some(id.patient),
            ..all_studies()
        };
        let (listed, _) = index.worklist(&by_patient).await.unwrap();
        assert!(listed.iter().all(|w| w.study_instance_uid != s.study_uid()));
    }
}

#[tokio::test]
async fn a_limited_index_reads_only_its_projects() {
    for index in common::indexes().await {
        let [liver, lung, received] = three_studies(&index).await;
        // measurements on every study, written by someone who sees them all
        for s in [&liver, &lung, &received] {
            assert!(
                index
                    .put_annotations(&s.study_uid(), Some("[0]"))
                    .await
                    .unwrap()
            );
        }
        check(&index, &[&liver, &lung, &received], &[]).await;

        // not limited: everything, under any name
        check(
            &index.limited_to("user:nobody-limited-me"),
            &[],
            &[&liver, &lung, &received],
        )
        .await;
        let reader = index.limited_to(READER);
        assert_eq!(reader.account(), Some(READER));
        assert_eq!(reader.unlimited().account(), None);

        index
            .set_account_projects(READER, Some(&projects(&["liver"])))
            .await
            .unwrap();
        check(&reader, &[&liver], &[&lung, &received]).await;
        // the whole index is as it was, but for the measurements the reader wrote
        check(&index, &[&liver, &lung, &received], &[]).await;

        index
            .set_account_projects(READER, Some(&projects(&["liver", "lung"])))
            .await
            .unwrap();
        check(&reader, &[&liver, &lung], &[&received]).await;

        index.set_account_projects(READER, Some(&[])).await.unwrap();
        check(&reader, &[], &[&liver, &lung, &received]).await;
    }
}

#[tokio::test]
async fn a_patient_is_the_studies_the_account_sees() {
    for index in common::indexes().await {
        let [liver, lung, received] = three_studies(&index).await;
        index
            .set_account_projects(READER, Some(&projects(&["liver"])))
            .await
            .unwrap();
        let reader = index.limited_to(READER);
        let (shared, other) = (ids(&liver).patient, ids(&received).patient);
        assert_eq!(shared, ids(&lung).patient);

        assert_eq!(
            reader.patient_public_ids(None).await.unwrap(),
            [shared.clone()]
        );
        assert!(
            reader
                .patient_public_ids(Some("SYN83"))
                .await
                .unwrap()
                .is_empty()
        );
        let patient = reader.rest_patient(&shared).await.unwrap().unwrap();
        assert_eq!(patient.studies, [ids(&liver).study]);
        assert_eq!(
            reader.patient_series_ids(&shared).await.unwrap(),
            [ids(&liver).series]
        );
        assert_eq!(
            reader.rest_patient_instances(&shared).await.unwrap().len(),
            2
        );
        let stats = reader.patient_statistics(&shared).await.unwrap().unwrap();
        assert_eq!((stats.studies, stats.instances), (1, 2));
        assert_eq!(
            reader
                .audited_by_public_id(Level::Patient, &shared)
                .await
                .unwrap()
                .len(),
            1
        );
        let (patients, total) = reader.worklist_patients(&all_patients()).await.unwrap();
        assert_eq!((patients.len(), total), (1, 1));
        assert_eq!((patients[0].studies, patients[0].instances), (1, 2));

        // the other patient is no patient to this account
        assert!(reader.rest_patient(&other).await.unwrap().is_none());
        assert!(reader.patient_series_ids(&other).await.unwrap().is_empty());
        assert!(
            reader
                .rest_patient_instances(&other)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(reader.patient_statistics(&other).await.unwrap().is_none());
        assert!(
            reader
                .audited_by_public_id(Level::Patient, &other)
                .await
                .unwrap()
                .is_empty()
        );

        // and to the whole index the first patient has both studies
        let whole = index.rest_patient(&shared).await.unwrap().unwrap();
        assert_eq!(whole.studies.len(), 2);
        let (patients, _) = index.worklist_patients(&all_patients()).await.unwrap();
        assert_eq!(patients.len(), 2);
    }
}

#[tokio::test]
async fn a_limited_index_deletes_only_what_it_sees() {
    for index in common::indexes().await {
        let [liver, lung, received] = three_studies(&index).await;
        index
            .set_account_projects(READER, Some(&projects(&["liver"])))
            .await
            .unwrap();
        let reader = index.limited_to(READER);

        // what it does not see is not there to delete, at any level
        for hidden in [&lung, &received] {
            let id = ids(hidden);
            for (level, id) in [
                (Level::Instance, &id.instance),
                (Level::Series, &id.series),
                (Level::Study, &id.study),
            ] {
                assert!(reader.delete(level, id).await.unwrap().is_none());
            }
        }
        assert!(
            reader
                .delete(Level::Patient, &ids(&received).patient)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(index.study_public_ids().await.unwrap().len(), 3);

        // an instance of its own: the series is left, and named
        let deleted = reader
            .delete(Level::Instance, &ids(&liver).instance)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deleted.object_keys.len(), 1);
        assert_eq!(
            deleted.remaining_ancestor,
            Some((Level::Series, ids(&liver).series))
        );

        // the patient: its liver study goes, the lung study stays, and the account is told
        // of no patient left, as it sees none
        let deleted = reader
            .delete(Level::Patient, &ids(&liver).patient)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deleted.object_keys.len(), 1);
        assert_eq!(deleted.remaining_ancestor, None);
        assert!(index.study(&liver.study_uid()).await.unwrap().is_none());
        assert!(index.study(&lung.study_uid()).await.unwrap().is_some());
        assert!(index.study(&received.study_uid()).await.unwrap().is_some());

        // a study whose patient has another study the account does not see: no ancestor named
        record(&index, &liver, Some("liver")).await;
        let deleted = reader
            .delete(Level::Study, &ids(&liver).study)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deleted.remaining_ancestor, None);
        // while the whole index names the patient that is left
        record(&index, &liver, Some("liver")).await;
        let deleted = index
            .delete(Level::Study, &ids(&liver).study)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            deleted.remaining_ancestor,
            Some((Level::Patient, ids(&lung).patient))
        );
    }
}
