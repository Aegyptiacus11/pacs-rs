//! pacs-rs as a Retrieve SCP, driven by a bare dicom-ul client and a Storage SCP that
//! plays the destination: C-MOVE on the Study Root model (PS3.4 C.4.2.3, C.6.2; PS3.7
//! 9.1.4), its counts and final statuses, its failures, who is answered, where it sends
//! and what a project's AE title gives.

mod common;

use std::sync::Arc;

use common::storage::{Peer, storage_scp};
use common::*;
use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use pacs_archive::Archive;
use pacs_core::sample::SampleSeries;
use pacs_deid::{Keys, Profile};
use pacs_dimse::command::{Command, NO_DATA_SET, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_dimse::scu::retrieve::STUDY_ROOT_MOVE;
use pacs_dimse::{AeTitle, Config, Destination, Project, RemoteAe, Scp, Status};

/// A configuration that answers VIEWER, reads from HOSPITAL and sends to `destination`.
fn sending_to(destination: &RemoteAe) -> Config {
    let mut config = config();
    config.remotes = vec![
        "viewer=VIEWER@127.0.0.1:11112".parse().unwrap(),
        "hospital=HOSP_PACS@127.0.0.1:104".parse().unwrap(),
        destination.clone(),
    ];
    config.move_destinations = vec![destination.id.clone()];
    config
}

async fn viewer(addr: std::net::SocketAddr, called: &str) -> Client {
    Client::connect(
        addr,
        "VIEWER",
        called,
        &[STUDY_ROOT_MOVE],
        &[EXPLICIT_VR_LE, IMPLICIT_VR_LE],
    )
    .await
    .unwrap()
}

/// Doe's study of a CT series (seed 601, two slices) and an MR one (602, three), and
/// Roe's CT (603, one).
async fn two_studies(archive: &Archive) -> [SampleSeries; 3] {
    let series = [
        SampleSeries::new(601, 2).with_patient("Doe^Jane", None),
        SampleSeries::new(602, 3)
            .in_study(601, 2)
            .with_patient("Doe^Jane", None),
        SampleSeries::new(603, 1).with_patient("Roe^Richard", None),
    ];
    for one in &series {
        ingest(archive, one).await;
    }
    series
}

fn keys(level: &str, keys: &[(Tag, &str)]) -> dicom_object::InMemDicomObject {
    let keys: Vec<(Tag, VR, &str)> = keys.iter().map(|(tag, uid)| (*tag, VR::UI, *uid)).collect();
    identifier(level, &keys)
}

fn study(series: &SampleSeries) -> dicom_object::InMemDicomObject {
    keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &series.study_uid())])
}

const ALL: &[&str] = &[CT, MR];

/// PS3.4 C.4.2.3.1: every instance of the study goes to the Move Destination by C-STORE
/// on another association; the Pending responses count them as they go, and the final one
/// is Success with the counts and without a data set.
#[tokio::test]
async fn a_study_is_sent_to_the_move_destination() {
    let archive = archive().await;
    let [ct, mr, _] = two_studies(&archive).await;
    let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;

    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    assert_eq!(moved.status(), Status::SUCCESS);
    assert!(moved.failed.is_empty());
    // five sub-operations: a Pending response after each but the last, then the final one
    let pending: Vec<[Option<u16>; 4]> = moved.pending.iter().map(Retrieval::counts).collect();
    assert_eq!(
        pending,
        [
            [Some(4), Some(1), Some(0), Some(0)],
            [Some(3), Some(2), Some(0), Some(0)],
            [Some(2), Some(3), Some(0), Some(0)],
            [Some(1), Some(4), Some(0), Some(0)],
        ]
    );
    for response in &moved.pending {
        assert_eq!(response.status(), Ok(Status::PENDING));
        assert_eq!(response.command_field(), Ok(field::C_MOVE_RSP));
    }
    // no Remaining count in a final response (C.4.2.1.6)
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(5), Some(0), Some(0)]
    );
    assert_eq!(
        moved.last.ui(tag::AFFECTED_SOP_CLASS_UID),
        Ok(STUDY_ROOT_MOVE)
    );
    client.release().await;

    let received = received.lock().unwrap();
    let mut got: Vec<String> = received
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
    let mut expected: Vec<String> = (0..2)
        .map(|slice| ct.instance_uid(slice))
        .chain((0..3).map(|slice| mr.instance_uid(slice)))
        .collect();
    expected.sort();
    assert_eq!(got, expected);
    for store in received.iter() {
        // sent by the AE that was asked, for the AE that asked (PS3.7 9.3.1.1)
        assert_eq!(store.calling, "PACSRS");
        assert_eq!(
            store.command.text(tag::MOVE_ORIGINATOR_AE_TITLE),
            Some("VIEWER")
        );
        assert_eq!(
            store.command.us(tag::MOVE_ORIGINATOR_MESSAGE_ID),
            moved.last.us(tag::MESSAGE_ID_BEING_RESPONDED_TO)
        );
        assert_eq!(
            value(&store.object(), tags::PATIENT_NAME).unwrap(),
            "Doe^Jane"
        );
    }
}

/// PS3.4 C.6.2.1.5: a retrieval at the SERIES level sends the instances of the series
/// named, one at the IMAGE level the instances named, each a list of UID under single
/// values above; several studies go by a list at the STUDY level.
#[tokio::test]
async fn series_instances_and_several_studies_are_selected_by_their_unique_keys() {
    let archive = archive().await;
    let [ct, mr, roe] = two_studies(&archive).await;
    let sent = async |identifier: dicom_object::InMemDicomObject| -> (Status, Vec<String>) {
        let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
        let addr = start(sending_to(&destination), archive.clone()).await;
        let mut client = viewer(addr, "PACSRS").await;
        let moved = client
            .c_move(STUDY_ROOT_MOVE, "STORESCP", &identifier)
            .await;
        client.release().await;
        let mut uids: Vec<String> = received
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
        uids.sort();
        (moved.status(), uids)
    };
    let study_uid = ct.study_uid();
    let of = |series: &SampleSeries, slices: std::ops::Range<u32>| -> Vec<String> {
        slices.map(|slice| series.instance_uid(slice)).collect()
    };

    let one_series = keys(
        "SERIES",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &mr.series_uid()),
        ],
    );
    assert_eq!(sent(one_series).await, (Status::SUCCESS, of(&mr, 0..3)));

    let both = format!("{}\\{}", ct.series_uid(), mr.series_uid());
    let two_series = keys(
        "SERIES",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &both),
        ],
    );
    let mut all = of(&ct, 0..2);
    all.extend(of(&mr, 0..3));
    all.sort();
    assert_eq!(sent(two_series).await, (Status::SUCCESS, all.clone()));

    let two = format!("{}\\{}", mr.instance_uid(0), mr.instance_uid(2));
    let instances = keys(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &mr.series_uid()),
            (tags::SOP_INSTANCE_UID, &two),
        ],
    );
    assert_eq!(
        sent(instances).await,
        (
            Status::SUCCESS,
            vec![mr.instance_uid(0), mr.instance_uid(2)]
        )
    );

    let studies = format!("{}\\{}", study_uid, roe.study_uid());
    let mut everything = all;
    everything.extend(of(&roe, 0..1));
    everything.sort();
    assert_eq!(
        sent(keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &studies)])).await,
        (Status::SUCCESS, everything)
    );

    // an instance that is in another series than the one named is not under these keys
    let elsewhere = keys(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &ct.series_uid()),
            (tags::SOP_INSTANCE_UID, &mr.instance_uid(0)),
        ],
    );
    assert_eq!(sent(elsewhere).await, (Status::SUCCESS, Vec::new()));
}

/// PS3.4 Table C.4-2: a Move Destination that is not one this server sends to is A801,
/// whether it is unknown, a remote AE that is only read from, or not given; nothing is
/// sent anywhere.
#[tokio::test]
async fn a_move_to_another_destination_is_refused_a801() {
    let archive = archive().await;
    let [ct, ..] = two_studies(&archive).await;
    let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    for elsewhere in ["NOBODY", "HOSP_PACS", "VIEWER", "peer", ""] {
        let moved = client.c_move(STUDY_ROOT_MOVE, elsewhere, &study(&ct)).await;
        assert_eq!(
            moved.status(),
            Status::MOVE_DESTINATION_UNKNOWN,
            "{elsewhere}"
        );
        assert!(moved.pending.is_empty());
        let comment = moved.last.text(tag::ERROR_COMMENT).unwrap();
        assert!(comment.contains("not a destination"), "{comment}");
        // no sub-operation, so no count of them
        assert_eq!(Retrieval::counts(&moved.last), [None; 4]);
    }
    assert!(received.lock().unwrap().is_empty());
    // the association goes on: the same study to the destination that is one
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    assert_eq!(moved.status(), Status::SUCCESS);
    client.release().await;
}

/// The failures before any sub-operation: a caller that is not configured (0124), an
/// Identifier that is not one of the model (A900) or cannot be read (C000).
#[tokio::test]
async fn a_request_that_cannot_be_served_is_a_failure() {
    let archive = archive().await;
    let [ct, mr, _] = two_studies(&archive).await;
    let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive).await;

    let mut stranger = Client::connect(
        addr,
        "STRANGER",
        "PACSRS",
        &[STUDY_ROOT_MOVE],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let moved = stranger
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    assert_eq!(moved.status(), Status::NOT_AUTHORIZED);
    assert!(
        moved
            .last
            .text(tag::ERROR_COMMENT)
            .unwrap()
            .contains("STRANGER")
    );
    stranger.release().await;

    let mut client = viewer(addr, "PACSRS").await;
    let two_studies = format!("{}\\2.25.9", ct.study_uid());
    for (identifier, offending, why) in [
        (
            keys("PATIENT", &[(tags::STUDY_INSTANCE_UID, &ct.study_uid())]),
            tags::QUERY_RETRIEVE_LEVEL,
            "a level the Study Root model lacks",
        ),
        (keys("STUDY", &[]), tags::STUDY_INSTANCE_UID, "no study"),
        (
            keys("STUDY", &[(tags::STUDY_INSTANCE_UID, "")]),
            tags::STUDY_INSTANCE_UID,
            "an empty Unique Key: a retrieval has no universal matching",
        ),
        (
            keys("SERIES", &[(tags::STUDY_INSTANCE_UID, &ct.study_uid())]),
            tags::SERIES_INSTANCE_UID,
            "no series at the SERIES level",
        ),
        (
            keys(
                "SERIES",
                &[
                    (tags::STUDY_INSTANCE_UID, &two_studies),
                    (tags::SERIES_INSTANCE_UID, &mr.series_uid()),
                ],
            ),
            tags::STUDY_INSTANCE_UID,
            "two studies above the SERIES level",
        ),
    ] {
        let moved = client
            .c_move(STUDY_ROOT_MOVE, "STORESCP", &identifier)
            .await;
        assert_eq!(
            moved.status(),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
            "{why}"
        );
        let offending_element = moved.last.get(tag::OFFENDING_ELEMENT).map(|bytes| {
            Tag(
                u16::from_le_bytes([bytes[0], bytes[1]]),
                u16::from_le_bytes([bytes[2], bytes[3]]),
            )
        });
        assert_eq!(offending_element, Some(offending), "{why}");
        assert!(
            moved
                .last
                .text(tag::ERROR_COMMENT)
                .is_some_and(|c| !c.is_empty())
        );
    }
    // bytes that are not a data set, and no Identifier at all
    let id = client.message_id();
    let context = client.context(STUDY_ROOT_MOVE);
    message::send(
        &mut client.association,
        context,
        &c_move_rq(id, STUDY_ROOT_MOVE, "STORESCP"),
        Some(&[0x08, 0x00, 0x52, 0x00, 0xFF, 0xFF]),
    )
    .await
    .unwrap();
    assert_eq!(
        client.retrieval(STUDY_ROOT_MOVE).await.status(),
        Status::UNABLE_TO_PROCESS
    );
    let id = client.message_id();
    let rsp = client
        .response(
            context,
            &no_data(c_move_rq(id, STUDY_ROOT_MOVE, "STORESCP")),
            None,
        )
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS)
    );
    assert!(received.lock().unwrap().is_empty());
    client.release().await;
}

/// A study that is not there has no instance to send: Success with nothing counted, and
/// the destination is not called.
#[tokio::test]
async fn a_study_that_is_not_there_is_success_with_nothing_sent() {
    let archive = archive().await;
    two_studies(&archive).await;
    let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(
            STUDY_ROOT_MOVE,
            "STORESCP",
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, "2.25.404")]),
        )
        .await;
    assert_eq!(moved.status(), Status::SUCCESS);
    assert!(moved.pending.is_empty());
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(0), Some(0), Some(0)]
    );
    assert!(received.lock().unwrap().is_empty());
    client.release().await;
}

/// PS3.4 Table C.4-2: a destination that cannot be associated with is Refused: Out of
/// resources, unable to perform sub-operations (A702), every instance failed and named.
#[tokio::test]
async fn a_destination_that_does_not_answer_is_a702_with_every_instance_failed() {
    let archive = archive().await;
    let [ct, mr, _] = two_studies(&archive).await;
    // a port nothing listens on
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let gone: RemoteAe = format!("gone=STORESCP@127.0.0.1:{port}").parse().unwrap();
    let addr = start(sending_to(&gone), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    assert_eq!(moved.status(), Status::UNABLE_TO_PERFORM_SUB_OPERATIONS);
    assert!(moved.pending.is_empty());
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(0), Some(5), Some(0)]
    );
    let mut failed = moved.failed.clone();
    failed.sort();
    let mut expected: Vec<String> = (0..2)
        .map(|slice| ct.instance_uid(slice))
        .chain((0..3).map(|slice| mr.instance_uid(slice)))
        .collect();
    expected.sort();
    assert_eq!(failed, expected);
    assert!(
        moved
            .last
            .text(tag::ERROR_COMMENT)
            .unwrap()
            .contains("STORESCP")
    );
    client.release().await;
}

/// PS3.4 C.4.2.3.1: a sub-operation is a failure when the destination accepted no
/// presentation context for the instance's SOP class, or answered with a failure; with
/// some stored the final status is Warning (B000), with the failed instances named.
#[tokio::test]
async fn what_the_destination_does_not_take_fails_and_the_move_ends_in_a_warning() {
    let archive = archive().await;
    // a study of a CT series and an MR series: SOP classes CT and MR
    let ct = SampleSeries::new(611, 2);
    ingest(&archive, &ct).await;
    let mut mr_files = Vec::new();
    let mr = SampleSeries::new(612, 2).in_study(611, 2);
    for slice in 0..2 {
        // the same slices under the MR Image Storage SOP class
        let mut object = dicom_object::from_reader(&*mr.slice_bytes(slice)).unwrap();
        object.put(dicom_core::DataElement::new(
            tags::SOP_CLASS_UID,
            VR::UI,
            dicom_core::PrimitiveValue::from(MR),
        ));
        let meta = dicom_object::FileMetaTableBuilder::new()
            .transfer_syntax(EXPLICIT_VR_LE)
            .media_storage_sop_class_uid(MR)
            .media_storage_sop_instance_uid(mr.instance_uid(slice))
            .build()
            .unwrap();
        let mut bytes = Vec::new();
        object
            .into_inner()
            .with_exact_meta(meta)
            .write_all(&mut bytes)
            .unwrap();
        mr_files.push(bytes);
    }
    for file in mr_files {
        archive.ingest(file.into()).await.unwrap();
    }

    // a destination that takes CT only: the MR instances are not sent
    let (destination, received) = storage_scp(&[CT], &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive.clone()).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    client.release().await;
    assert_eq!(moved.status(), Status::SUB_OPERATIONS_WARNING);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(2), Some(2), Some(0)]
    );
    let mut failed = moved.failed.clone();
    failed.sort();
    assert_eq!(failed, [mr.instance_uid(0), mr.instance_uid(1)]);
    assert_eq!(received.lock().unwrap().len(), 2);
    // the counts add up at every step
    for response in &moved.pending {
        let [remaining, completed, failed, warning] =
            Retrieval::counts(response).map(Option::unwrap);
        assert_eq!(remaining + completed + failed + warning, 4);
    }

    // one that refuses the first instance, and stores the second with a warning
    let (destination, _) =
        storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[0xA700, 0xB000])).await;
    let addr = start(sending_to(&destination), archive.clone()).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    client.release().await;
    assert_eq!(moved.status(), Status::SUB_OPERATIONS_WARNING);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(2), Some(1), Some(1)]
    );
    assert_eq!(moved.failed.len(), 1);

    // one that refuses every instance: Failure, all of them named
    let (destination, _) = storage_scp(
        ALL,
        &[EXPLICIT_VR_LE],
        Peer::Answers(&[0xA700, 0xA700, 0xC000, 0xA900]),
    )
    .await;
    let addr = start(sending_to(&destination), archive.clone()).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    client.release().await;
    assert_eq!(moved.status(), Status::UNABLE_TO_PERFORM_SUB_OPERATIONS);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(0), Some(4), Some(0)]
    );
    assert_eq!(moved.failed.len(), 4);

    // one that aborts at the first instance: the rest cannot be sent, and fails too
    let (destination, _) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Aborts).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let moved = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&ct))
        .await;
    client.release().await;
    assert_eq!(moved.status(), Status::UNABLE_TO_PERFORM_SUB_OPERATIONS);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [None, Some(0), Some(4), Some(0)]
    );
}

fn c_cancel_rq(message_id: u16) -> Command {
    Command::new()
        .with_us(tag::COMMAND_FIELD, field::C_CANCEL_RQ)
        .with_us(tag::MESSAGE_ID_BEING_RESPONDED_TO, message_id)
        .with_us(tag::COMMAND_DATA_SET_TYPE, NO_DATA_SET)
}

/// PS3.4 C.4.2.3.1: a C-CANCEL-MOVE-RQ interrupts the sub-operations; the response is
/// Cancel with the counts, Remaining being those that were not started. The destination
/// holds its answer to the second instance, and the cancel is sent while it does, so this
/// does not depend on who is faster.
#[tokio::test]
async fn a_cancel_stops_the_sub_operations() {
    let archive = archive().await;
    let [ct, ..] = two_studies(&archive).await;
    let go = Arc::new(tokio::sync::Notify::new());
    let (destination, received) =
        storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Holds(1, go.clone())).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let id = client.message_id();
    let context = client.context(STUDY_ROOT_MOVE);
    let mut bytes = Vec::new();
    study(&ct)
        .write_dataset_with_ts(&mut bytes, client.transfer_syntax(STUDY_ROOT_MOVE))
        .unwrap();
    message::send(
        &mut client.association,
        context,
        &c_move_rq(id, STUDY_ROOT_MOVE, "STORESCP"),
        Some(&bytes),
    )
    .await
    .unwrap();
    // the first instance is stored
    match message::receive(&mut client.association, 1 << 20)
        .await
        .unwrap()
    {
        Incoming::Message(first) => {
            assert_eq!(first.command.status(), Ok(Status::PENDING));
            assert_eq!(
                Retrieval::counts(&first.command),
                [Some(4), Some(1), Some(0), Some(0)]
            );
        }
        other => panic!("expected a Pending response, got {other:?}"),
    }
    for _ in 0..200 {
        if received.lock().unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(received.lock().unwrap().len(), 2);
    // the server waits for the destination's answer: the cancel is there before it
    message::send(&mut client.association, context, &c_cancel_rq(id), None)
        .await
        .unwrap();
    go.notify_one();
    let moved = client.retrieval(STUDY_ROOT_MOVE).await;
    assert_eq!(moved.status(), Status::CANCEL);
    // the second instance was under way and is stored; three were not started
    assert_eq!(moved.pending.len(), 1);
    assert_eq!(
        Retrieval::counts(&moved.last),
        [Some(3), Some(2), Some(0), Some(0)]
    );
    assert!(moved.failed.is_empty());
    assert_eq!(received.lock().unwrap().len(), 2);
    // the association is whole
    let again = client
        .c_move(
            STUDY_ROOT_MOVE,
            "NOBODY",
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, "2.25.404")]),
        )
        .await;
    assert_eq!(again.status(), Status::MOVE_DESTINATION_UNKNOWN);
    client.release().await;
}

/// Rights per project: a C-MOVE to a project's AE title sends that project's studies, as
/// the project; a study outside it is not there to send.
#[tokio::test]
async fn a_project_ae_title_sends_only_its_project() {
    let archive = archive().await;
    let received_as_is = SampleSeries::new(621, 2).with_patient("Identified^Ida", None);
    ingest(&archive, &received_as_is).await;
    let liver = archive.clone().for_project("liver-2026");
    let in_liver = SampleSeries::new(622, 2).with_patient("PSEUDO-1", None);
    ingest(&liver, &in_liver).await;
    let (destination, received) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let project = Project {
        ae_title: AeTitle::new("LIVER_2026").unwrap(),
        profile: Profile::from_json(r#"{"name": "liver-2026"}"#).unwrap(),
        keys: Keys::new(*b"a project secret of 32 bytes ok!").unwrap(),
        archive: liver,
        quarantine: None,
    };
    let scp = Scp::new(sending_to(&destination), Destination::Archive(archive)).with_route(
        AeTitle::new("LIVER_2026").unwrap(),
        Destination::Project(Arc::new(project)),
    );
    let addr = start_scp(scp).await;
    let mut client = viewer(addr, "LIVER_2026").await;

    // the identified study, by its UID: nothing to send
    let outside = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&received_as_is))
        .await;
    assert_eq!(outside.status(), Status::SUCCESS);
    assert_eq!(
        Retrieval::counts(&outside.last),
        [None, Some(0), Some(0), Some(0)]
    );
    assert!(received.lock().unwrap().is_empty());
    // nor by an instance of it
    let instance = keys(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, &received_as_is.study_uid()),
            (tags::SERIES_INSTANCE_UID, &received_as_is.series_uid()),
            (tags::SOP_INSTANCE_UID, &received_as_is.instance_uid(0)),
        ],
    );
    let outside = client.c_move(STUDY_ROOT_MOVE, "STORESCP", &instance).await;
    assert_eq!(
        Retrieval::counts(&outside.last),
        [None, Some(0), Some(0), Some(0)]
    );
    assert!(received.lock().unwrap().is_empty());

    let inside = client
        .c_move(STUDY_ROOT_MOVE, "STORESCP", &study(&in_liver))
        .await;
    assert_eq!(inside.status(), Status::SUCCESS);
    assert_eq!(
        Retrieval::counts(&inside.last),
        [None, Some(2), Some(0), Some(0)]
    );
    client.release().await;
    let received = received.lock().unwrap();
    assert_eq!(received.len(), 2);
    for store in received.iter() {
        assert_eq!(store.calling, "LIVER_2026");
        assert_eq!(
            value(&store.object(), tags::PATIENT_NAME).unwrap(),
            "PSEUDO-1"
        );
    }
}

/// PS3.7 9.1.4.1.7: another operation on the MOVE SOP class is Unrecognized operation.
#[tokio::test]
async fn another_operation_on_the_move_sop_class_is_0211() {
    let archive = archive().await;
    let (destination, _) = storage_scp(ALL, &[EXPLICIT_VR_LE], Peer::Answers(&[])).await;
    let addr = start(sending_to(&destination), archive).await;
    let mut client = viewer(addr, "PACSRS").await;
    let context = client.context(STUDY_ROOT_MOVE);
    for operation in [field::C_FIND_RQ, field::C_GET_RQ, field::C_STORE_RQ] {
        let id = client.message_id();
        let request = no_data(c_move_rq(id, STUDY_ROOT_MOVE, "STORESCP"))
            .with_us(tag::COMMAND_FIELD, operation);
        let rsp = client.response(context, &request, None).await;
        assert_eq!(rsp.status(), Ok(Status::UNRECOGNIZED_OPERATION));
    }
    client.release().await;
}
