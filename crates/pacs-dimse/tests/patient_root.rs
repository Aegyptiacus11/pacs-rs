//! The Patient Root Query/Retrieve Information Model (PS3.4 C.6.1) as pacs-rs answers it,
//! driven by a bare dicom-ul client: C-FIND at its four levels, and C-MOVE and C-GET of a
//! patient, of studies, series and instances under their patient.

mod common;

use std::sync::Arc;

use common::storage::{Peer, storage_scp};
use common::*;
use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use pacs_archive::Archive;
use pacs_core::synthetic::SyntheticSeries;
use pacs_deid::{Keys, Profile};
use pacs_dimse::command::tag;
use pacs_dimse::query::{PATIENT_ROOT_FIND, PATIENT_ROOT_GET, PATIENT_ROOT_MOVE, STUDY_ROOT_FIND};
use pacs_dimse::{AeTitle, Config, Destination, Project, Scp, Status};

/// A configuration that answers VIEWER.
fn answering() -> Config {
    let mut config = config();
    config.remotes = vec!["viewer=VIEWER@127.0.0.1:11112".parse().unwrap()];
    config
}

async fn viewer(addr: std::net::SocketAddr, called: &str) -> Client {
    Client::connect_receiving(
        addr,
        "VIEWER",
        called,
        &[
            PATIENT_ROOT_FIND,
            PATIENT_ROOT_MOVE,
            PATIENT_ROOT_GET,
            STUDY_ROOT_FIND,
        ],
        &[CT],
        &[EXPLICIT_VR_LE, IMPLICIT_VR_LE],
    )
    .await
    .unwrap()
}

/// Doe (patient 800) with two studies, the first of two series; Roe (803) with one.
/// Seeds 801 (two slices), 802 (in study 801, three), 804 (one), 803 (one).
async fn two_patients(archive: &Archive) -> [SyntheticSeries; 4] {
    let series = [
        SyntheticSeries::new(801, 2)
            .for_patient(800)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        SyntheticSeries::new(802, 3)
            .in_study(801, 2)
            .for_patient(800)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000"),
        SyntheticSeries::new(804, 1)
            .for_patient(800)
            .with_patient("Doe^Jane", None)
            .at("20260301", "090000")
            .of_modality("MR"),
        SyntheticSeries::new(803, 1)
            .with_patient("Roe^Richard", None)
            .at("20260106", "120000"),
    ];
    for one in &series {
        ingest(archive, one).await;
    }
    series
}

fn sorted(mut values: Vec<Option<String>>) -> Vec<String> {
    values.sort();
    values.into_iter().map(Option::unwrap_or_default).collect()
}

fn uid_keys(level: &str, keys: &[(Tag, &str)]) -> dicom_object::InMemDicomObject {
    let keys: Vec<(Tag, VR, &str)> = keys
        .iter()
        .map(|(tag, value)| {
            let vr = if *tag == tags::PATIENT_ID {
                VR::LO
            } else {
                VR::UI
            };
            (*tag, vr, *value)
        })
        .collect();
    identifier(level, &keys)
}

fn instances(series: &[&SyntheticSeries]) -> Vec<String> {
    let mut uids: Vec<String> = series
        .iter()
        .flat_map(|one| (0..one.slices).map(|slice| one.instance_uid(slice)))
        .collect();
    uids.sort();
    uids
}

/// PS3.4 C.6.1.1.2: at the PATIENT level a patient is found once, however many studies it
/// has, with what is counted of it.
#[tokio::test]
async fn patients_are_found_once_each() {
    let archive = archive().await;
    two_patients(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let patients = |keys: &[(Tag, VR, &'static str)]| {
        let mut all = vec![
            (tags::PATIENT_NAME, VR::PN, ""),
            (tags::PATIENT_ID, VR::LO, ""),
            (tags::PATIENT_BIRTH_DATE, VR::DA, ""),
            (tags::PATIENT_SEX, VR::CS, ""),
            (tags::NUMBER_OF_PATIENT_RELATED_STUDIES, VR::IS, ""),
            (tags::NUMBER_OF_PATIENT_RELATED_SERIES, VR::IS, ""),
            (tags::NUMBER_OF_PATIENT_RELATED_INSTANCES, VR::IS, ""),
        ];
        all.retain(|(tag, ..)| !keys.iter().any(|(given, ..)| given == tag));
        all.extend_from_slice(keys);
        identifier("PATIENT", &all)
    };
    let answer = client.find(PATIENT_ROOT_FIND, &patients(&[])).await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING; 2]);
    assert_eq!(
        sorted(answer.values(tags::PATIENT_ID)),
        ["SYN800", "SYN803"]
    );
    let doe = answer
        .matches
        .iter()
        .find(|found| value(found, tags::PATIENT_ID).as_deref() == Some("SYN800"))
        .unwrap();
    for (tag, expected) in [
        (tags::QUERY_RETRIEVE_LEVEL, "PATIENT"),
        (tags::RETRIEVE_AE_TITLE, "PACSRS"),
        (tags::PATIENT_NAME, "Doe^Jane"),
        (tags::PATIENT_BIRTH_DATE, "19700101"),
        (tags::PATIENT_SEX, "O"),
        (tags::NUMBER_OF_PATIENT_RELATED_STUDIES, "2"),
        (tags::NUMBER_OF_PATIENT_RELATED_SERIES, "3"),
        (tags::NUMBER_OF_PATIENT_RELATED_INSTANCES, "6"),
    ] {
        assert_eq!(value(doe, tag).as_deref(), Some(expected), "{tag}");
    }
    // nothing of a study at this level
    assert!(doe.get(tags::STUDY_INSTANCE_UID).is_none());

    for (keys, expected, why) in [
        (
            vec![(tags::PATIENT_NAME, VR::PN, "roe*")],
            vec!["SYN803"],
            "a name, without regard to case",
        ),
        (
            vec![(tags::PATIENT_ID, VR::LO, "SYN80?")],
            vec!["SYN800", "SYN803"],
            "a wild card",
        ),
        (
            vec![(tags::PATIENT_ID, VR::LO, "SYN800")],
            vec!["SYN800"],
            "one patient",
        ),
        (
            vec![(tags::NUMBER_OF_PATIENT_RELATED_STUDIES, VR::IS, "2")],
            vec!["SYN800"],
            "what is counted",
        ),
        (vec![(tags::PATIENT_ID, VR::LO, "NOBODY")], vec![], "nobody"),
    ] {
        let answer = client.find(PATIENT_ROOT_FIND, &patients(&keys)).await;
        assert_eq!(answer.status(), Status::SUCCESS, "{why}");
        assert_eq!(sorted(answer.values(tags::PATIENT_ID)), expected, "{why}");
    }
    // a key of the study is not one of the patient: left out, and said
    let answer = client
        .find(
            PATIENT_ROOT_FIND,
            &patients(&[(tags::STUDY_DATE, VR::DA, "20260105")]),
        )
        .await;
    assert_eq!(answer.pending, [Status::PENDING_OPTIONAL_KEYS; 2]);
    assert!(answer.matches[0].get(tags::STUDY_DATE).is_none());
    client.release().await;
}

/// PS3.4 C.4.1.3.1.1 on the Patient Root model: a patient's studies under its Patient ID,
/// a study's series under both, a series' instances under the three; a study that is
/// another patient's is not under the Patient ID that was named.
#[tokio::test]
async fn studies_series_and_instances_are_found_under_their_patient() {
    let archive = archive().await;
    let [first, second, later, roe] = two_patients(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;

    let studies = |patient: &'static str, keys: &[(Tag, VR, &'static str)]| {
        let mut all = vec![
            (tags::PATIENT_ID, VR::LO, patient),
            (tags::STUDY_INSTANCE_UID, VR::UI, ""),
            (tags::STUDY_DATE, VR::DA, ""),
            (tags::MODALITIES_IN_STUDY, VR::CS, ""),
        ];
        all.extend_from_slice(keys);
        identifier("STUDY", &all)
    };
    let answer = client
        .find(PATIENT_ROOT_FIND, &studies("SYN800", &[]))
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert_eq!(answer.pending, [Status::PENDING; 2]);
    let mut expected = vec![first.study_uid(), later.study_uid()];
    expected.sort();
    assert_eq!(sorted(answer.values(tags::STUDY_INSTANCE_UID)), expected);
    for found in &answer.matches {
        // the Unique Key of the level above comes back; nothing else of the patient
        assert_eq!(value(found, tags::PATIENT_ID).unwrap(), "SYN800");
        assert_eq!(value(found, tags::QUERY_RETRIEVE_LEVEL).unwrap(), "STUDY");
        assert!(found.get(tags::PATIENT_NAME).is_none());
    }
    // the keys of the level narrow it
    let answer = client
        .find(
            PATIENT_ROOT_FIND,
            &identifier(
                "STUDY",
                &[
                    (tags::PATIENT_ID, VR::LO, "SYN800"),
                    (tags::STUDY_INSTANCE_UID, VR::UI, ""),
                    (tags::STUDY_DATE, VR::DA, "20260201-"),
                ],
            ),
        )
        .await;
    assert_eq!(
        answer.values(tags::STUDY_INSTANCE_UID),
        [Some(later.study_uid())]
    );
    // a key that is the patient's is not one of this level in this model: left out
    let answer = client
        .find(
            PATIENT_ROOT_FIND,
            &studies("SYN800", &[(tags::PATIENT_NAME, VR::PN, "")]),
        )
        .await;
    assert_eq!(answer.pending, [Status::PENDING_OPTIONAL_KEYS; 2]);
    assert!(answer.matches[0].get(tags::PATIENT_NAME).is_none());
    // the Patient ID is matched as it is written
    let answer = client
        .find(PATIENT_ROOT_FIND, &studies("syn800", &[]))
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert!(answer.matches.is_empty());

    let series = |patient: &str, study: &str| {
        identifier(
            "SERIES",
            &[
                (tags::PATIENT_ID, VR::LO, patient),
                (tags::STUDY_INSTANCE_UID, VR::UI, study),
                (tags::SERIES_INSTANCE_UID, VR::UI, ""),
                (tags::MODALITY, VR::CS, ""),
            ],
        )
    };
    let answer = client
        .find(PATIENT_ROOT_FIND, &series("SYN800", &first.study_uid()))
        .await;
    assert_eq!(
        answer.values(tags::SERIES_INSTANCE_UID),
        [Some(first.series_uid()), Some(second.series_uid())]
    );
    for found in &answer.matches {
        assert_eq!(value(found, tags::PATIENT_ID).unwrap(), "SYN800");
        assert_eq!(
            value(found, tags::STUDY_INSTANCE_UID).unwrap(),
            first.study_uid()
        );
    }
    // Roe's study under Doe's Patient ID, and Doe's under Roe's: not under these keys
    for (patient, study) in [("SYN800", roe.study_uid()), ("SYN803", first.study_uid())] {
        let answer = client
            .find(PATIENT_ROOT_FIND, &series(patient, &study))
            .await;
        assert_eq!(answer.status(), Status::SUCCESS);
        assert!(answer.matches.is_empty(), "{patient} {study}");
    }

    let images = |patient: &str| {
        identifier(
            "IMAGE",
            &[
                (tags::PATIENT_ID, VR::LO, patient),
                (tags::STUDY_INSTANCE_UID, VR::UI, &first.study_uid()),
                (tags::SERIES_INSTANCE_UID, VR::UI, &second.series_uid()),
                (tags::SOP_INSTANCE_UID, VR::UI, ""),
                (tags::INSTANCE_NUMBER, VR::IS, ""),
            ],
        )
    };
    let answer = client.find(PATIENT_ROOT_FIND, &images("SYN800")).await;
    assert_eq!(
        answer.values(tags::SOP_INSTANCE_UID),
        [0, 1, 2].map(|slice| Some(second.instance_uid(slice)))
    );
    assert!(
        client
            .find(PATIENT_ROOT_FIND, &images("SYN803"))
            .await
            .matches
            .is_empty()
    );
    client.release().await;
}

/// The hierarchy of each model is its own (PS3.4 C.6.1.1, C.6.2.1): below the PATIENT
/// level the Patient Root model needs one Patient ID; the Study Root model has no PATIENT
/// level, and needs none.
#[tokio::test]
async fn each_model_has_its_levels_and_its_unique_keys() {
    let archive = archive().await;
    let [first, ..] = two_patients(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let study = first.study_uid();
    for (query, offending, why) in [
        (
            identifier("STUDY", &[(tags::STUDY_INSTANCE_UID, VR::UI, "")]),
            tags::PATIENT_ID,
            "no Patient ID at the STUDY level",
        ),
        (
            identifier(
                "STUDY",
                &[
                    (tags::PATIENT_ID, VR::LO, "SYN80*"),
                    (tags::STUDY_INSTANCE_UID, VR::UI, ""),
                ],
            ),
            tags::PATIENT_ID,
            "a pattern is not one Patient ID",
        ),
        (
            identifier(
                "SERIES",
                &[
                    (tags::STUDY_INSTANCE_UID, VR::UI, &study),
                    (tags::SERIES_INSTANCE_UID, VR::UI, ""),
                ],
            ),
            tags::PATIENT_ID,
            "no Patient ID at the SERIES level",
        ),
        (
            identifier(
                "SERIES",
                &[
                    (tags::PATIENT_ID, VR::LO, "SYN800"),
                    (tags::SERIES_INSTANCE_UID, VR::UI, ""),
                ],
            ),
            tags::STUDY_INSTANCE_UID,
            "no Study Instance UID at the SERIES level",
        ),
        (
            identifier("WORKLIST", &[(tags::PATIENT_ID, VR::LO, "")]),
            tags::QUERY_RETRIEVE_LEVEL,
            "a level no model has",
        ),
    ] {
        let answer = client.find(PATIENT_ROOT_FIND, &query).await;
        assert_eq!(
            answer.status(),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            "{why}"
        );
        let offending_element = answer.last.get(tag::OFFENDING_ELEMENT).map(|bytes| {
            Tag(
                u16::from_le_bytes([bytes[0], bytes[1]]),
                u16::from_le_bytes([bytes[2], bytes[3]]),
            )
        });
        assert_eq!(offending_element, Some(offending), "{why}");
    }
    // the same association, the other model: no PATIENT level, and a study needs no patient
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &identifier("PATIENT", &[(tags::PATIENT_ID, VR::LO, "")]),
        )
        .await;
    assert_eq!(answer.status(), Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS);
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &identifier(
                "STUDY",
                &[
                    (tags::STUDY_INSTANCE_UID, VR::UI, ""),
                    (tags::PATIENT_NAME, VR::PN, ""),
                ],
            ),
        )
        .await;
    assert_eq!(answer.pending, [Status::PENDING; 3]);
    client.release().await;
}

/// A study without a Patient ID belongs to no patient of the Patient Root model: it is
/// found by the Study Root model.
#[tokio::test]
async fn a_study_without_a_patient_id_is_no_patient() {
    let archive = archive().await;
    let series = SyntheticSeries::new(811, 1);
    let mut object = dicom_object::from_reader(&*series.slice_bytes(0)).unwrap();
    object.remove_element(tags::PATIENT_ID);
    let mut bytes = Vec::new();
    object.write_all(&mut bytes).unwrap();
    archive.ingest(bytes.into()).await.unwrap();
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let answer = client
        .find(
            PATIENT_ROOT_FIND,
            &identifier("PATIENT", &[(tags::PATIENT_ID, VR::LO, "")]),
        )
        .await;
    assert_eq!(answer.status(), Status::SUCCESS);
    assert!(answer.matches.is_empty());
    let answer = client
        .find(
            STUDY_ROOT_FIND,
            &identifier("STUDY", &[(tags::STUDY_INSTANCE_UID, VR::UI, "")]),
        )
        .await;
    assert_eq!(
        answer.values(tags::STUDY_INSTANCE_UID),
        [Some(series.study_uid())]
    );
    client.release().await;
}

/// PS3.4 C.6.1.1.6: a C-GET at the PATIENT level sends every instance of the patient; at
/// the levels below, what is named under the patient, and nothing of another patient.
#[tokio::test]
async fn a_patient_is_retrieved_whole_and_by_parts_with_c_get() {
    let archive = archive().await;
    let [first, second, later, roe] = two_patients(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let mut get = async |identifier: dicom_object::InMemDicomObject| -> (Status, Vec<String>) {
        let (retrieval, got) = client
            .c_get(PATIENT_ROOT_GET, &identifier, Answering::With(&[]))
            .await
            .unwrap();
        let mut uids: Vec<String> = got
            .iter()
            .map(|store| {
                store
                    .command
                    .ui(tag::AFFECTED_SOP_INSTANCE_UID)
                    .unwrap()
                    .to_owned()
            })
            .collect();
        uids.sort();
        (retrieval.status(), uids)
    };
    assert_eq!(
        get(uid_keys("PATIENT", &[(tags::PATIENT_ID, "SYN800")])).await,
        (Status::SUCCESS, instances(&[&first, &second, &later]))
    );
    assert_eq!(
        get(uid_keys("PATIENT", &[(tags::PATIENT_ID, "NOBODY")])).await,
        (Status::SUCCESS, Vec::new())
    );
    // studies under their patient; one that is another patient's is not sent
    let both = format!("{}\\{}", later.study_uid(), roe.study_uid());
    assert_eq!(
        get(uid_keys(
            "STUDY",
            &[
                (tags::PATIENT_ID, "SYN800"),
                (tags::STUDY_INSTANCE_UID, &both)
            ]
        ))
        .await,
        (Status::SUCCESS, instances(&[&later]))
    );
    let series = [
        (tags::PATIENT_ID, "SYN800"),
        (tags::STUDY_INSTANCE_UID, &first.study_uid()),
        (tags::SERIES_INSTANCE_UID, &second.series_uid()),
    ];
    assert_eq!(
        get(uid_keys("SERIES", &series)).await,
        (Status::SUCCESS, instances(&[&second]))
    );
    let mut image = series.to_vec();
    let one = second.instance_uid(1);
    image.push((tags::SOP_INSTANCE_UID, &one));
    assert_eq!(
        get(uid_keys("IMAGE", &image)).await,
        (Status::SUCCESS, vec![second.instance_uid(1)])
    );
    // the same series under Roe's Patient ID
    let mut wrong = series.to_vec();
    wrong[0] = (tags::PATIENT_ID, "SYN803");
    assert_eq!(
        get(uid_keys("SERIES", &wrong)).await,
        (Status::SUCCESS, Vec::new())
    );
    // a retrieval that names no patient is not one of this model
    let (retrieval, _) = client
        .c_get(
            PATIENT_ROOT_GET,
            &uid_keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &first.study_uid())]),
            Answering::With(&[]),
        )
        .await
        .unwrap();
    assert_eq!(
        retrieval.status(),
        Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS
    );
    client.release().await;
}

/// The same with C-MOVE: a patient whole to the Move Destination, counted.
#[tokio::test]
async fn a_patient_is_sent_whole_with_c_move() {
    let archive = archive().await;
    let [first, second, later, _] = two_patients(&archive).await;
    let (destination, received) = storage_scp(&[CT], &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let mut config = answering();
    config.remotes.push(destination.clone());
    config.move_destinations = vec![destination.id.clone()];
    let addr = start(config, archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(
            PATIENT_ROOT_MOVE,
            "STORESCP",
            &uid_keys("PATIENT", &[(tags::PATIENT_ID, "SYN800")]),
        )
        .await;
    assert_eq!(moved.status(), Status::SUCCESS);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(6), Some(0), Some(0)]
    );
    assert_eq!(moved.pending.len(), 5);
    assert_eq!(
        moved.last.ui(tag::AFFECTED_SOP_CLASS_UID),
        Ok(PATIENT_ROOT_MOVE)
    );
    // a study alone, under its patient, to a destination that is not one
    let study = uid_keys(
        "STUDY",
        &[
            (tags::PATIENT_ID, "SYN800"),
            (tags::STUDY_INSTANCE_UID, &later.study_uid()),
        ],
    );
    let refused = client.c_move(PATIENT_ROOT_MOVE, "NOBODY", &study).await;
    assert_eq!(refused.status(), Status::MOVE_DESTINATION_UNKNOWN);
    client.release().await;
    let mut got: Vec<String> = received
        .lock()
        .unwrap()
        .iter()
        .map(|store| {
            store
                .command
                .ui(tag::AFFECTED_SOP_INSTANCE_UID)
                .unwrap()
                .to_owned()
        })
        .collect();
    got.sort();
    assert_eq!(got, instances(&[&first, &second, &later]));
}

/// Rights per project hold in this model as in the other: under a project's AE title a
/// patient is its studies in the project, found and retrieved.
#[tokio::test]
async fn a_project_ae_title_shows_a_patient_as_its_studies_in_the_project() {
    let archive = archive().await;
    // one patient, a study in the project and one outside it
    let outside = SyntheticSeries::new(821, 2).for_patient(820);
    ingest(&archive, &outside).await;
    let liver = archive.clone().for_project("liver-2026");
    let inside = SyntheticSeries::new(822, 1).for_patient(820);
    ingest(&liver, &inside).await;
    let project = Project {
        ae_title: AeTitle::new("LIVER_2026").unwrap(),
        profile: Profile::from_json(r#"{"name": "liver-2026"}"#).unwrap(),
        keys: Keys::new(*b"a project secret of 32 bytes ok!").unwrap(),
        archive: liver,
        quarantine: None,
    };
    let scp = Scp::new(answering(), Destination::Archive(archive)).with_route(
        AeTitle::new("LIVER_2026").unwrap(),
        Destination::Project(Arc::new(project)),
    );
    let addr = start_scp(scp).await;
    let patient = identifier(
        "PATIENT",
        &[
            (tags::PATIENT_ID, VR::LO, "SYN820"),
            (tags::NUMBER_OF_PATIENT_RELATED_STUDIES, VR::IS, ""),
            (tags::NUMBER_OF_PATIENT_RELATED_INSTANCES, VR::IS, ""),
        ],
    );
    let whole = uid_keys("PATIENT", &[(tags::PATIENT_ID, "SYN820")]);
    for (called, studies, instances_seen, sent) in [
        ("LIVER_2026", "1", "1", instances(&[&inside])),
        ("PACSRS", "2", "3", instances(&[&outside, &inside])),
    ] {
        let mut client = viewer(addr, called).await;
        let answer = client.find(PATIENT_ROOT_FIND, &patient).await;
        assert_eq!(answer.pending, [Status::PENDING], "{called}");
        let found = &answer.matches[0];
        assert_eq!(
            value(found, tags::NUMBER_OF_PATIENT_RELATED_STUDIES).unwrap(),
            studies
        );
        assert_eq!(
            value(found, tags::NUMBER_OF_PATIENT_RELATED_INSTANCES).unwrap(),
            instances_seen
        );
        let (retrieval, got) = client
            .c_get(PATIENT_ROOT_GET, &whole, Answering::With(&[]))
            .await
            .unwrap();
        assert_eq!(retrieval.status(), Status::SUCCESS);
        let mut uids: Vec<String> = got
            .iter()
            .map(|store| {
                store
                    .command
                    .ui(tag::AFFECTED_SOP_INSTANCE_UID)
                    .unwrap()
                    .to_owned()
            })
            .collect();
        uids.sort();
        assert_eq!(uids, sent, "{called}");
        client.release().await;
    }
}
