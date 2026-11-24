//! The way back from a project's copy to the original, for write-back.

mod common;

use pacs_core::Instance;
use pacs_core::real::RealSeries;
use pacs_index::{NewInstance, OriginalStudy};
use serde_json::{Map, Value, json};

fn tags(value: Value) -> Map<String, Value> {
    value.as_object().unwrap().clone()
}

#[tokio::test]
async fn originals_are_kept_per_project_and_the_first_stays() {
    for index in common::indexes().await {
        let original = OriginalStudy {
            patient: tags(json!({"PatientID": "P123", "PatientName": "BENALI^AMINE"})),
            study: tags(json!({"AccessionNumber": "ACC9", "StudyDate": "20240301"})),
        };
        let pairs = vec![
            ("2.25.1".to_owned(), "1.2.3".to_owned()),
            ("2.25.2".to_owned(), "1.2.3.1".to_owned()),
        ];
        index
            .record_originals("liver", &pairs, "2.25.1", &original)
            .await
            .unwrap();
        // a later instance of the same study records again: nothing changes
        let other = OriginalStudy {
            patient: tags(json!({"PatientID": "OTHER"})),
            study: Map::new(),
        };
        index
            .record_originals(
                "liver",
                &[("2.25.1".to_owned(), "9.9.9".to_owned())],
                "2.25.1",
                &other,
            )
            .await
            .unwrap();

        let asked = ["2.25.1", "2.25.2", "2.25.3"].map(str::to_owned);
        let found = index.original_uids("liver", &asked).await.unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found["2.25.1"], "1.2.3");
        assert_eq!(found["2.25.2"], "1.2.3.1");
        assert_eq!(
            index.original_study("liver", "2.25.1").await.unwrap(),
            Some(original)
        );
        // another project has nothing of it
        assert!(
            index
                .original_uids("lung", &asked)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(index.original_study("lung", "2.25.1").await.unwrap(), None);
    }
}

/// A row that is not what the index writes is an error, not an empty original.
#[tokio::test]
async fn a_damaged_original_is_an_error() {
    for index in common::indexes().await {
        sqlx::query(
            "INSERT INTO project_uids (project, replaced, original)
             VALUES ('liver', '2.25.1', '1.2.3')",
        )
        .execute(index.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO project_studies (project, replaced_study, patient_main_tags,
                                          study_main_tags) VALUES ('liver', '2.25.1', 'not json', '{}')",
        )
        .execute(index.pool())
        .await
        .unwrap();
        assert!(index.original_study("liver", "2.25.1").await.is_err());
    }
}

async fn record(index: &pacs_index::Index, series: &RealSeries, result: bool) {
    let bytes = series.header_bytes(0);
    let instance = Instance::from_bytes(&bytes).unwrap();
    let new = NewInstance::new(&instance, format!("dicom/{}.dcm", series.seed), bytes.len())
        .unwrap()
        .in_project(Some("liver"))
        .a_result(result);
    index.record(&new).await.unwrap();
}

/// A result is noted for write-back with its instance, taken out once by one claimer; the
/// results of a study are its instances the project did not make.
#[tokio::test]
async fn a_result_is_due_for_write_back_and_told_apart_from_the_copy() {
    for index in common::indexes().await {
        let copy = RealSeries::new(71, 1);
        let result = RealSeries::new(72, 1).in_study(71, 2);
        record(&index, &copy, false).await;
        // the copy's instance is one the project made by replacing an original
        index
            .record_originals(
                "liver",
                &[(copy.instance_uid(0), "1.2.3.9".to_owned())],
                &copy.study_uid(),
                &OriginalStudy {
                    patient: Map::new(),
                    study: Map::new(),
                },
            )
            .await
            .unwrap();
        assert!(index.claim_write_backs(i64::MAX).await.unwrap().is_empty());
        record(&index, &result, true).await;
        // not quiet yet
        assert!(index.claim_write_backs(0).await.unwrap().is_empty());
        assert_eq!(
            index.claim_write_backs(i64::MAX).await.unwrap(),
            [copy.study_uid()]
        );
        assert!(index.claim_write_backs(i64::MAX).await.unwrap().is_empty());
        let results = index.results_of("liver", &copy.study_uid()).await.unwrap();
        let uids: Vec<_> = results.iter().map(|r| r.sop_instance_uid.clone()).collect();
        assert_eq!(uids, [result.instance_uid(0)]);
    }
}

/// A study whose UID the project kept is no copy that takes results: not as 0.16.0 recorded
/// it (no UID pair), nor with the UID paired with itself.
#[tokio::test]
async fn a_study_whose_uid_was_kept_has_no_way_back() {
    for index in common::indexes().await {
        let original = OriginalStudy {
            patient: tags(json!({"PatientID": "P123"})),
            study: Map::new(),
        };
        for pairs in [
            vec![],
            vec![("2.25.7".to_owned(), "1.2.3.7".to_owned())],
            vec![("1.2.3".to_owned(), "1.2.3".to_owned())],
        ] {
            index
                .record_originals("liver", &pairs, "1.2.3", &original)
                .await
                .unwrap();
            assert_eq!(index.original_study("liver", "1.2.3").await.unwrap(), None);
        }
    }
}
