//! What each AE title of pacs-rs offers (`Config::capabilities`): its own services and
//! transfer syntaxes, negotiated as such on one port (PS3.8 9.3.3.2: a presentation context
//! is rejected for an abstract syntax or a transfer syntax that is not supported), while an
//! AE title given none offers everything.

mod common;

use common::*;
use dicom_ul::pdu::PresentationContextResultReason;
use dicom_ul::pdu::PresentationContextResultReason as Reason;
use pacs_dimse::query::STUDY_ROOT_FIND;
use pacs_dimse::{AeTitle, VERIFICATION};

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";
const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

async fn server() -> std::net::SocketAddr {
    let mut config = config();
    config.extra_ae_titles = vec![AeTitle::new("RESEARCH").unwrap()];
    config.capabilities = vec![
        "RESEARCH=storage+verification/uncompressed"
            .parse()
            .unwrap(),
    ];
    start(config, archive().await).await
}

/// For each abstract syntax proposed, alone in its context with `syntaxes`: the result.
async fn negotiated(
    addr: std::net::SocketAddr,
    called: &str,
    proposed: &[&str],
    syntaxes: &[&str],
) -> Vec<(String, PresentationContextResultReason, String)> {
    let client = Client::connect(addr, "VIEWER", called, proposed, syntaxes)
        .await
        .unwrap();
    client
        .association
        .presentation_contexts()
        .iter()
        .map(|pc| {
            (
                pc.abstract_syntax.trim_end_matches('\0').to_owned(),
                pc.reason.clone(),
                pc.transfer_syntax.trim_end_matches('\0').to_owned(),
            )
        })
        .collect()
}

/// The accepted contexts, as (abstract syntax, transfer syntax): a client is told of the
/// others only that they were not accepted.
fn accepted(of: &[(String, Reason, String)]) -> Vec<(&str, &str)> {
    let mut accepted: Vec<(&str, &str)> = of
        .iter()
        .filter(|(_, reason, _)| *reason == Reason::Acceptance)
        .map(|(abstract_syntax, _, ts)| (abstract_syntax.as_str(), ts.as_str()))
        .collect();
    accepted.sort();
    accepted
}

#[tokio::test]
async fn an_ae_title_negotiates_its_own_services_and_syntaxes() {
    let addr = server().await;
    let proposed = [VERIFICATION, CT, STUDY_ROOT_FIND];
    let syntaxes = [JPEG_LS, EXPLICIT_VR_LE];
    // stored, but uncompressed only: Explicit VR Little Endian, not JPEG-LS; no C-FIND
    let research = negotiated(addr, "RESEARCH", &proposed, &syntaxes).await;
    assert_eq!(
        accepted(&research),
        [(VERIFICATION, EXPLICIT_VR_LE), (CT, EXPLICIT_VR_LE)]
    );
    // in JPEG-LS alone nothing is taken by RESEARCH, everything by PACSRS
    let only_jpeg_ls =
        |called| Client::connect(addr, "VIEWER", called, &[VERIFICATION, CT], &[JPEG_LS]);
    assert!(only_jpeg_ls("RESEARCH").await.is_err());
    assert!(only_jpeg_ls("PACSRS").await.is_ok());
    // pacs-rs's own AE title, on the same port, offers everything
    let own = negotiated(addr, "PACSRS", &proposed, &syntaxes).await;
    assert_eq!(
        accepted(&own),
        [
            (VERIFICATION, JPEG_LS),
            (CT, JPEG_LS),
            (STUDY_ROOT_FIND, JPEG_LS)
        ]
    );
}
