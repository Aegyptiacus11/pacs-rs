//! Instances sent to a project's AE title: de-identified by its profile on the way in
//! (PS3.15 Annex E, through pacs-deid), only the result archived; quarantined ones kept apart
//! or refused.

mod common;

use std::sync::Arc;

use common::*;
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use pacs_core::synthetic::SyntheticSeries;
use pacs_deid::{Keys, Profile};
use pacs_dimse::{AeTitle, Destination, Project, Scp, Status};
use pacs_store::Store;

const SC: &str = "1.2.840.10008.5.1.4.1.1.7";

fn keys() -> Keys {
    Keys::new(*b"a project secret of 32 bytes ok!").unwrap()
}

/// pacs-rs with a project on LIVER_2026; quarantined instances kept in `quarantine` if any.
async fn with_project(quarantine: Option<Store>) -> (std::net::SocketAddr, pacs_archive::Archive) {
    let archive = archive().await;
    let profile = Profile::from_json(
        r#"{"name": "liver-2026", "options": ["retain-longitudinal-modified-dates", "retain-patient-characteristics", "clean-descriptors"]}"#,
    )
    .unwrap();
    let project = Project {
        ae_title: AeTitle::new("LIVER_2026").unwrap(),
        profile,
        keys: keys(),
        archive: archive.clone(),
        quarantine,
    };
    let scp = Scp::new(config(), Destination::Archive(archive.clone())).with_route(
        AeTitle::new("LIVER_2026").unwrap(),
        Destination::Project(Arc::new(project)),
    );
    (start_scp(scp).await, archive)
}

/// Sent to the project's AE title: Success, and only the de-identified instance is in the
/// archive, under its replacement UIDs, with the patient's pseudonym, Patient Identity
/// Removed YES, and File Meta Information written anew without the AE titles.
#[tokio::test]
async fn an_instance_sent_to_a_project_is_de_identified() {
    let (addr, archive) = with_project(None).await;
    let series = SyntheticSeries::new(101, 1);
    let mut client = Client::connect(addr, "HOSPITAL", "LIVER_2026", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            CT,
            CT,
            &series.instance_uid(0),
            &data_set(&series.slice_bytes(0)),
        )
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::SUCCESS),
        "{:?}",
        rsp.text(pacs_dimse::command::tag::ERROR_COMMENT)
    );
    client.release().await;

    let index = archive.index();
    assert!(
        index
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_none(),
        "no identified copy"
    );
    let new_uid = keys().uid(&series.instance_uid(0));
    let entry = index
        .rest_instance_by_uid(&new_uid)
        .await
        .unwrap()
        .expect("the de-identified instance");
    let file = archive.file_at(&entry.object_key).await.unwrap();
    let instance = pacs_core::Instance::from_bytes(&file).unwrap();
    assert_eq!(
        instance.key.study_instance_uid,
        keys().uid(&series.study_uid())
    );
    let o = &instance.object;
    let text = |tag| o.element(tag).unwrap().to_str().unwrap().trim().to_owned();
    assert_eq!(text(tags::PATIENT_IDENTITY_REMOVED), "YES");
    assert_ne!(text(tags::PATIENT_ID), series.patient_id());
    assert_ne!(text(tags::STUDY_DATE), "20260101", "dates shifted");
    let meta = o.meta();
    assert_eq!(
        meta.media_storage_sop_instance_uid().trim_end_matches('\0'),
        new_uid
    );
    assert_eq!(
        meta.implementation_class_uid().trim_end_matches('\0'),
        pacs_dimse::IMPLEMENTATION_CLASS_UID
    );
    assert!(
        meta.source_application_entity_title.is_none(),
        "no AE titles from the original meta"
    );
}

/// pacs-rs's own AE title is not a project: the instance is stored as received.
#[tokio::test]
async fn pacs_rs_own_title_stores_as_received() {
    let (addr, archive) = with_project(None).await;
    let series = SyntheticSeries::new(102, 1);
    let mut client = Client::connect(addr, "HOSPITAL", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            CT,
            CT,
            &series.instance_uid(0),
            &data_set(&series.slice_bytes(0)),
        )
        .await;
    assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    client.release().await;
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_some()
    );
}

/// The same instance sent twice to a project gets the same replacement UIDs: one instance,
/// kept as first stored, and Success both times.
#[tokio::test]
async fn a_project_instance_sent_twice_is_kept_once() {
    let (addr, archive) = with_project(None).await;
    let series = SyntheticSeries::new(103, 1);
    let mut client = Client::connect(addr, "HOSPITAL", "LIVER_2026", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    for _ in 0..2 {
        let rsp = client
            .store_as(
                CT,
                CT,
                &series.instance_uid(0),
                &data_set(&series.slice_bytes(0)),
            )
            .await;
        assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    }
    client.release().await;
    let study = keys().uid(&series.study_uid());
    let patient = archive
        .index()
        .rest_instance_by_uid(&keys().uid(&series.instance_uid(0)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(patient.study_instance_uid, study);
}

fn secondary_capture(series: &SyntheticSeries) -> Vec<u8> {
    let mut object = pacs_core::Instance::from_bytes(&series.slice_bytes(0))
        .unwrap()
        .object;
    object.put(DataElement::new(
        tags::SOP_CLASS_UID,
        VR::UI,
        PrimitiveValue::from(SC),
    ));
    object.meta_mut().media_storage_sop_class_uid = SC.into();
    let mut file = Vec::new();
    object.write_all(&mut file).unwrap();
    data_set(&file)
}

/// A Secondary Capture, which the profile quarantines, with no quarantine kept: refused with
/// Cannot understand (C003) and the reason; nothing archived.
#[tokio::test]
async fn a_quarantined_instance_is_refused_without_a_quarantine() {
    let (addr, archive) = with_project(None).await;
    let series = SyntheticSeries::new(104, 1);
    let mut client = Client::connect(addr, "HOSPITAL", "LIVER_2026", &[SC], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(SC, SC, &series.instance_uid(0), &secondary_capture(&series))
        .await;
    assert_eq!(rsp.status(), Ok(Status::CANNOT_UNDERSTAND_QUARANTINED));
    assert!(
        rsp.text(pacs_dimse::command::tag::ERROR_COMMENT)
            .unwrap()
            .contains("SOP Class")
    );
    client.release().await;
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&keys().uid(&series.instance_uid(0)))
            .await
            .unwrap()
            .is_none()
    );
}

/// With a quarantine store: Success (the instance is stored, in quarantine), the file is there
/// as received, and the archive does not have it.
#[tokio::test]
async fn a_quarantined_instance_goes_to_the_quarantine_store() {
    let quarantine = Store::memory();
    let (addr, archive) = with_project(Some(quarantine.clone())).await;
    let series = SyntheticSeries::new(105, 1);
    let mut client = Client::connect(addr, "HOSPITAL", "LIVER_2026", &[SC], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(SC, SC, &series.instance_uid(0), &secondary_capture(&series))
        .await;
    assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    client.release().await;
    let key = pacs_core::InstanceKey {
        study_instance_uid: series.study_uid(),
        series_instance_uid: series.series_uid(),
        sop_instance_uid: series.instance_uid(0),
        sop_class_uid: SC.into(),
        transfer_syntax_uid: EXPLICIT_VR_LE.into(),
    };
    let object_key = quarantine.key_for(&key).unwrap();
    assert!(
        quarantine.get(&object_key).await.is_ok(),
        "kept in quarantine"
    );
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_none()
    );
}

/// A Patient's Name in UN is a Patient's Name: the profile reaches it, and it does not survive
/// as bytes in the de-identified file.
#[tokio::test]
async fn a_name_in_un_is_de_identified() {
    let (addr, archive) = with_project(None).await;
    let file = pacs_core::synthetic::RawFile::new(EXPLICIT_VR_LE)
        .identifiers(102, b"UI")
        .long((0x0010, 0x0010), b"UN", b"REAL^NAME ");
    let mut client = Client::connect(addr, "HOSPITAL", "LIVER_2026", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(CT, CT, "2.25.1023", &data_set(&file.bytes()))
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::SUCCESS),
        "{:?}",
        rsp.text(pacs_dimse::command::tag::ERROR_COMMENT)
    );
    client.release().await;
    let entry = archive
        .index()
        .rest_instance_by_uid(&keys().uid("2.25.1023"))
        .await
        .unwrap()
        .expect("the de-identified instance");
    let stored = archive.file_at(&entry.object_key).await.unwrap();
    assert!(!stored.windows(9).any(|w| w == b"REAL^NAME"));
}
