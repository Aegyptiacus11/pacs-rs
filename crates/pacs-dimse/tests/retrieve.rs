//! The C-MOVE SCU against a scripted SCP: the C-MOVE-RQ of PS3.7 Table 9.3-9, the Pending
//! counts and final statuses of PS3.4 Table C.4-2, C-CANCEL-MOVE-RQ (Table 9.3-11), and the
//! Failed SOP Instance UID List (C.4.2.1.4.2).

mod common;

use std::ops::ControlFlow;
use std::time::Duration;

use common::scripted::{Act, ended, response, serve};
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use pacs_dimse::command::{Command, field, tag};
use pacs_dimse::scu::retrieve::STUDY_ROOT_MOVE;
use pacs_dimse::{AeTitle, MoveError, MoveOutcome, Progress, RemoteAe, Status, move_studies};

const EXPLICIT: &str = "1.2.840.10008.1.2.1";

/// A C-MOVE-RSP with these counts, `None` left out.
fn rsp(id: u16, status: u16, counts: [Option<u16>; 4]) -> Command {
    let mut c = response(STUDY_ROOT_MOVE, field::C_MOVE_RSP, id, status);
    for (element, count) in [
        tag::NUMBER_OF_REMAINING_SUB_OPERATIONS,
        tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS,
        tag::NUMBER_OF_FAILED_SUB_OPERATIONS,
        tag::NUMBER_OF_WARNING_SUB_OPERATIONS,
    ]
    .into_iter()
    .zip(counts)
    {
        if let Some(count) = count {
            c = c.with_us(element, count);
        }
    }
    c
}

fn pending(id: u16, remaining: u16, completed: u16) -> Act {
    Act::Send(
        rsp(
            id,
            0xFF00,
            [Some(remaining), Some(completed), Some(0), Some(0)],
        ),
        None,
    )
}

fn failed_list(uids: &[&str]) -> InMemDicomObject {
    let mut o = InMemDicomObject::new_empty();
    o.put(DataElement::new(
        tags::FAILED_SOP_INSTANCE_UID_LIST,
        VR::UI,
        PrimitiveValue::Strs(uids.iter().map(|u| u.to_string()).collect()),
    ));
    o
}

async fn pull(
    remote: &RemoteAe,
    studies: &[&str],
    stop_after: Option<usize>,
) -> (Result<MoveOutcome, MoveError>, Vec<Progress>) {
    let mut seen = Vec::new();
    let studies: Vec<String> = studies.iter().map(|s| s.to_string()).collect();
    let result = move_studies(
        remote,
        &AeTitle::new("PACSRS").unwrap(),
        &studies,
        &AeTitle::new("PACSRS").unwrap(),
        Duration::from_secs(2),
        Duration::from_millis(1500),
        |progress| {
            seen.push(progress);
            match stop_after {
                Some(n) if seen.len() >= n => ControlFlow::Break(()),
                _ => ControlFlow::Continue(()),
            }
        },
    )
    .await;
    (result, seen)
}

/// PS3.7 Table 9.3-9 and PS3.4 C.4.2.1.4.1: Study Root MOVE, Command Field 0021H, Move
/// Destination pacs-rs's AE title, and an Identifier at STUDY level whose Study Instance
/// UID is the list asked for (List of UID Matching, C.4.2.2.1).
#[tokio::test]
async fn the_request_is_a_study_root_c_move_to_pacs_rs() {
    let (remote, seen) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![Act::Send(
            rsp(id, 0x0000, [None, Some(2), Some(0), Some(0)]),
            None,
        )]
    })
    .await;
    let (result, _) = pull(&remote, &["2.25.1", "2.25.2"], None).await;
    assert_eq!(result.unwrap().status, Status::SUCCESS);
    let seen = seen.lock().unwrap();
    let rq = seen.request.as_ref().unwrap();
    assert_eq!(rq.command_field(), Ok(field::C_MOVE_RQ));
    assert_eq!(rq.ui(tag::AFFECTED_SOP_CLASS_UID), Ok(STUDY_ROOT_MOVE));
    assert_eq!(rq.text(tag::MOVE_DESTINATION), Some("PACSRS"));
    assert_eq!(rq.has_data_set(), Ok(true));
    let id = seen.identifier.as_ref().unwrap();
    assert_eq!(
        id.element(tags::QUERY_RETRIEVE_LEVEL)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "STUDY"
    );
    let uids = id
        .element(tags::STUDY_INSTANCE_UID)
        .unwrap()
        .to_multi_str()
        .unwrap();
    let uids: Vec<_> = uids.iter().map(|u| u.trim_end_matches('\0')).collect();
    assert_eq!(uids, ["2.25.1", "2.25.2"]);
}

/// C.4.2.2.1: Pending responses carry the Remaining, Completed, Failed and Warning counts,
/// handed over in order; Success ends with every sub-operation done.
#[tokio::test]
async fn pending_counts_report_progress_until_success() {
    let (remote, seen) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![
            pending(id, 2, 0),
            pending(id, 1, 1),
            pending(id, 0, 2),
            Act::Send(rsp(id, 0x0000, [None, Some(2), Some(0), Some(0)]), None),
        ]
    })
    .await;
    let (result, progress) = pull(&remote, &["2.25.1"], None).await;
    let outcome = result.unwrap();
    let remaining: Vec<_> = progress.iter().map(|p| p.remaining).collect();
    assert_eq!(remaining, [Some(2), Some(1), Some(0)]);
    assert_eq!(outcome.status, Status::SUCCESS);
    assert_eq!(
        (outcome.progress.completed, outcome.progress.failed),
        (Some(2), Some(0))
    );
    assert!(outcome.failed_instances.is_empty() && !outcome.cancelled);
    assert_eq!(ended(&seen).await, Some("release"));
}

/// C.4.2.1.7-9: a final response may leave the counts out; the last Pending's stand.
#[tokio::test]
async fn a_final_response_without_counts_keeps_the_last_ones() {
    let (remote, _) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![
            pending(id, 0, 3),
            Act::Send(rsp(id, 0x0000, [None; 4]), None),
        ]
    })
    .await;
    let outcome = pull(&remote, &["2.25.1"], None).await.0.unwrap();
    assert_eq!(outcome.progress.completed, Some(3));
}

/// Table C.4-2, B000: some sub-operations failed; C.4.2.1.4.2: the Failed SOP Instance UID
/// List names them.
#[tokio::test]
async fn a_warning_names_the_failed_instances() {
    let (remote, _) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![Act::Send(
            rsp(id, 0xB000, [None, Some(1), Some(1), Some(0)]),
            Some(failed_list(&["2.25.1.2"])),
        )]
    })
    .await;
    let outcome = pull(&remote, &["2.25.1"], None).await.0.unwrap();
    assert_eq!(outcome.status, Status::SUB_OPERATIONS_WARNING);
    assert_eq!(outcome.failed_instances, ["2.25.1.2"]);
    assert_eq!(outcome.progress.failed, Some(1));
}

/// Table C.4-2, A801: the remote AE does not know pacs-rs as a destination; the error says
/// what a site has to do. A failure releases the association.
#[tokio::test]
async fn an_unknown_move_destination_says_to_register_pacs_rs() {
    let (remote, seen) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![Act::Send(rsp(id, 0xA801, [None; 4]), None)]
    })
    .await;
    match pull(&remote, &["2.25.1"], None).await.0 {
        Err(e @ MoveError::Failed { .. }) => {
            let MoveError::Failed { status, .. } = &e else {
                unreachable!()
            };
            assert_eq!(*status, Status::MOVE_DESTINATION_UNKNOWN);
            assert!(e.to_string().contains("registered"), "{e}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(ended(&seen).await, Some("release"));
}

/// Table C.4-2, Cxxx: Failed, with its Error Comment and failed instances.
#[tokio::test]
async fn a_failure_carries_its_comment_and_failed_instances() {
    let (remote, _) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![Act::Send(
            rsp(id, 0xC000, [None, Some(0), Some(2), Some(0)])
                .with_text(tag::ERROR_COMMENT, "unable to process"),
            Some(failed_list(&["2.25.1.1", "2.25.1.2"])),
        )]
    })
    .await;
    match pull(&remote, &["2.25.1"], None).await.0 {
        Err(MoveError::Failed {
            status,
            comment,
            failed_instances,
            progress,
            ..
        }) => {
            assert_eq!(status, Status(0xC000));
            assert_eq!(comment.as_deref(), Some("unable to process"));
            assert_eq!(failed_instances.len(), 2);
            assert_eq!(progress.failed, Some(2));
        }
        other => panic!("{other:?}"),
    }
}

/// C.4.2.2.1 and PS3.7 Table 9.3-11: stopping sends C-CANCEL-MOVE-RQ for the request's
/// Message ID; Cancel ends it, with the sub-operations not started as Remaining.
#[tokio::test]
async fn stopping_sends_c_cancel_move() {
    let (remote, seen) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![
            pending(id, 9, 1),
            Act::ExpectCancel,
            Act::Send(
                rsp(id, 0xFE00, [Some(8), Some(1), Some(0), Some(0)]),
                Some(failed_list(&[])),
            ),
        ]
    })
    .await;
    let outcome = pull(&remote, &["2.25.1"], Some(1)).await.0.unwrap();
    assert_eq!(outcome.status, Status::CANCEL);
    assert!(outcome.cancelled);
    assert_eq!(
        (outcome.progress.remaining, outcome.progress.completed),
        (Some(8), Some(1))
    );
    let seen = seen.lock().unwrap();
    let cancel = seen.cancel.as_ref().unwrap();
    assert_eq!(cancel.command_field(), Ok(field::C_CANCEL_RQ));
    assert_eq!(
        cancel.us(tag::MESSAGE_ID_BEING_RESPONDED_TO),
        seen.request.as_ref().unwrap().message_id()
    );
}

/// PS3.7 Table 9.3-10: the response is a C-MOVE-RSP to the request; anything else aborts.
#[tokio::test]
async fn a_response_of_another_kind_aborts() {
    let (remote, seen) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![Act::Send(
            response(STUDY_ROOT_MOVE, field::C_FIND_RSP, id, 0),
            None,
        )]
    })
    .await;
    let result = pull(&remote, &["2.25.1"], None).await.0;
    assert!(
        matches!(result, Err(MoveError::WrongResponse(_))),
        "{result:?}"
    );
    assert_eq!(ended(&seen).await, Some("abort"));
}

/// Nothing to move, or something that is not a UID, is refused before any connection.
#[tokio::test]
async fn bad_study_uids_never_connect() {
    let remote: RemoteAe = "x=X@127.0.0.1:9".parse().unwrap();
    for studies in [&[][..], &["not a uid"][..], &["1.2..3"][..]] {
        let result = pull(&remote, studies, None).await.0;
        assert!(
            matches!(result, Err(MoveError::Request(_))),
            "{studies:?}: {result:?}"
        );
    }
}

/// The wait between two responses is bounded: a remote AE that goes quiet is given up on.
#[tokio::test]
async fn a_silent_scp_times_out() {
    let (remote, _) = serve("QRSCP", STUDY_ROOT_MOVE, &[EXPLICIT], |id| {
        vec![pending(id, 5, 0), Act::Silence]
    })
    .await;
    let started = std::time::Instant::now();
    let result = pull(&remote, &["2.25.1"], None).await.0;
    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(5));
}
