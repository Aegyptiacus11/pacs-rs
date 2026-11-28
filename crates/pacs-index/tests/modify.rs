//! A patient's instances replaced by rewritten ones: the change must cover every instance the
//! patient has when it commits, so one stored meanwhile (by another server sharing the index)
//! is not left under the old identity.

mod common;

use pacs_core::Instance;
use pacs_core::patient::PatientChange;
use pacs_core::public_ids;
use pacs_core::real::RealSeries;
use pacs_index::{Error, Index, IntoTrash, NewInstance};

/// Record a series, each instance's storage key its SOP Instance UID.
async fn record(index: &Index, series: &RealSeries) {
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        let key = series.instance_uid(slice as u32);
        let new = NewInstance::new(&instance, key, bytes.len()).unwrap();
        index.record(&new).await.unwrap();
    }
}

/// The first `slices` instances of `series` rewritten with Patient ID `H-0042`.
fn rewritten(series: &RealSeries, slices: usize) -> Vec<NewInstance> {
    let change = PatientChange {
        id: Some("H-0042".into()),
        ..Default::default()
    };
    series
        .all_bytes()
        .iter()
        .take(slices)
        .enumerate()
        .map(|(slice, bytes)| {
            let bytes = Instance::from_bytes(bytes)
                .unwrap()
                .with_patient(&change)
                .unwrap();
            let instance = Instance::from_bytes(&bytes).unwrap();
            NewInstance::new(&instance, format!("new/{slice}"), bytes.len()).unwrap()
        })
        .collect()
}

async fn replace(index: &Index, patient: &str, news: &[NewInstance]) -> Result<(), Error> {
    let studies = serde_json::json!([{ "StudyInstanceUID": "1.2.3" }]);
    let trash_key = |id: i64, key: &str| format!("trash/{id}/{key}");
    let mut trash = IntoTrash {
        level: "Modification",
        public_id: patient,
        studies: &studies,
        trash_key: &trash_key,
        id: None,
    };
    index
        .replace_instances(patient, news, &mut trash)
        .await
        .map(|_| ())
}

#[tokio::test]
async fn every_instance_of_the_patient_is_replaced() {
    for index in common::indexes().await {
        let series = RealSeries::new(81, 3).for_patient(81);
        record(&index, &series).await;
        let old = public_ids::patient(&series.patient_id());

        replace(&index, &old, &rewritten(&series, 3)).await.unwrap();

        assert!(index.rest_patient(&old).await.unwrap().is_none());
        let new = public_ids::patient("H-0042");
        assert_eq!(index.rest_patient_instances(&new).await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn an_instance_stored_meanwhile_refuses_the_change() {
    for index in common::indexes().await {
        let series = RealSeries::new(82, 3).for_patient(82);
        record(&index, &series).await;
        let old = public_ids::patient(&series.patient_id());
        // the files were read when the patient had two instances; a third arrived since
        let news = rewritten(&series, 2);

        let refused = replace(&index, &old, &news).await;

        match refused {
            Err(Error::Conflict(why)) => assert!(
                why.contains(&series.instance_uid(2)),
                "names the instance that arrived: {why}"
            ),
            other => panic!("expected a conflict, got {other:?}"),
        }
        // nothing changed: every instance still under the old patient, no trash entry
        assert_eq!(index.rest_patient_instances(&old).await.unwrap().len(), 3);
        assert!(
            index
                .rest_patient(&public_ids::patient("H-0042"))
                .await
                .unwrap()
                .is_none()
        );
        assert!(index.trash().await.unwrap().is_empty());
    }
}
