//! The SCU against an archive that takes Patient Root only (PS3.4 C.6.1): a search goes
//! from the patients to their studies, each named by its Patient ID (C.4.1.2.2.1), and a
//! move names its studies under their patient (C.4.2.2.1). The archive here is a small SCP
//! that answers hierarchical queries from a table, as a Patient Root archive would, and
//! refuses what PS3.4 does not let a hierarchical query ask.

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::ServerAssociationOptions;
use pacs_dimse::command::{Command, NO_DATA_SET, field, tag};
use pacs_dimse::message::{self, DataSet, Incoming};
use pacs_dimse::query::{PATIENT_ROOT_FIND, PATIENT_ROOT_MOVE, wildcard_match};
use pacs_dimse::{AeTitle, MoveError, RemoteAe, StudyQuery, find_studies, move_studies};

const EXPLICIT: &str = "1.2.840.10008.1.2.1";

/// The archive's table: (Patient ID, Patient's Name, Study Instance UIDs).
const PATIENTS: &[(&str, &str, &[&str])] = &[
    ("LIV-1", "DOE^JANE", &["1.2.826.0.1.1", "1.2.826.0.1.2"]),
    ("LIV-2", "DOE^JOHN", &["1.2.826.0.1.3"]),
    ("KID-1", "ROE^RICHARD", &["1.2.826.0.1.4"]),
];

/// What the archive was asked, in order: each request's command and Identifier.
type Asked = Arc<Mutex<Vec<(Command, InMemDicomObject)>>>;

fn text(object: &InMemDicomObject, tag: dicom_core::Tag) -> String {
    object
        .element(tag)
        .ok()
        .and_then(|e| e.to_str().ok())
        .map(|s| s.trim_matches([' ', '\0']).to_owned())
        .unwrap_or_default()
}

fn put(object: &mut InMemDicomObject, tag: dicom_core::Tag, vr: VR, value: &str) {
    object.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
}

/// The matches of one hierarchical C-FIND, or the status refusing it: at STUDY level a
/// Patient ID of one value is required, being the unique key of the level above
/// (C.4.1.2.2.1); 0xA900 (Identifier does not match SOP Class) otherwise.
fn answer(identifier: &InMemDicomObject) -> Result<Vec<InMemDicomObject>, u16> {
    let pattern = |tag| {
        let value = text(identifier, tag);
        if value.is_empty() {
            "*".to_owned()
        } else {
            value
        }
    };
    match text(identifier, tags::QUERY_RETRIEVE_LEVEL).as_str() {
        "PATIENT" => Ok(PATIENTS
            .iter()
            .filter(|(id, name, _)| {
                wildcard_match(&pattern(tags::PATIENT_ID), id, false)
                    && wildcard_match(&pattern(tags::PATIENT_NAME), name, true)
            })
            .map(|(id, name, _)| {
                let mut found = InMemDicomObject::new_empty();
                put(&mut found, tags::QUERY_RETRIEVE_LEVEL, VR::CS, "PATIENT");
                put(&mut found, tags::PATIENT_ID, VR::LO, id);
                put(&mut found, tags::PATIENT_NAME, VR::PN, name);
                put(&mut found, tags::PATIENT_SEX, VR::CS, "F");
                found
            })
            .collect()),
        "STUDY" => {
            let patient = text(identifier, tags::PATIENT_ID);
            if patient.is_empty() || patient.contains(['*', '?']) {
                return Err(0xA900);
            }
            // a hierarchical query asks nothing of the levels above but their unique keys
            if identifier.element(tags::PATIENT_NAME).is_ok() {
                return Err(0xA900);
            }
            Ok(PATIENTS
                .iter()
                .filter(|(id, _, _)| *id == patient)
                .flat_map(|(id, _, studies)| studies.iter().map(move |uid| (id, uid)))
                .map(|(id, uid)| {
                    let mut found = InMemDicomObject::new_empty();
                    put(&mut found, tags::QUERY_RETRIEVE_LEVEL, VR::CS, "STUDY");
                    put(&mut found, tags::PATIENT_ID, VR::LO, id);
                    put(&mut found, tags::STUDY_INSTANCE_UID, VR::UI, uid);
                    put(&mut found, tags::STUDY_DATE, VR::DA, "20260105");
                    found
                })
                .collect())
        }
        _ => Err(0xA900),
    }
}

/// Serve one association as an archive of Patient Root FIND and MOVE only. A C-MOVE is
/// answered Success with as many sub-operations as studies named.
async fn patient_root_archive() -> (RemoteAe, Asked) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let asked: Asked = Arc::default();
    let record = asked.clone();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut association = ServerAssociationOptions::new()
            .ae_title("PATROOT")
            .with_abstract_syntax(PATIENT_ROOT_FIND)
            .with_abstract_syntax(PATIENT_ROOT_MOVE)
            .with_transfer_syntax(EXPLICIT)
            .read_timeout(Duration::from_secs(5))
            .establish_async(socket)
            .await
            .unwrap();
        let ts = TransferSyntaxRegistry.get(EXPLICIT).unwrap();
        loop {
            let Ok(Incoming::Message(rq)) = message::receive(&mut association, 1 << 20).await
            else {
                return;
            };
            let DataSet::Received(bytes) = rq.data else {
                panic!("a request without an Identifier")
            };
            let identifier = InMemDicomObject::read_dataset_with_ts(&bytes[..], ts).unwrap();
            let message_id = rq.command.message_id().unwrap();
            let sop_class = rq
                .command
                .ui(tag::AFFECTED_SOP_CLASS_UID)
                .unwrap()
                .to_owned();
            let is_move = rq.command.command_field() == Ok(field::C_MOVE_RQ);
            record
                .lock()
                .unwrap()
                .push((rq.command, identifier.clone()));
            let response = |status: u16| {
                Command::new()
                    .with_ui(tag::AFFECTED_SOP_CLASS_UID, &sop_class)
                    .with_us(
                        tag::COMMAND_FIELD,
                        if is_move {
                            field::C_MOVE_RSP
                        } else {
                            field::C_FIND_RSP
                        },
                    )
                    .with_us(tag::MESSAGE_ID_BEING_RESPONDED_TO, message_id)
                    .with_us(tag::STATUS, status)
            };
            let context = association.presentation_contexts()[0].id;
            let context = association
                .presentation_contexts()
                .iter()
                .find(|pc| pc.abstract_syntax.trim_end_matches('\0') == sop_class)
                .map_or(context, |pc| pc.id);
            if is_move {
                let studies = identifier
                    .element(tags::STUDY_INSTANCE_UID)
                    .unwrap()
                    .to_multi_str()
                    .unwrap()
                    .len();
                let done = response(0x0000)
                    .with_us(tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS, studies as u16)
                    .with_us(tag::NUMBER_OF_FAILED_SUB_OPERATIONS, 0)
                    .with_us(tag::NUMBER_OF_WARNING_SUB_OPERATIONS, 0)
                    .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET);
                message::send(&mut association, context, &done, None)
                    .await
                    .unwrap();
                continue;
            }
            let (matches, status) = match answer(&identifier) {
                Ok(matches) => (matches, 0x0000),
                Err(status) => (Vec::new(), status),
            };
            for found in matches {
                let mut data = Vec::new();
                found.write_dataset_with_ts(&mut data, ts).unwrap();
                let pending = response(0xFF00).with_us(tag::COMMAND_DATA_SET_TYPE, 0x0102);
                message::send(&mut association, context, &pending, Some(&data))
                    .await
                    .unwrap();
            }
            let last = response(status).with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET);
            message::send(&mut association, context, &last, None)
                .await
                .unwrap();
        }
    });
    let remote = format!("archive=PATROOT@127.0.0.1:{port}").parse().unwrap();
    (remote, asked)
}

fn calling() -> AeTitle {
    AeTitle::new("PACSRS").unwrap()
}

async fn search(
    remote: &RemoteAe,
    query: StudyQuery,
    stop_after: Option<usize>,
) -> (pacs_dimse::FindOutcome, Vec<InMemDicomObject>) {
    let mut found = Vec::new();
    let outcome = find_studies(remote, &calling(), &query, Duration::from_secs(3), |m| {
        found.push(m);
        match stop_after {
            Some(n) if found.len() >= n => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        }
    })
    .await
    .unwrap();
    (outcome, found)
}

fn levels(asked: &Asked) -> Vec<(String, String)> {
    asked
        .lock()
        .unwrap()
        .iter()
        .map(|(_, identifier)| {
            (
                text(identifier, tags::QUERY_RETRIEVE_LEVEL),
                text(identifier, tags::PATIENT_ID),
            )
        })
        .collect()
}

/// Patients first, by name, then the studies of each one found, by its Patient ID: every
/// request in Patient Root FIND, and each study comes back with its patient's name.
#[tokio::test]
async fn a_search_goes_from_the_patients_to_their_studies() {
    let (remote, asked) = patient_root_archive().await;
    let query = StudyQuery {
        patient_name: Some("doe^*".into()),
        ..Default::default()
    };
    let (outcome, found) = search(&remote, query, None).await;
    assert!(outcome.complete(), "{outcome:?}");
    let studies: Vec<(String, String, String)> = found
        .iter()
        .map(|m| {
            (
                text(m, tags::STUDY_INSTANCE_UID),
                text(m, tags::PATIENT_ID),
                text(m, tags::PATIENT_NAME),
            )
        })
        .collect();
    assert_eq!(
        studies,
        [
            ("1.2.826.0.1.1".into(), "LIV-1".into(), "DOE^JANE".into()),
            ("1.2.826.0.1.2".into(), "LIV-1".into(), "DOE^JANE".into()),
            ("1.2.826.0.1.3".into(), "LIV-2".into(), "DOE^JOHN".into()),
        ]
    );
    assert_eq!(
        levels(&asked),
        [
            ("PATIENT".into(), String::new()),
            ("STUDY".into(), "LIV-1".into()),
            ("STUDY".into(), "LIV-2".into()),
        ]
    );
    for (command, _) in asked.lock().unwrap().iter() {
        assert_eq!(
            command.ui(tag::AFFECTED_SOP_CLASS_UID),
            Ok(PATIENT_ROOT_FIND)
        );
    }
    // the message IDs differ on the one association
    let ids: Vec<u16> = asked
        .lock()
        .unwrap()
        .iter()
        .map(|(c, _)| c.message_id().unwrap())
        .collect();
    assert_eq!(ids, [1, 2, 3]);
}

/// The study keys go to the study level, filtered by the SCU as in Study Root; a patient
/// that has none of the studies asked for gives nothing.
#[tokio::test]
async fn the_study_keys_are_asked_one_level_down() {
    let (remote, asked) = patient_root_archive().await;
    let query = StudyQuery {
        patient_id: Some("LIV-*".into()),
        study_date: Some("20260101-20260131".parse().unwrap()),
        ..Default::default()
    };
    let (outcome, found) = search(&remote, query, None).await;
    assert_eq!(found.len(), 3);
    assert!(outcome.complete());
    let study_level = asked.lock().unwrap()[1].1.clone();
    assert_eq!(text(&study_level, tags::STUDY_DATE), "20260101-20260131");
    assert_eq!(text(&study_level, tags::PATIENT_ID), "LIV-1");

    let (remote, _) = patient_root_archive().await;
    let query = StudyQuery {
        patient_id: Some("KID-1".into()),
        study_date: Some("20250101-20250131".parse().unwrap()),
        ..Default::default()
    };
    let (outcome, found) = search(&remote, query, None).await;
    // the archive here does not match dates: the SCU's own filtering drops the study
    assert!(found.is_empty());
    assert_eq!(outcome.filtered_out, 1);
}

/// Stopping cancels the study query running and asks for no other patient's.
#[tokio::test]
async fn stopping_asks_no_further_patient() {
    let (remote, asked) = patient_root_archive().await;
    let query = StudyQuery {
        patient_name: Some("DOE*".into()),
        ..Default::default()
    };
    let (outcome, found) = search(&remote, query, Some(1)).await;
    assert_eq!(found.len(), 1);
    assert!(outcome.cancelled);
    assert!(!outcome.complete());
    let levels = levels(&asked);
    assert!(
        !levels.contains(&("STUDY".into(), "LIV-2".into())),
        "{levels:?}"
    );
}

/// A move in Patient Root names the studies under their patient: Patient ID, then the
/// Study Instance UIDs, in Patient Root MOVE.
#[tokio::test]
async fn a_move_names_the_studies_under_their_patient() {
    let (remote, asked) = patient_root_archive().await;
    let studies = vec!["1.2.826.0.1.1".to_owned(), "1.2.826.0.1.2".to_owned()];
    let outcome = move_studies(
        &remote,
        &calling(),
        Some("LIV-1"),
        &studies,
        &calling(),
        Duration::from_secs(3),
        Duration::from_secs(3),
        |_| ControlFlow::Continue(()),
    )
    .await
    .unwrap();
    assert_eq!(outcome.progress.completed, Some(2));
    let asked = asked.lock().unwrap();
    let (command, identifier) = &asked[0];
    assert_eq!(
        command.ui(tag::AFFECTED_SOP_CLASS_UID),
        Ok(PATIENT_ROOT_MOVE)
    );
    assert_eq!(command.text(tag::MOVE_DESTINATION), Some("PACSRS"));
    assert_eq!(text(identifier, tags::QUERY_RETRIEVE_LEVEL), "STUDY");
    assert_eq!(text(identifier, tags::PATIENT_ID), "LIV-1");
    assert_eq!(
        identifier
            .element(tags::STUDY_INSTANCE_UID)
            .unwrap()
            .to_multi_str()
            .unwrap()
            .to_vec(),
        studies
    );
}

/// Without a Patient ID a Patient Root archive cannot be asked: refused before any
/// request, with what to give. A Patient ID that is not one value is refused before any
/// connection.
#[tokio::test]
async fn a_move_without_its_patient_is_refused() {
    let (remote, asked) = patient_root_archive().await;
    let studies = vec!["1.2.826.0.1.1".to_owned()];
    let moved = |patient: Option<&'static str>, remote: RemoteAe| {
        let studies = studies.clone();
        async move {
            move_studies(
                &remote,
                &calling(),
                patient,
                &studies,
                &calling(),
                Duration::from_secs(3),
                Duration::from_secs(3),
                |_| ControlFlow::Continue(()),
            )
            .await
        }
    };
    let error = moved(None, remote).await.unwrap_err();
    assert!(
        matches!(&error, MoveError::Request(e) if e.contains("Patient Root") && e.contains("Patient ID")),
        "{error}"
    );
    assert!(asked.lock().unwrap().is_empty());
    let nowhere: RemoteAe = "archive=PATROOT@127.0.0.1:9".parse().unwrap();
    for bad in ["LIV-*", "LIV-1\\LIV-2"] {
        let error = moved(Some(bad), nowhere.clone()).await.unwrap_err();
        assert!(matches!(error, MoveError::Request(_)), "{bad}: {error}");
    }
}
