//! pacs-rs as a Retrieve SCP for C-GET, driven by a bare dicom-ul client that receives on
//! the association it asks on (PS3.4 C.4.3.3, C.5.3, C.6.2; PS3.7 9.1.3, D.3.3.4): the
//! role selection, the counts and final statuses, the failures, who is answered and what a
//! project's AE title gives.

mod common;

use std::sync::Arc;

use common::*;
use dicom_core::{Tag, VR};
use dicom_dictionary_std::tags;
use pacs_archive::{Archive, Compression};
use pacs_core::real::RealSeries;
use pacs_deid::{Keys, Profile};
use pacs_dimse::command::{field, tag};
use pacs_dimse::query::STUDY_ROOT_GET;
use pacs_dimse::{AeTitle, Config, Destination, Project, Scp, Status, VERIFICATION};
use pacs_index::Index;
use pacs_store::Store;

/// A configuration that answers VIEWER.
fn answering() -> Config {
    let mut config = config();
    config.remotes = vec!["viewer=VIEWER@127.0.0.1:11112".parse().unwrap()];
    config
}

/// VIEWER, asking with C-GET and proposing to be the SCP of `storage`.
async fn viewer(addr: std::net::SocketAddr, called: &str, storage: &[&str]) -> Client {
    Client::connect_receiving(
        addr,
        "VIEWER",
        called,
        &[STUDY_ROOT_GET, VERIFICATION],
        storage,
        &[EXPLICIT_VR_LE, IMPLICIT_VR_LE],
    )
    .await
    .unwrap()
}

/// Doe's study of two series (seed 701 of two slices, 702 of three) and Roe's (703).
async fn two_studies(archive: &Archive) -> [RealSeries; 3] {
    let series = [
        RealSeries::new(701, 2).with_patient("Doe^Jane", None),
        RealSeries::new(702, 3)
            .in_study(701, 2)
            .with_patient("Doe^Jane", None),
        RealSeries::new(703, 1).with_patient("Roe^Richard", None),
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

fn study(series: &RealSeries) -> dicom_object::InMemDicomObject {
    keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &series.study_uid())])
}

fn uids(got: &[Got]) -> Vec<String> {
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
    uids
}

fn instances(series: &[&RealSeries]) -> Vec<String> {
    let mut uids: Vec<String> = series
        .iter()
        .flat_map(|one| (0..one.slices).map(|slice| one.instance_uid(slice)))
        .collect();
    uids.sort();
    uids
}

/// PS3.4 C.4.3.3.1: every instance of the study comes by C-STORE on the association of
/// the C-GET, on a context of its SOP class where the requester is the SCP; the Pending
/// responses count them, and the final one is Success. Stored compressed, an instance is
/// decoded for a requester that proposed uncompressed transfer syntaxes only.
#[tokio::test]
async fn a_study_comes_on_the_association_that_asked() {
    let archive = Archive::new(Store::memory(), Index::in_memory().await.unwrap())
        .with_compression(Compression::JpegLs);
    let [first, second, _] = two_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &study(&first), Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert!(retrieval.failed.is_empty());
    let pending: Vec<[Option<u16>; 4]> = retrieval.pending.iter().map(Retrieval::counts).collect();
    assert_eq!(
        pending,
        [
            [Some(4), Some(1), Some(0), Some(0)],
            [Some(3), Some(2), Some(0), Some(0)],
            [Some(2), Some(3), Some(0), Some(0)],
            [Some(1), Some(4), Some(0), Some(0)],
        ]
    );
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [None, Some(5), Some(0), Some(0)]
    );
    assert_eq!(
        retrieval.last.ui(tag::AFFECTED_SOP_CLASS_UID),
        Ok(STUDY_ROOT_GET)
    );
    assert_eq!(uids(&got), instances(&[&first, &second]));
    let get_id = retrieval
        .last
        .us(tag::MESSAGE_ID_BEING_RESPONDED_TO)
        .unwrap();
    let mut ids = Vec::new();
    for store in &got {
        let command = &store.command;
        assert_eq!(command.ui(tag::AFFECTED_SOP_CLASS_UID), Ok(CT));
        // not a move: no Move Originator (PS3.7 9.3.1.1)
        assert!(command.get(tag::MOVE_ORIGINATOR_AE_TITLE).is_none());
        assert!(command.get(tag::MOVE_ORIGINATOR_MESSAGE_ID).is_none());
        ids.push(command.message_id().unwrap());
        // decoded: the pixels are the ones that were stored
        let object = store.object();
        let slice = object
            .get(tags::INSTANCE_NUMBER)
            .unwrap()
            .to_int::<u32>()
            .unwrap()
            - 1;
        let series = if value(&object, tags::SERIES_INSTANCE_UID).unwrap() == first.series_uid() {
            &first
        } else {
            &second
        };
        let pixels: Vec<u16> = object
            .get(tags::PIXEL_DATA)
            .unwrap()
            .to_bytes()
            .unwrap()
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        assert_eq!(pixels, series.pixels(slice, 0));
    }
    // each sub-operation has a Message ID of its own, none the C-GET's
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 5);
    assert!(!ids.contains(&get_id));
    // the association goes on
    assert_eq!(client.echo().await.status(), Ok(Status::SUCCESS));
    client.release().await;
}

/// PS3.4 C.6.2.1.5: the series of one study and the instances of one series are
/// retrieved by their Unique Keys; what is not there is Success with nothing sent.
#[tokio::test]
async fn series_and_instances_are_selected_by_their_unique_keys() {
    let archive = archive().await;
    let [first, second, roe] = two_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let study_uid = first.study_uid();
    let one_series = keys(
        "SERIES",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &second.series_uid()),
        ],
    );
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &one_series, Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(uids(&got), instances(&[&second]));

    let two = format!("{}\\{}", second.instance_uid(2), second.instance_uid(0));
    let two_instances = keys(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, &study_uid),
            (tags::SERIES_INSTANCE_UID, &second.series_uid()),
            (tags::SOP_INSTANCE_UID, &two),
        ],
    );
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &two_instances, Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(uids(&got), [second.instance_uid(0), second.instance_uid(2)]);

    let both = format!("{}\\{}", study_uid, roe.study_uid());
    let (retrieval, got) = client
        .c_get(
            STUDY_ROOT_GET,
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &both)]),
            Answering::With(&[]),
        )
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(uids(&got), instances(&[&first, &second, &roe]));

    let (retrieval, got) = client
        .c_get(
            STUDY_ROOT_GET,
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, "2.25.404")]),
            Answering::With(&[]),
        )
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [None, Some(0), Some(0), Some(0)]
    );
    assert!(got.is_empty() && retrieval.pending.is_empty());
    client.release().await;
}

/// PS3.7 D.3.3.4 and PS3.4 C.4.3.3.1: without the SCP role for an instance's SOP class the
/// requester cannot be sent it, and the sub-operation fails. With the role proposed for no
/// class the retrieval is a Failure, every instance named; with it for some, a Warning.
#[tokio::test]
async fn an_instance_the_requester_did_not_take_the_scp_role_for_fails() {
    let archive = archive().await;
    let ct = RealSeries::new(711, 2);
    ingest(&archive, &ct).await;
    // the same study's second series is an MR: another Storage SOP class
    let mr = RealSeries::mr(712, 2).in_study(711, 2);
    ingest(&archive, &mr).await;
    let addr = start(answering(), archive).await;

    // CT and MR contexts proposed, and no role selection: the default roles hold, the
    // requester is the SCU of both
    let mut plain = Client::connect(
        addr,
        "VIEWER",
        "PACSRS",
        &[STUDY_ROOT_GET, CT, MR],
        &[EXPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let (retrieval, got) = plain
        .c_get(STUDY_ROOT_GET, &study(&ct), Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::UNABLE_TO_PERFORM_SUB_OPERATIONS);
    assert!(got.is_empty());
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [None, Some(0), Some(4), Some(0)]
    );
    let mut failed = retrieval.failed.clone();
    failed.sort();
    assert_eq!(failed, instances(&[&ct, &mr]));
    plain.release().await;

    // the SCP role for CT only: the MR instances fail, the CT ones come
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &study(&ct), Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUB_OPERATIONS_WARNING);
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [None, Some(2), Some(2), Some(0)]
    );
    assert_eq!(uids(&got), instances(&[&ct]));
    let mut failed = retrieval.failed.clone();
    failed.sort();
    assert_eq!(failed, instances(&[&mr]));
    for response in &retrieval.pending {
        let [remaining, completed, failed, warning] =
            Retrieval::counts(response).map(Option::unwrap);
        assert_eq!(remaining + completed + failed + warning, 4);
    }
    client.release().await;

    // both roles taken, and the requester refuses one instance and warns for another
    let mut client = viewer(addr, "PACSRS", &[CT, MR]).await;
    let (retrieval, got) = client
        .c_get(
            STUDY_ROOT_GET,
            &study(&ct),
            Answering::With(&[0xA700, 0xB000]),
        )
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUB_OPERATIONS_WARNING);
    assert_eq!(got.len(), 4);
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [None, Some(2), Some(1), Some(1)]
    );
    assert_eq!(retrieval.failed.len(), 1);
    client.release().await;
}

/// PS3.4 C.4.3.3.1: a C-CANCEL-GET-RQ interrupts the sub-operations. It is sent here
/// before the answer to the second instance, which is finished all the same; the rest is
/// not sent, and the response is Cancel with what remained.
#[tokio::test]
async fn a_cancel_stops_the_sub_operations() {
    let archive = archive().await;
    let [first, ..] = two_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &study(&first), Answering::CancellingAt(2))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::CANCEL);
    assert_eq!(got.len(), 2);
    assert_eq!(
        Retrieval::counts(&retrieval.last),
        [Some(3), Some(2), Some(0), Some(0)]
    );
    assert!(retrieval.failed.is_empty());
    assert_eq!(retrieval.last.command_field(), Ok(field::C_GET_RSP));
    // the association is whole: the same study again, to its end
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &study(&first), Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(got.len(), 5);
    // a cancel that comes with the answer to the last instance has nothing left to stop
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &study(&first), Answering::CancellingAt(5))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(got.len(), 5);
    client.release().await;
}

/// The failures before any sub-operation: a caller that is not configured (0124), an
/// Identifier that is not one of the model (A900), cannot be read (C000) or is not there.
#[tokio::test]
async fn a_request_that_cannot_be_served_is_a_failure() {
    let archive = archive().await;
    let [first, ..] = two_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut stranger = Client::connect_receiving(
        addr,
        "STRANGER",
        "PACSRS",
        &[STUDY_ROOT_GET],
        &[CT],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let (retrieval, got) = stranger
        .c_get(STUDY_ROOT_GET, &study(&first), Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::NOT_AUTHORIZED);
    assert!(got.is_empty());
    assert!(
        retrieval
            .last
            .text(tag::ERROR_COMMENT)
            .unwrap()
            .contains("STRANGER")
    );
    stranger.release().await;

    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    for (identifier, offending) in [
        (
            keys("PATIENT", &[(tags::STUDY_INSTANCE_UID, &first.study_uid())]),
            tags::QUERY_RETRIEVE_LEVEL,
        ),
        (keys("STUDY", &[]), tags::STUDY_INSTANCE_UID),
        (
            keys("SERIES", &[(tags::STUDY_INSTANCE_UID, &first.study_uid())]),
            tags::SERIES_INSTANCE_UID,
        ),
    ] {
        let (retrieval, got) = client
            .c_get(STUDY_ROOT_GET, &identifier, Answering::With(&[]))
            .await
            .unwrap();
        assert_eq!(
            retrieval.status(),
            Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS
        );
        assert!(got.is_empty());
        let offending_element = retrieval.last.get(tag::OFFENDING_ELEMENT).map(|bytes| {
            Tag(
                u16::from_le_bytes([bytes[0], bytes[1]]),
                u16::from_le_bytes([bytes[2], bytes[3]]),
            )
        });
        assert_eq!(offending_element, Some(offending));
    }
    // no Identifier at all
    let id = client.message_id();
    let context = client.context(STUDY_ROOT_GET);
    let rsp = client
        .response(context, &no_data(c_get_rq(id, STUDY_ROOT_GET)), None)
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS)
    );
    assert_eq!(rsp.command_field(), Ok(field::C_GET_RSP));
    // another operation on the GET SOP class
    for operation in [field::C_FIND_RQ, field::C_MOVE_RQ, field::C_STORE_RQ] {
        let id = client.message_id();
        let request = no_data(c_get_rq(id, STUDY_ROOT_GET)).with_us(tag::COMMAND_FIELD, operation);
        let rsp = client.response(context, &request, None).await;
        assert_eq!(rsp.status(), Ok(Status::UNRECOGNIZED_OPERATION));
    }
    client.release().await;
}

/// While an instance is with the requester, the only messages it may send are the
/// C-STORE-RSP and a cancel: another request is a protocol error, and the association is
/// aborted.
#[tokio::test]
async fn another_request_instead_of_the_c_store_rsp_aborts() {
    let archive = archive().await;
    let [first, ..] = two_studies(&archive).await;
    let addr = start(answering(), archive).await;
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let ended = client
        .c_get(
            STUDY_ROOT_GET,
            &study(&first),
            Answering::AskingSomethingElse,
        )
        .await;
    assert!(ended.is_none(), "the association went on");
}

/// Rights per project: a C-GET to a project's AE title gives that project's studies and
/// no other.
#[tokio::test]
async fn a_project_ae_title_gives_only_its_project() {
    let archive = archive().await;
    let identified = RealSeries::new(721, 2).with_patient("Identified^Ida", None);
    ingest(&archive, &identified).await;
    let liver = archive.clone().for_project("liver-2026");
    let in_liver = RealSeries::new(722, 2).with_patient("PSEUDO-1", None);
    ingest(&liver, &in_liver).await;
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
    let mut client = viewer(addr, "LIVER_2026", &[CT]).await;
    let both = format!("{}\\{}", identified.study_uid(), in_liver.study_uid());
    let (retrieval, got) = client
        .c_get(
            STUDY_ROOT_GET,
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &both)]),
            Answering::With(&[]),
        )
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert_eq!(uids(&got), instances(&[&in_liver]));
    for store in &got {
        assert_eq!(
            value(&store.object(), tags::PATIENT_NAME).unwrap(),
            "PSEUDO-1"
        );
    }
    // an instance of the identified study, by its three UIDs: not there
    let instance = keys(
        "IMAGE",
        &[
            (tags::STUDY_INSTANCE_UID, &identified.study_uid()),
            (tags::SERIES_INSTANCE_UID, &identified.series_uid()),
            (tags::SOP_INSTANCE_UID, &identified.instance_uid(0)),
        ],
    );
    let (retrieval, got) = client
        .c_get(STUDY_ROOT_GET, &instance, Answering::With(&[]))
        .await
        .unwrap();
    assert_eq!(retrieval.status(), Status::SUCCESS);
    assert!(got.is_empty());
    client.release().await;

    // under the server's own title, both
    let mut client = viewer(addr, "PACSRS", &[CT]).await;
    let (_, got) = client
        .c_get(
            STUDY_ROOT_GET,
            &keys("STUDY", &[(tags::STUDY_INSTANCE_UID, &both)]),
            Answering::With(&[]),
        )
        .await
        .unwrap();
    assert_eq!(uids(&got), instances(&[&identified, &in_liver]));
    client.release().await;
}
