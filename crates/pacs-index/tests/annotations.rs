mod common;

use pacs_core::Instance;
use pacs_core::public_ids;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Level, NewInstance};

#[tokio::test]
async fn measurements_are_kept_per_study_and_go_with_it() {
    for index in common::indexes().await {
        let series = SyntheticSeries::new(81, 1);
        let bytes = series.slice_bytes(0);
        let instance = Instance::from_bytes(&bytes).unwrap();
        let new = NewInstance::new(&instance, series.instance_uid(0), bytes.len()).unwrap();
        index.record(&new).await.unwrap();
        let study = series.study_uid();

        assert_eq!(index.annotations(&study).await.unwrap(), None);
        assert!(
            index
                .put_annotations(&study, Some(r#"[{"a":1}]"#))
                .await
                .unwrap()
        );
        assert!(
            index
                .put_annotations(&study, Some(r#"[{"a":2}]"#))
                .await
                .unwrap()
        );
        assert_eq!(
            index.annotations(&study).await.unwrap().as_deref(),
            Some(r#"[{"a":2}]"#)
        );
        // none: removed
        assert!(index.put_annotations(&study, None).await.unwrap());
        assert_eq!(index.annotations(&study).await.unwrap(), None);
        // a study the index doesn't have: refused, nothing kept
        assert!(!index.put_annotations("1.2.3.4", Some("[]")).await.unwrap());
        assert_eq!(index.annotations("1.2.3.4").await.unwrap(), None);

        // deleting the study deletes its measurements
        index
            .put_annotations(&study, Some(r#"[{"a":3}]"#))
            .await
            .unwrap();
        let id = public_ids::study(&series.patient_id(), &study);
        index.delete(Level::Study, &id).await.unwrap().unwrap();
        assert_eq!(index.annotations(&study).await.unwrap(), None);
    }
}
