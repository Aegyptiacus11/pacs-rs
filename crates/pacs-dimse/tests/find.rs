//! The C-FIND SCU against a scripted SCP that answers whatever a test needs: Pending
//! responses, the final statuses of PS3.4 Table C.4-1, C-CANCEL handling (C.4.1.2.1), and
//! responses PS3.7 9.1.2 does not allow.

mod common;

use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::scripted::{Act, Seen, ended, response};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use dicom_ul::ServerAssociationOptions;
use pacs_dimse::command::{field, tag};
use pacs_dimse::query::STUDY_ROOT_FIND;
use pacs_dimse::{AeTitle, FindError, RemoteAe, Status, StudyQuery, find_studies};

const EXPLICIT: &str = "1.2.840.10008.1.2.1";
const IMPLICIT: &str = "1.2.840.10008.1.2";

/// One thing the scripted SCP does after the C-FIND-RQ.
enum Step {
    /// A C-FIND-RSP with this status and, for Pending, this Identifier.
    Respond(u16, Option<InMemDicomObject>),
    /// A final response with an Error Comment.
    Fail(u16, &'static str),
    /// Read the next message, which must be the C-CANCEL-FIND-RQ.
    ExpectCancel,
    /// A response to another Message ID.
    WrongMessageId,
    /// Say nothing more.
    Silence,
}

fn study(uid: &str, patient_id: &str) -> InMemDicomObject {
    let mut o = InMemDicomObject::new_empty();
    o.put(DataElement::new(
        tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        PrimitiveValue::from("STUDY"),
    ));
    o.put(DataElement::new(
        tags::PATIENT_ID,
        VR::LO,
        PrimitiveValue::from(patient_id),
    ));
    o.put(DataElement::new(
        tags::STUDY_INSTANCE_UID,
        VR::UI,
        PrimitiveValue::from(uid),
    ));
    o
}

/// Start the scripted SCP, accepting `transfer_syntaxes` for Study Root FIND.
async fn scripted(
    script: Vec<Step>,
    transfer_syntaxes: &'static [&'static str],
) -> (RemoteAe, Arc<Mutex<Seen>>) {
    common::scripted::serve("QRSCP", STUDY_ROOT_FIND, transfer_syntaxes, move |id| {
        let rsp = |status, id| response(STUDY_ROOT_FIND, field::C_FIND_RSP, id, status);
        script
            .into_iter()
            .map(|step| match step {
                Step::Respond(status, identifier) => Act::Send(rsp(status, id), identifier),
                Step::Fail(status, comment) => {
                    Act::Send(rsp(status, id).with_text(tag::ERROR_COMMENT, comment), None)
                }
                Step::ExpectCancel => Act::ExpectCancel,
                Step::WrongMessageId => Act::Send(rsp(0x0000, id + 1), None),
                Step::Silence => Act::Silence,
            })
            .collect()
    })
    .await
}

async fn find(
    remote: &RemoteAe,
    query: &StudyQuery,
    stop_after: Option<usize>,
) -> (Result<pacs_dimse::FindOutcome, FindError>, Vec<String>) {
    let mut found = Vec::new();
    let result = find_studies(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        query,
        Duration::from_millis(1500),
        |m| {
            let uid = m
                .element(tags::STUDY_INSTANCE_UID)
                .unwrap()
                .to_str()
                .unwrap();
            found.push(uid.trim_end_matches('\0').to_owned());
            match stop_after {
                Some(n) if found.len() >= n => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        },
    )
    .await;
    (result, found)
}

fn by_patient(id: &str) -> StudyQuery {
    StudyQuery {
        patient_id: Some(id.into()),
        ..Default::default()
    }
}

/// PS3.7 Table 9.3-3 as received, and the Identifier in the negotiated transfer syntax:
/// Explicit VR Little Endian, proposed first.
#[tokio::test]
async fn the_request_is_a_study_root_c_find() {
    let (remote, seen) = scripted(vec![Step::Respond(0x0000, None)], &[EXPLICIT, IMPLICIT]).await;
    let (result, _) = find(&remote, &by_patient("P1"), None).await;
    assert!(result.unwrap().complete());
    let seen = seen.lock().unwrap();
    let rq = seen.request.as_ref().unwrap();
    assert_eq!(rq.command_field(), Ok(field::C_FIND_RQ));
    assert_eq!(rq.ui(tag::AFFECTED_SOP_CLASS_UID), Ok(STUDY_ROOT_FIND));
    assert_eq!(seen.transfer_syntax, EXPLICIT);
    let id = seen.identifier.as_ref().unwrap();
    assert_eq!(
        id.element(tags::PATIENT_ID)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "P1"
    );
    assert_eq!(
        id.element(tags::QUERY_RETRIEVE_LEVEL)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "STUDY"
    );
}

/// An SCP that takes Implicit VR Little Endian only gets the Identifier in it.
#[tokio::test]
async fn the_identifier_follows_the_negotiated_transfer_syntax() {
    let (remote, seen) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::Respond(0x0000, None),
        ],
        &[IMPLICIT],
    )
    .await;
    let (result, found) = find(&remote, &by_patient("P1"), None).await;
    assert_eq!(result.unwrap().matches, 1);
    assert_eq!(found, ["2.25.1"]);
    assert_eq!(seen.lock().unwrap().transfer_syntax, IMPLICIT);
}

/// C.4.1.2.1: each Pending response is a match, handed over in order; Success ends them;
/// the association is released.
#[tokio::test]
async fn pending_responses_are_matches_until_success() {
    let (remote, seen) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::Respond(0xFF00, Some(study("2.25.2", "P1"))),
            Step::Respond(0x0000, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let (result, found) = find(&remote, &by_patient("P1"), None).await;
    let outcome = result.unwrap();
    assert_eq!(found, ["2.25.1", "2.25.2"]);
    assert_eq!(
        (outcome.status, outcome.matches, outcome.cancelled),
        (Status::SUCCESS, 2, false)
    );
    assert_eq!(ended(&seen).await, Some("release"));
}

/// Table C.4-1, FF01: still a match, with Optional Keys unsupported, so worth knowing.
#[tokio::test]
async fn ff01_is_a_match_with_optional_keys_unsupported() {
    let (remote, _) = scripted(
        vec![
            Step::Respond(0xFF01, Some(study("2.25.1", "P1"))),
            Step::Respond(0x0000, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let outcome = find(&remote, &by_patient("P1"), None).await.0.unwrap();
    assert_eq!(outcome.matches, 1);
    assert!(outcome.optional_keys_unsupported);
}

/// C.4.1.2.1: the SCU filters matches an SCP made without supporting a key.
#[tokio::test]
async fn matches_that_miss_a_key_are_filtered_out() {
    let (remote, _) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::Respond(0xFF00, Some(study("2.25.2", "OTHER"))),
            Step::Respond(0x0000, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let (result, found) = find(&remote, &by_patient("P1"), None).await;
    assert_eq!(found, ["2.25.1"]);
    assert_eq!(result.unwrap().filtered_out, 1);
}

/// C.4.1.2.1 and PS3.7 9.3.2.3: stopping sends C-CANCEL-FIND-RQ for the request's Message
/// ID; the Pending responses still in flight are dropped; Cancel (FE00) ends it.
#[tokio::test]
async fn stopping_sends_c_cancel_and_waits_for_cancel() {
    let (remote, seen) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::ExpectCancel,
            Step::Respond(0xFF00, Some(study("2.25.2", "P1"))),
            Step::Respond(0xFE00, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let (result, found) = find(&remote, &by_patient("P1"), Some(1)).await;
    let outcome = result.unwrap();
    assert_eq!(found, ["2.25.1"]);
    assert_eq!((outcome.status, outcome.cancelled), (Status::CANCEL, true));
    assert!(!outcome.complete());
    let seen = seen.lock().unwrap();
    let cancel = seen.cancel.as_ref().unwrap();
    assert_eq!(cancel.command_field(), Ok(field::C_CANCEL_RQ));
    assert_eq!(
        cancel.us(tag::MESSAGE_ID_BEING_RESPONDED_TO),
        seen.request.as_ref().unwrap().message_id()
    );
}

/// An SCP may finish with Success though it was asked to cancel (it had no more matches).
#[tokio::test]
async fn success_after_a_cancel_is_still_cancelled() {
    let (remote, _) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::ExpectCancel,
            Step::Respond(0x0000, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let outcome = find(&remote, &by_patient("P1"), Some(1)).await.0.unwrap();
    assert!(outcome.cancelled && !outcome.complete());
}

/// Table C.4-1: A700, A900 and Cxxx are failures; their Error Comment is reported, and the
/// association is released, not aborted.
#[tokio::test]
async fn a_failure_status_is_reported_with_its_comment() {
    for code in [0xA700, 0xA900, 0xC001, 0x0122] {
        let (remote, seen) = scripted(
            vec![
                Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
                Step::Fail(code, "unable to process"),
            ],
            &[EXPLICIT],
        )
        .await;
        match find(&remote, &by_patient("P1"), None).await.0 {
            Err(FindError::Failed { status, comment }) => {
                assert_eq!(status, Status(code));
                assert_eq!(comment.as_deref(), Some("unable to process"));
            }
            other => panic!("{code:04X}: {other:?}"),
        }
        assert_eq!(ended(&seen).await, Some("release"));
    }
}

/// PS3.7 9.1.2.1.6, Warning: matching ended early, the matches may be incomplete.
#[tokio::test]
async fn a_warning_ends_with_incomplete_matches() {
    let (remote, _) = scripted(
        vec![
            Step::Respond(0xFF00, Some(study("2.25.1", "P1"))),
            Step::Respond(0xB000, None),
        ],
        &[EXPLICIT],
    )
    .await;
    let outcome = find(&remote, &by_patient("P1"), None).await.0.unwrap();
    assert_eq!(outcome.status, Status(0xB000));
    assert!(!outcome.complete());
}

/// PS3.7 9.1.2.1.5: a Pending response carries an Identifier; one without is a protocol
/// error, and the association is aborted.
#[tokio::test]
async fn a_pending_response_without_identifier_aborts() {
    let (remote, seen) = scripted(vec![Step::Respond(0xFF00, None)], &[EXPLICIT]).await;
    let result = find(&remote, &by_patient("P1"), None).await.0;
    assert!(
        matches!(result, Err(FindError::PendingWithoutIdentifier)),
        "{result:?}"
    );
    assert_eq!(ended(&seen).await, Some("abort"));
}

/// PS3.7 Table 9.3-4: Message ID Being Responded To is the request's.
#[tokio::test]
async fn a_response_to_another_message_aborts() {
    let (remote, _) = scripted(vec![Step::WrongMessageId], &[EXPLICIT]).await;
    let result = find(&remote, &by_patient("P1"), None).await.0;
    assert!(
        matches!(result, Err(FindError::WrongResponse(_))),
        "{result:?}"
    );
}

/// C.4.1.1.3.2: the response's Query/Retrieve Level is the request's.
#[tokio::test]
async fn a_match_at_another_level_is_refused() {
    let mut series = study("2.25.1", "P1");
    series.put(DataElement::new(
        tags::QUERY_RETRIEVE_LEVEL,
        VR::CS,
        PrimitiveValue::from("SERIES"),
    ));
    let (remote, _) = scripted(vec![Step::Respond(0xFF00, Some(series))], &[EXPLICIT]).await;
    let result = find(&remote, &by_patient("P1"), None).await.0;
    assert!(
        matches!(result, Err(FindError::Identifier(_))),
        "{result:?}"
    );
}

/// A silent SCP: the SCU gives up after its timeout, it does not hang.
#[tokio::test]
async fn a_silent_scp_times_out() {
    let (remote, _) = scripted(vec![Step::Silence], &[EXPLICIT]).await;
    let started = std::time::Instant::now();
    let result = find(&remote, &by_patient("P1"), None).await.0;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A query that breaks the matching rules is refused before any connection.
#[tokio::test]
async fn an_invalid_query_never_connects() {
    let remote: RemoteAe = "x=X@127.0.0.1:9".parse().unwrap();
    let result = find(&remote, &by_patient("A\\B"), None).await.0;
    assert!(matches!(result, Err(FindError::Query(_))), "{result:?}");
}

/// An SCP without Study Root FIND rejects the presentation context; no association is used.
#[tokio::test]
async fn an_scp_without_study_root_find_is_reported() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _ = ServerAssociationOptions::new()
            .ae_title("ECHOONLY")
            .with_abstract_syntax(pacs_dimse::VERIFICATION)
            .establish_async(socket)
            .await;
    });
    let remote: RemoteAe = format!("e=ECHOONLY@127.0.0.1:{port}").parse().unwrap();
    let result = find(&remote, &by_patient("P1"), None).await.0;
    assert!(matches!(result, Err(FindError::Connect(_))), "{result:?}");
}
