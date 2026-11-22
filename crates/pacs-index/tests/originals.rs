//! The way back from a project's copy to the original, for write-back.

mod common;

use pacs_index::OriginalStudy;
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
            "INSERT INTO project_studies (project, replaced_study, patient_main_tags,
                                          study_main_tags) VALUES ('liver', '2.25.1', 'not json', '{}')",
        )
        .execute(index.pool())
        .await
        .unwrap();
        assert!(index.original_study("liver", "2.25.1").await.is_err());
    }
}
