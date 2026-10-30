mod common;

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use pacs_core::Instance;
use pacs_core::public_ids;
use pacs_core::synthetic::SyntheticSeries;
use pacs_index::{Index, NewInstance, WorklistQuery, WorklistSort};

/// A one-slice study of its own patient: this date, description, sex and modality.
async fn study(index: &Index, seed: u32, date: &str, description: &str, sex: &str, modality: &str) {
    let series = SyntheticSeries::new(seed, 1);
    let bytes = series.slice_bytes(0);
    let mut instance = Instance::from_bytes(&bytes).unwrap();
    for (tag, vr, value) in [
        (Tag(0x0008, 0x0020), VR::DA, date),
        (Tag(0x0008, 0x1030), VR::LO, description),
        (Tag(0x0010, 0x0040), VR::CS, sex),
        (Tag(0x0008, 0x0060), VR::CS, modality),
    ] {
        instance
            .object
            .put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
    }
    let new = NewInstance::new(&instance, series.instance_uid(0), bytes.len()).unwrap();
    index.record(&new).await.unwrap();
}

fn descriptions(page: &[pacs_index::WorklistStudy]) -> Vec<String> {
    page.iter()
        .map(|s| s.main_tags["StudyDescription"].as_str().unwrap().to_owned())
        .collect()
}

fn all() -> WorklistQuery {
    WorklistQuery {
        limit: 100,
        descending: true,
        ..WorklistQuery::default()
    }
}

#[tokio::test]
async fn the_list_is_filtered_sorted_and_paged_in_the_index() {
    for index in common::indexes().await {
        study(&index, 91, "20260110", "CT CHEST", "F", "CT").await;
        study(&index, 92, "20260220", "MR LIVER", "M", "MR").await;
        study(&index, 93, "20260305", "CT ABDOMEN 100%", "", "CT").await;
        study(&index, 94, "20250701", "XR HAND", "F", "CR").await;

        // newest first, then a page of two, then the next
        let (page, total) = index.worklist(&all()).await.unwrap();
        assert_eq!(total, 4);
        assert_eq!(
            descriptions(&page),
            ["CT ABDOMEN 100%", "MR LIVER", "CT CHEST", "XR HAND"]
        );
        let (page, total) = index
            .worklist(&WorklistQuery {
                limit: 2,
                offset: 2,
                ..all()
            })
            .await
            .unwrap();
        assert_eq!(
            (descriptions(&page), total),
            (vec!["CT CHEST".into(), "XR HAND".into()], 4)
        );
        assert_eq!(page[0].modalities, ["CT"]);
        assert_eq!((page[0].series, page[0].instances), (1, 1));

        // a date range, both ends included
        let q = WorklistQuery {
            from: Some("20260110".into()),
            to: Some("20260220".into()),
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["MR LIVER", "CT CHEST"]
        );
        // modalities, sexes ("O": neither F nor M)
        let q = WorklistQuery {
            modalities: vec!["CT".into(), "CR".into()],
            ..all()
        };
        assert_eq!(index.worklist(&q).await.unwrap().1, 3);
        let q = WorklistQuery {
            sexes: vec!["O".into()],
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["CT ABDOMEN 100%"]
        );
        // words, in any case, in any field; % is itself
        let q = WorklistQuery {
            words: vec!["ct".into(), "abd".into()],
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["CT ABDOMEN 100%"]
        );
        let q = WorklistQuery {
            words: vec!["0%".into()],
            ..all()
        };
        assert_eq!(index.worklist(&q).await.unwrap().1, 1);
        let q = WorklistQuery {
            words: vec!["syn92".into()],
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["MR LIVER"]
        );
        // one patient
        let patient = public_ids::patient("SYN94");
        let q = WorklistQuery {
            patient: Some(patient.clone()),
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["XR HAND"]
        );
        // sorted by description, A to Z
        let q = WorklistQuery {
            sort: WorklistSort::Description,
            descending: false,
            ..all()
        };
        assert_eq!(
            descriptions(&index.worklist(&q).await.unwrap().0),
            ["CT ABDOMEN 100%", "CT CHEST", "MR LIVER", "XR HAND"]
        );

        // the totals of a filtered list
        let q = WorklistQuery {
            from: Some("20260101".into()),
            ..all()
        };
        let summary = index.worklist_summary(&q).await.unwrap();
        assert_eq!(
            (
                summary.patients,
                summary.studies,
                summary.series,
                summary.instances
            ),
            (3, 3, 3, 3)
        );
        assert_eq!(
            summary.by_modality,
            [("CT".to_owned(), 2), ("MR".to_owned(), 1)]
        );

        // the filters' choices
        let facets = index.worklist_facets(None).await.unwrap();
        assert_eq!(
            facets.modalities,
            [
                ("CR".to_owned(), 1),
                ("CT".to_owned(), 2),
                ("MR".to_owned(), 1)
            ]
        );
        assert_eq!(
            facets.sexes,
            [
                ("F".to_owned(), 2),
                ("M".to_owned(), 1),
                ("O".to_owned(), 1)
            ]
        );
        assert_eq!(facets.dates.len(), 4);
        assert_eq!(
            index.worklist_facets(Some(&patient)).await.unwrap().dates,
            [("20250701".to_owned(), 1)]
        );
    }
}
