//! Modifying a patient: every file rewritten, the index naming the new ones under the public
//! IDs the new Patient ID gives, labels and metadata following, the old files in the trash,
//! and a restore of that entry undoing it all.

use bytes::Bytes;
use dicom_dictionary_std::tags;
use pacs_archive::{Archive, ModifyError};
use pacs_core::Instance;
use pacs_core::patient::PatientChange;
use pacs_core::public_ids;
use pacs_core::real::RealSeries;
use pacs_index::{Index, Level};
use pacs_store::Store;

async fn archive(trash_days: u64) -> Archive {
    Archive::new(Store::memory(), Index::in_memory().await.unwrap()).with_trash(trash_days)
}

async fn store(archive: &Archive, series: &RealSeries) {
    for bytes in series.all_bytes() {
        archive.ingest(Bytes::from(bytes)).await.unwrap();
    }
}

/// The patient attribute `tag` of every stored file of `series`.
async fn stored(
    archive: &Archive,
    series: &RealSeries,
    slices: u32,
    tag: dicom_core::Tag,
) -> Vec<String> {
    let mut values = Vec::new();
    for i in 0..slices {
        let entry = archive
            .index()
            .instance(
                &series.study_uid(),
                &series.series_uid(),
                &series.instance_uid(i),
            )
            .await
            .unwrap()
            .expect("the instance is indexed");
        let file = archive.file(&entry).await.unwrap();
        values.push(
            Instance::from_bytes(&file)
                .unwrap()
                .string(tag)
                .unwrap_or_default(),
        );
    }
    values
}

fn corrected() -> PatientChange {
    PatientChange {
        name: Some("Benali^Amina".into()),
        id: Some("H-0042".into()),
        birth_date: Some("19800102".into()),
        sex: Some("F".into()),
    }
}

#[tokio::test]
async fn every_file_gets_the_new_values_and_the_ids_follow() {
    let archive = archive(30).await;
    let series = RealSeries::new(71, 3).for_patient(71);
    store(&archive, &series).await;
    let old = public_ids::patient(&series.patient_id());
    let study_old = public_ids::study(&series.patient_id(), &series.study_uid());
    archive
        .index()
        .add_label(Level::Patient, &old, "teaching")
        .await
        .unwrap();
    archive
        .index()
        .add_label(Level::Study, &study_old, "liver")
        .await
        .unwrap();

    let done = archive
        .modify_patient(&old, &corrected())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(done.instances, 3);
    assert_eq!(done.patient, public_ids::patient("H-0042"));
    assert_eq!(
        stored(&archive, &series, 3, tags::PATIENT_ID).await,
        ["H-0042"; 3]
    );
    assert_eq!(
        stored(&archive, &series, 3, tags::PATIENT_NAME).await,
        ["Benali^Amina"; 3]
    );
    assert_eq!(
        stored(&archive, &series, 3, tags::PATIENT_SEX).await,
        ["F"; 3]
    );
    // the old IDs are gone, the labels went with the resources
    assert!(
        archive
            .index()
            .audited_by_public_id(Level::Patient, &old)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        archive.index().labels(&done.patient).await.unwrap(),
        ["teaching"]
    );
    let study_new = public_ids::study("H-0042", &series.study_uid());
    assert_eq!(archive.index().labels(&study_new).await.unwrap(), ["liver"]);
    assert!(archive.index().labels(&old).await.unwrap().is_empty());
    // the old files wait in the trash
    let trash = archive
        .index()
        .trash_entry(done.trash.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(trash.level, "Modification");
    assert_eq!(trash.files, 3);
    assert_eq!(trash.studies[0]["PatientID"], series.patient_id());
    assert_eq!(trash.studies[0]["changed"]["PatientID"], "H-0042");
}

#[tokio::test]
async fn restoring_the_entry_undoes_the_modification() {
    let archive = archive(30).await;
    let series = RealSeries::new(72, 2).for_patient(72);
    store(&archive, &series).await;
    let old = public_ids::patient(&series.patient_id());
    archive
        .index()
        .add_label(Level::Patient, &old, "teaching")
        .await
        .unwrap();
    let done = archive
        .modify_patient(&old, &corrected())
        .await
        .unwrap()
        .unwrap();

    let restored = archive.restore(done.trash.unwrap()).await.unwrap().unwrap();

    assert_eq!(restored.restored, 2, "{:?}", restored.kept);
    assert_eq!(
        stored(&archive, &series, 2, tags::PATIENT_ID).await,
        [series.patient_id(), series.patient_id()]
    );
    assert_eq!(archive.index().labels(&old).await.unwrap(), ["teaching"]);
    assert!(
        archive
            .index()
            .audited_by_public_id(Level::Patient, &done.patient)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        archive
            .index()
            .trash_entry(done.trash.unwrap())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_patient_merged_into_another_takes_their_identity() {
    let archive = archive(30).await;
    let kept = RealSeries::new(73, 1)
        .for_patient(73)
        .with_patient("KEPT^ONE", None)
        .born("19700101");
    let merged = RealSeries::new(74, 2).for_patient(74);
    store(&archive, &kept).await;
    store(&archive, &merged).await;
    let into = public_ids::patient(&kept.patient_id());
    let from = public_ids::patient(&merged.patient_id());

    let done = archive.merge_patients(&from, &into).await.unwrap().unwrap();

    assert_eq!(done.patient, into);
    assert_eq!(
        stored(&archive, &merged, 2, tags::PATIENT_ID).await,
        [kept.patient_id(), kept.patient_id()]
    );
    assert_eq!(
        stored(&archive, &merged, 2, tags::PATIENT_NAME).await,
        ["KEPT^ONE"; 2]
    );
    assert_eq!(
        stored(&archive, &merged, 2, tags::PATIENT_BIRTH_DATE).await,
        ["19700101"; 2]
    );
    assert_eq!(
        archive
            .index()
            .audited_by_public_id(Level::Patient, &into)
            .await
            .unwrap()
            .len(),
        2
    );
    assert!(
        archive
            .index()
            .audited_by_public_id(Level::Patient, &from)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_merge_into_oneself_or_a_missing_patient_is_refused() {
    let archive = archive(30).await;
    let series = RealSeries::new(75, 1).for_patient(75);
    store(&archive, &series).await;
    let id = public_ids::patient(&series.patient_id());
    assert!(matches!(
        archive.merge_patients(&id, &id).await,
        Err(ModifyError::Invalid(_))
    ));
    assert!(
        archive
            .merge_patients(&id, "no-such-patient")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        archive
            .modify_patient("no-such-patient", &corrected())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn bad_values_change_nothing() {
    let archive = archive(30).await;
    let series = RealSeries::new(76, 1).for_patient(76);
    store(&archive, &series).await;
    let id = public_ids::patient(&series.patient_id());
    let error = archive
        .modify_patient(
            &id,
            &PatientChange {
                birth_date: Some("19991340".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, ModifyError::Invalid(_)), "{error}");
    assert_eq!(
        stored(&archive, &series, 1, tags::PATIENT_ID).await,
        [series.patient_id()]
    );
    assert!(archive.index().trash().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_project_copy_is_not_modified() {
    let archive = archive(30).await;
    let series = RealSeries::new(77, 1).for_patient(77);
    for bytes in series.all_bytes() {
        archive
            .clone()
            .for_project("liver")
            .ingest(Bytes::from(bytes))
            .await
            .unwrap();
    }
    let id = public_ids::patient(&series.patient_id());
    let error = archive.modify_patient(&id, &corrected()).await.unwrap_err();
    assert!(
        matches!(error, ModifyError::Project(_, ref p) if p == "liver"),
        "{error}"
    );
    assert_eq!(
        stored(&archive, &series, 1, tags::PATIENT_ID).await,
        [series.patient_id()]
    );
}

#[tokio::test]
async fn without_a_trash_the_old_files_are_gone_at_once() {
    let archive = archive(0).await;
    let series = RealSeries::new(78, 2).for_patient(78);
    store(&archive, &series).await;
    let id = public_ids::patient(&series.patient_id());
    let done = archive
        .modify_patient(&id, &corrected())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(done.trash, None);
    assert!(archive.index().trash().await.unwrap().is_empty());
    assert_eq!(
        stored(&archive, &series, 2, tags::PATIENT_ID).await,
        ["H-0042"; 2]
    );
}

#[tokio::test]
async fn undoing_a_merge_gives_each_patient_their_own_labels() {
    let archive = archive(30).await;
    let kept = RealSeries::new(75, 1).for_patient(75);
    let merged = RealSeries::new(76, 2).for_patient(76);
    store(&archive, &kept).await;
    store(&archive, &merged).await;
    let into = public_ids::patient(&kept.patient_id());
    let from = public_ids::patient(&merged.patient_id());
    let index = archive.index();
    index
        .add_label(Level::Patient, &into, "teaching")
        .await
        .unwrap();
    index
        .add_label(Level::Patient, &into, "both")
        .await
        .unwrap();
    index
        .add_label(Level::Patient, &from, "follow-up")
        .await
        .unwrap();
    index
        .add_label(Level::Patient, &from, "both")
        .await
        .unwrap();

    let done = archive.merge_patients(&from, &into).await.unwrap().unwrap();
    assert_eq!(
        index.labels(&into).await.unwrap(),
        ["both", "follow-up", "teaching"]
    );
    // the moves are in the entry from the transaction that made it
    let entry = index
        .trash_entry(done.trash.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        pacs_index::Moved::list_from_json(&entry.studies[0]["rekeyed"]).unwrap(),
        done.rekeyed
    );

    archive.restore(done.trash.unwrap()).await.unwrap().unwrap();

    // the patient merged into keeps theirs, "both" included; the other gets theirs back
    assert_eq!(index.labels(&into).await.unwrap(), ["both", "teaching"]);
    assert_eq!(index.labels(&from).await.unwrap(), ["both", "follow-up"]);
}

#[tokio::test]
async fn an_undo_that_cannot_be_whole_changes_nothing() {
    let archive = archive(30).await;
    let series = RealSeries::new(77, 3).for_patient(77);
    store(&archive, &series).await;
    let old = public_ids::patient(&series.patient_id());
    archive
        .index()
        .add_label(Level::Patient, &old, "teaching")
        .await
        .unwrap();
    let done = archive
        .modify_patient(&old, &corrected())
        .await
        .unwrap()
        .unwrap();
    // one modified instance deleted since: it can no longer be put back
    let gone = public_ids::instance(
        "H-0042",
        &series.study_uid(),
        &series.series_uid(),
        &series.instance_uid(1),
    );
    archive
        .delete(Level::Instance, &gone)
        .await
        .unwrap()
        .unwrap();

    let refused = archive.restore(done.trash.unwrap()).await;

    assert!(
        matches!(
            refused,
            Err(pacs_archive::CheckError::Index(
                pacs_index::Error::Conflict(_)
            ))
        ),
        "{refused:?}"
    );
    // every instance left is still the modified one, and the labels stayed with it
    for i in [0, 2] {
        let entry = archive
            .index()
            .instance(
                &series.study_uid(),
                &series.series_uid(),
                &series.instance_uid(i),
            )
            .await
            .unwrap()
            .unwrap();
        let file = archive.file(&entry).await.unwrap();
        assert_eq!(
            Instance::from_bytes(&file)
                .unwrap()
                .string(tags::PATIENT_ID)
                .unwrap(),
            "H-0042"
        );
    }
    assert_eq!(
        archive.index().labels(&done.patient).await.unwrap(),
        ["teaching"]
    );
    assert!(archive.index().labels(&old).await.unwrap().is_empty());
    let entry = archive
        .index()
        .trash_entry(done.trash.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.files, 3, "the entry keeps every old file");
}

#[tokio::test]
async fn the_entry_keeps_the_birth_date_and_sex_before() {
    let archive = archive(30).await;
    let series = RealSeries::new(78, 1).for_patient(78).born("19650303");
    store(&archive, &series).await;
    let old = public_ids::patient(&series.patient_id());
    let before = archive
        .index()
        .patient_main_tags(&old)
        .await
        .unwrap()
        .unwrap();

    let done = archive
        .modify_patient(&old, &corrected())
        .await
        .unwrap()
        .unwrap();

    let entry = archive
        .index()
        .trash_entry(done.trash.unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry.studies[0]["PatientBirthDate"], "19650303");
    assert_eq!(entry.studies[0]["PatientSex"], before["PatientSex"]);
    assert_eq!(entry.studies[0]["changed"]["PatientBirthDate"], "19800102");
}
