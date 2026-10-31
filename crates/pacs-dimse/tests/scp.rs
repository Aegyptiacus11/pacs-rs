//! pacs-rs as an SCP, driven by a bare dicom-ul client: association acceptance (PS3.8),
//! Verification (PS3.4 Annex A, PS3.7 9.1.5) and Storage (PS3.4 Annex B, PS3.7 9.1.1).

mod common;

use std::time::Duration;

use common::*;
use dicom_ul::association::Error as UlError;
use dicom_ul::pdu::{
    AssociationRJ, AssociationRJResult, AssociationRJServiceProviderPresentationReason,
    AssociationRJServiceUserReason, AssociationRJSource,
};
use pacs_core::synthetic::SyntheticSeries;
use pacs_dimse::command::{field, tag};
use pacs_dimse::message::Incoming;
use pacs_dimse::{AeTitle, RemoteAe, Status, VERIFICATION};

fn rejection(result: Result<Client, UlError>) -> AssociationRJ {
    match result {
        Err(UlError::Rejected { association_rj, .. }) => association_rj,
        Err(e) => panic!("expected a rejection, got {e}"),
        Ok(_) => panic!("expected a rejection, the association was accepted"),
    }
}

/// PS3.7 9.1.5: a C-ECHO-RQ on the Verification context is answered Success.
#[tokio::test]
async fn c_echo_is_answered_success() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(
        addr,
        "ECHOSCU",
        "PACSRS",
        &[VERIFICATION],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let rsp = client.echo().await;
    assert_eq!(rsp.command_field(), Ok(field::C_ECHO_RSP));
    assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    client.release().await;
}

/// pacs-rs's own SCU against its SCP: one C-ECHO, released.
#[tokio::test]
async fn the_echo_scu_reaches_an_scp() {
    let addr = start(config(), archive().await).await;
    let remote: RemoteAe = format!("self=PACSRS@127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let calling = AeTitle::new("TESTSCU").unwrap();
    pacs_dimse::echo(&remote, &calling, Duration::from_secs(5))
        .await
        .unwrap();
}

/// The SCU reports a refused association, not a hang.
#[tokio::test]
async fn the_echo_scu_reports_a_rejection() {
    let addr = start(config(), archive().await).await;
    let remote: RemoteAe = format!("x=SOMEONE_ELSE@127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let error = pacs_dimse::echo(&remote, &AeTitle::new("T").unwrap(), Duration::from_secs(5))
        .await
        .unwrap_err();
    assert!(
        matches!(error, pacs_dimse::EchoError::Association { .. }),
        "{error}"
    );
}

/// The SCU gives up after its timeout on a peer that never answers.
#[tokio::test]
async fn the_echo_scu_times_out() {
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = silent.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _held = silent.accept().await;
        std::future::pending::<()>().await;
    });
    let remote: RemoteAe = format!("x=X@127.0.0.1:{port}").parse().unwrap();
    let started = std::time::Instant::now();
    let result = pacs_dimse::echo(
        &remote,
        &AeTitle::new("T").unwrap(),
        Duration::from_millis(500),
    )
    .await;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// PS3.8 Table 9-21, reason 7: a called AE title pacs-rs does not have is refused.
#[tokio::test]
async fn an_unknown_called_ae_title_is_rejected() {
    let addr = start(config(), archive().await).await;
    let rj = rejection(
        Client::connect(
            addr,
            "ECHOSCU",
            "NOT_US",
            &[VERIFICATION],
            &[IMPLICIT_VR_LE],
        )
        .await,
    );
    assert_eq!(rj.result, AssociationRJResult::Permanent);
    assert_eq!(
        rj.source,
        AssociationRJSource::ServiceUser(
            AssociationRJServiceUserReason::CalledAETitleNotRecognized
        )
    );
}

/// Every accepted AE title answers, on one port and one set of presentation contexts.
#[tokio::test]
async fn extra_ae_titles_are_accepted_on_the_same_port() {
    let mut config = config();
    config.extra_ae_titles = vec![AeTitle::new("RESEARCH").unwrap()];
    let addr = start(config, archive().await).await;
    for called in ["PACSRS", "RESEARCH"] {
        let mut client = Client::connect(
            addr,
            "ECHOSCU",
            called,
            &[VERIFICATION, CT],
            &[IMPLICIT_VR_LE],
        )
        .await
        .unwrap();
        client.context(CT);
        assert_eq!(client.echo().await.status(), Ok(Status::SUCCESS));
        client.release().await;
    }
}

/// PS3.8 Table 9-21, reason 3: with known callers only, an unlisted calling AE is refused
/// and a listed one accepted.
#[tokio::test]
async fn known_callers_only_refuses_strangers() {
    let mut config = config();
    config.known_callers_only = true;
    config.remotes = vec!["h=HOSPITAL@10.0.0.5:104".parse().unwrap()];
    let addr = start(config, archive().await).await;
    let rj = rejection(
        Client::connect(
            addr,
            "STRANGER",
            "PACSRS",
            &[VERIFICATION],
            &[IMPLICIT_VR_LE],
        )
        .await,
    );
    assert_eq!(
        rj.source,
        AssociationRJSource::ServiceUser(
            AssociationRJServiceUserReason::CallingAETitleNotRecognized
        )
    );
    Client::connect(
        addr,
        "HOSPITAL",
        "PACSRS",
        &[VERIFICATION],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap()
    .release()
    .await;
}

/// PS3.8 9.3.4, Table 9-21: over the association limit, rejected-transient, service
/// provider (presentation), local-limit-exceeded; the next one after a release is served.
#[tokio::test]
async fn over_the_limit_an_association_is_rejected_transient() {
    let mut config = config();
    config.max_associations = 1;
    let addr = start(config, archive().await).await;
    let first = Client::connect(addr, "A", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let rj =
        rejection(Client::connect(addr, "B", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE]).await);
    assert_eq!(rj.result, AssociationRJResult::Transient);
    assert_eq!(
        rj.source,
        AssociationRJSource::ServiceProviderPresentation(
            AssociationRJServiceProviderPresentationReason::LocalLimitExceeded
        )
    );
    first.release().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    Client::connect(addr, "B", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap()
        .release()
        .await;
}

/// PS3.8 9.3.3.2: an abstract syntax pacs-rs does not provide is refused in its
/// presentation context, the others accepted (dicom-ul lists the accepted ones only).
#[tokio::test]
async fn only_verification_and_storage_contexts_are_accepted() {
    let addr = start(config(), archive().await).await;
    let study_root_find = "1.2.840.10008.5.1.4.1.2.2.1";
    let client = Client::connect(
        addr,
        "A",
        "PACSRS",
        &[VERIFICATION, CT, study_root_find],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    let reasons: Vec<_> = client
        .association
        .presentation_contexts()
        .iter()
        .map(|pc| (pc.abstract_syntax.clone(), pc.reason.clone()))
        .collect();
    use dicom_ul::pdu::PresentationContextResultReason::Acceptance;
    assert_eq!(
        reasons,
        [
            (VERIFICATION.to_owned(), Acceptance),
            (CT.to_owned(), Acceptance)
        ]
    );
    client.release().await;
}

/// PS3.4 B.2.2: Success, and the instance is in the archive, as a REST upload would be.
#[tokio::test]
async fn c_store_stores_through_the_archive() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let series = SyntheticSeries::new(61, 2);
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    for slice in 0..2 {
        let rsp = client
            .store_as(
                CT,
                CT,
                &series.instance_uid(slice),
                &data_set(&series.slice_bytes(slice)),
            )
            .await;
        assert_eq!(rsp.command_field(), Ok(field::C_STORE_RSP));
        assert_eq!(rsp.status(), Ok(Status::SUCCESS));
        assert_eq!(
            rsp.ui(tag::AFFECTED_SOP_INSTANCE_UID),
            Ok(series.instance_uid(slice).as_str())
        );
    }
    client.release().await;
    let entry = archive
        .index()
        .instance(
            &series.study_uid(),
            &series.series_uid(),
            &series.instance_uid(1),
        )
        .await
        .unwrap()
        .expect("the instance is indexed");
    let file = archive.file(&entry).await.unwrap();
    assert!(file.ends_with(&data_set(&series.slice_bytes(1))));
}

/// The data set is stored in the transfer syntax negotiated for it, here Implicit VR.
#[tokio::test]
async fn c_store_keeps_the_negotiated_transfer_syntax() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let series = SyntheticSeries::new(62, 1);
    let mut object = pacs_core::Instance::from_bytes(&series.slice_bytes(0))
        .unwrap()
        .object;
    object.meta_mut().transfer_syntax = IMPLICIT_VR_LE.to_owned();
    object.meta_mut().update_information_group_length();
    let mut implicit = Vec::new();
    object.write_all(&mut implicit).unwrap();
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(CT, CT, &series.instance_uid(0), &data_set(&implicit))
        .await;
    assert_eq!(
        rsp.status(),
        Ok(Status::SUCCESS),
        "{:?}",
        rsp.text(tag::ERROR_COMMENT)
    );
    client.release().await;
    let entry = archive
        .index()
        .rest_instance_by_uid(&series.instance_uid(0))
        .await
        .unwrap()
        .unwrap();
    let stored = archive.file_at(&entry.object_key).await.unwrap();
    let instance = pacs_core::Instance::from_bytes(&stored).unwrap();
    assert_eq!(instance.key.transfer_syntax_uid, IMPLICIT_VR_LE);
}

/// An instance sent again is Success (it is stored: PS3.4 B.2.2), and the first copy is
/// kept, as `AlreadyStored` for a REST upload.
#[tokio::test]
async fn an_instance_sent_twice_is_success_and_kept_once() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let series = SyntheticSeries::new(63, 1);
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let data = data_set(&series.slice_bytes(0));
    for _ in 0..2 {
        let rsp = client
            .store_as(CT, CT, &series.instance_uid(0), &data)
            .await;
        assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    }
    client.release().await;
    let study = archive
        .index()
        .rest_instance_by_uid(&series.instance_uid(0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(study.sop_instance_uid, series.instance_uid(0));
}

/// PS3.4 Table B.2-1, A9xx: the data set is not of the SOP class it is sent as.
#[tokio::test]
async fn a_data_set_of_another_sop_class_is_a900() {
    let archive = archive().await;
    let addr = start(config(), archive.clone()).await;
    let series = SyntheticSeries::new(64, 1);
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[MR], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            MR,
            MR,
            &series.instance_uid(0),
            &data_set(&series.slice_bytes(0)),
        )
        .await;
    assert_eq!(rsp.status(), Ok(Status::DATA_SET_DOES_NOT_MATCH_SOP_CLASS));
    assert!(rsp.text(tag::ERROR_COMMENT).is_some());
    client.release().await;
    assert!(
        archive
            .index()
            .rest_instance_by_uid(&series.instance_uid(0))
            .await
            .unwrap()
            .is_none()
    );
}

/// PS3.4 Table B.2-1, Cxxx: SOP Instance UID is not the request's Affected SOP Instance UID.
#[tokio::test]
async fn a_data_set_with_another_instance_uid_is_refused() {
    let addr = start(config(), archive().await).await;
    let series = SyntheticSeries::new(65, 1);
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(CT, CT, "2.25.12345", &data_set(&series.slice_bytes(0)))
        .await;
    assert_eq!(rsp.status(), Ok(Status::CANNOT_UNDERSTAND_INSTANCE_UID));
    client.release().await;
}

/// PS3.4 Table B.2-1, Cxxx: a data set that cannot be parsed.
#[tokio::test]
async fn an_unreadable_data_set_is_cannot_understand() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            CT,
            CT,
            "2.25.1",
            &[0x08, 0x00, 0x16, 0x00, b'U', b'I', 200, 0],
        )
        .await;
    assert_eq!(rsp.status(), Ok(Status::CANNOT_UNDERSTAND));
    // the association goes on
    let rsp = client.store_as(CT, CT, "2.25.1", &[0xFF; 4]).await;
    assert_eq!(
        rsp.status().map(|s| s.class()),
        Ok(pacs_dimse::StatusClass::Failure)
    );
    client.release().await;
}

/// Cxxx: the UIDs contradict the archive, an instance already stored in another series.
#[tokio::test]
async fn a_hierarchy_conflict_is_refused() {
    let addr = start(config(), archive().await).await;
    let first = SyntheticSeries::new(66, 1);
    let mut client = Client::connect(addr, "STORESCU", "PACSRS", &[CT], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            CT,
            CT,
            &first.instance_uid(0),
            &data_set(&first.slice_bytes(0)),
        )
        .await;
    assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    // the same SOP Instance UID, put in another series
    let mut moved = pacs_core::Instance::from_bytes(&first.slice_bytes(0))
        .unwrap()
        .object;
    moved.put(dicom_object_element(
        dicom_dictionary_std::tags::SERIES_INSTANCE_UID,
        "2.25.6600000999",
    ));
    let mut file = Vec::new();
    moved.write_all(&mut file).unwrap();
    let rsp = client
        .store_as(CT, CT, &first.instance_uid(0), &data_set(&file))
        .await;
    assert_eq!(rsp.status(), Ok(Status::CANNOT_UNDERSTAND_CONFLICT));
    client.release().await;
}

fn dicom_object_element(tag: dicom_core::Tag, uid: &str) -> dicom_object::mem::InMemElement {
    dicom_core::DataElement::new(
        tag,
        dicom_core::VR::UI,
        dicom_core::PrimitiveValue::from(uid),
    )
}

/// A data set over the limit is read through and refused, Out of resources (A7xx); the
/// association stays usable.
#[tokio::test]
async fn a_data_set_over_the_limit_is_refused_out_of_resources() {
    let mut config = config();
    config.max_data_set_bytes = 100;
    let addr = start(config, archive().await).await;
    let series = SyntheticSeries::new(67, 1);
    let mut client = Client::connect(
        addr,
        "STORESCU",
        "PACSRS",
        &[VERIFICATION, CT],
        &[EXPLICIT_VR_LE],
    )
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
    assert_eq!(rsp.status(), Ok(Status::OUT_OF_RESOURCES_TOO_LARGE));
    assert_eq!(client.echo().await.status(), Ok(Status::SUCCESS));
    client.release().await;
}

/// PS3.7 9.1.5.1.4, 9.1.1.1.9: an Affected SOP Class UID pacs-rs has no such class for is
/// Refused: SOP Class not supported (0122H).
#[tokio::test]
async fn an_unsupported_sop_class_is_0122() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(addr, "A", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let context = client.context(VERIFICATION);
    let rq =
        no_data(c_store_rq(5, "1.2.3.4", "2.25.1")).with_us(tag::COMMAND_FIELD, field::C_ECHO_RQ);
    let rsp = client.response(context, &rq, None).await;
    assert_eq!(rsp.status(), Ok(Status::SOP_CLASS_NOT_SUPPORTED));
    client.release().await;
}

/// PS3.7 9.1.1.1.9 Mistyped argument (0212H): the Affected SOP Class UID is not the one
/// agreed for the presentation context the request came on.
#[tokio::test]
async fn a_sop_class_not_agreed_for_the_context_is_0212() {
    let addr = start(config(), archive().await).await;
    let series = SyntheticSeries::new(68, 1);
    let mut client = Client::connect(addr, "A", "PACSRS", &[CT, MR], &[EXPLICIT_VR_LE])
        .await
        .unwrap();
    let rsp = client
        .store_as(
            MR,
            CT,
            &series.instance_uid(0),
            &data_set(&series.slice_bytes(0)),
        )
        .await;
    assert_eq!(rsp.status(), Ok(Status::MISTYPED_ARGUMENT));
    client.release().await;
}

/// PS3.7 9.1.5.1.4: a C-ECHO for a SOP class that has no C-ECHO is Unrecognized operation
/// (0211H); so is any request pacs-rs does not provide, such as a C-FIND.
#[tokio::test]
async fn an_operation_the_sop_class_lacks_is_0211() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(addr, "A", "PACSRS", &[VERIFICATION, CT], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let ct = client.context(CT);
    let echo_on_ct = pacs_dimse::command::c_echo_rq(3).with_ui(tag::AFFECTED_SOP_CLASS_UID, CT);
    let rsp = client.response(ct, &echo_on_ct, None).await;
    assert_eq!(rsp.command_field(), Ok(field::C_ECHO_RSP));
    assert_eq!(rsp.status(), Ok(Status::UNRECOGNIZED_OPERATION));
    let verification = client.context(VERIFICATION);
    let find = no_data(c_store_rq(4, VERIFICATION, "2.25.1"))
        .with_us(tag::COMMAND_FIELD, field::C_FIND_RQ);
    let rsp = client.response(verification, &find, None).await;
    assert_eq!(rsp.command_field(), Ok(0x8020));
    assert_eq!(rsp.status(), Ok(Status::UNRECOGNIZED_OPERATION));
    client.release().await;
}

/// PS3.8 9.3.5: P-DATA on a presentation context that was not accepted ends the
/// association with A-ABORT.
#[tokio::test]
async fn a_message_on_a_refused_context_aborts() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(
        addr,
        "A",
        "PACSRS",
        &[VERIFICATION, "1.2.840.10008.5.1.4.1.2.2.1"],
        &[IMPLICIT_VR_LE],
    )
    .await
    .unwrap();
    // proposed second, so with the odd ID 3 (PS3.8 9.3.2.2); not accepted, so not listed
    let refused = 3;
    assert!(
        client
            .association
            .presentation_contexts()
            .iter()
            .all(|pc| pc.id != refused)
    );
    let incoming = client
        .exchange(refused, &pacs_dimse::command::c_echo_rq(2), None)
        .await;
    assert!(matches!(incoming, Incoming::Aborted), "{incoming:?}");
}

/// A response sent to pacs-rs, which has asked nothing, is a protocol error: A-ABORT.
#[tokio::test]
async fn an_unsolicited_response_aborts() {
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(addr, "A", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let context = client.context(VERIFICATION);
    let rsp =
        pacs_dimse::command::response(&pacs_dimse::command::c_echo_rq(1), Status::SUCCESS, None);
    let incoming = client.exchange(context, &rsp, None).await;
    assert!(matches!(incoming, Incoming::Aborted), "{incoming:?}");
}

/// An idle association is aborted once the timeout passes.
#[tokio::test]
async fn an_idle_association_is_aborted_after_the_timeout() {
    let mut config = config();
    config.timeout = Duration::from_millis(300);
    let addr = start(config, archive().await).await;
    let mut client = Client::connect(addr, "A", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let incoming = pacs_dimse::message::receive(&mut client.association, 0).await;
    assert!(matches!(incoming, Ok(Incoming::Aborted)), "{incoming:?}");
}

/// A command split over several PDVs is put back together (PS3.8 E.2).
#[tokio::test]
async fn a_command_in_fragments_is_reassembled() {
    use dicom_ul::Pdu;
    use dicom_ul::pdu::{PDataValue, PDataValueType};
    let addr = start(config(), archive().await).await;
    let mut client = Client::connect(addr, "A", "PACSRS", &[VERIFICATION], &[IMPLICIT_VR_LE])
        .await
        .unwrap();
    let context = client.context(VERIFICATION);
    let bytes = pacs_dimse::command::c_echo_rq(11).encode();
    let (a, b) = bytes.split_at(10);
    for (part, is_last) in [(a, false), (b, true)] {
        client
            .association
            .send(&Pdu::PData {
                data: vec![PDataValue {
                    presentation_context_id: context,
                    value_type: PDataValueType::Command,
                    is_last,
                    data: part.to_vec(),
                }],
            })
            .await
            .unwrap();
    }
    match pacs_dimse::message::receive(&mut client.association, 0)
        .await
        .unwrap()
    {
        Incoming::Message(m) => {
            assert_eq!(m.command.us(tag::MESSAGE_ID_BEING_RESPONDED_TO), Ok(11));
            assert_eq!(m.command.status(), Ok(Status::SUCCESS));
        }
        other => panic!("{other:?}"),
    }
    client.release().await;
}
