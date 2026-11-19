//! The DIMSE side as pacs-server runs it: its settings, the `/modalities` routes (echo and a
//! C-FIND search), and an instance received by C-STORE showing in the REST API and the web
//! interface's list.

use pacs_server::api::remote::{DicomJson, Echo, Error, Job, Moved, RemoteAes, Started};
use std::net::SocketAddr;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use dicom_ul::ClientAssociationOptions;
use http_body_util::BodyExt;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dimse::command::{Command, PRIORITY_MEDIUM, field, tag};
use pacs_dimse::message::{self, Incoming};
use pacs_server::{Settings, app, dimse_config, open_archive, serve_dimse};
use serde_json::{Value, json};
use tower::ServiceExt;

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";

/// An answer of the remote AE routes, as the type the API's description names for it; the
/// types refuse a field they do not name.
fn described<T: serde::de::DeserializeOwned>(answer: &Value) -> T {
    serde_json::from_value(answer.clone())
        .unwrap_or_else(|e| panic!("not what the description says: {e}\n{answer}"))
}

fn settings_in(dir: &Path, extra: &[&str]) -> Settings {
    let mut args = vec![
        "pacs-server".to_owned(),
        "--index".into(),
        dir.join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.join("files").display()),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    Settings::try_parse_from(args).unwrap()
}

async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
    send(app, Request::get(uri).body(Body::empty()).unwrap()).await
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// A DIMSE listener for `settings` on a free loopback port.
async fn start_dimse(settings: &Settings) -> SocketAddr {
    let archive = open_archive(settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = dimse_config(settings).unwrap();
    tokio::spawn(serve_dimse(
        listener,
        archive,
        config,
        Vec::new(),
        std::future::pending(),
    ));
    addr
}

#[test]
fn dimse_is_off_unless_a_port_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    assert_eq!(settings.dimse_listen, None);
    let config = dimse_config(&settings).unwrap();
    assert_eq!(config.ae_title.as_str(), "PACSRS");
    assert_eq!(config.max_associations, 16);
    assert_eq!(config.timeout, std::time::Duration::from_secs(30));
    assert!(config.remotes.is_empty() && config.extra_ae_titles.is_empty());
    assert_eq!(config.max_data_set_bytes, settings.max_upload_bytes);
}

#[test]
fn dimse_settings_are_read_and_checked() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(
        dir.path(),
        &[
            "--dimse-listen",
            "0.0.0.0:11112",
            "--dimse-ae-title",
            "RESEARCH_PACS",
            "--dimse-extra-ae-titles",
            "OLD_NAME,OTHER",
            "--dimse-remotes",
            "hospital=HOSP_PACS@10.0.0.5:104,ws=WS1@[::1]:11112",
            "--dimse-known-callers-only",
            "--dimse-move-destinations",
            "ws",
        ],
    );
    let config = dimse_config(&settings).unwrap();
    // only what is named may be sent to: not the archive that is read from
    assert_eq!(config.move_destination("WS1").unwrap().id, "ws");
    assert!(config.move_destination("HOSP_PACS").is_none());
    assert_eq!(config.ae_title.as_str(), "RESEARCH_PACS");
    assert_eq!(config.extra_ae_titles.len(), 2);
    assert_eq!(config.remote("ws").unwrap().host, "::1");
    assert!(config.known_callers_only);

    let base = ["pacs-server".to_owned()];
    let parse = |extra: &[&str]| {
        Settings::try_parse_from(
            base.iter()
                .cloned()
                .chain(extra.iter().map(|s| s.to_string())),
        )
    };
    assert!(parse(&["--dimse-ae-title", "A_TITLE_LONGER_THAN_16"]).is_err());
    assert!(parse(&["--dimse-remotes", "no-id@host:104"]).is_err());
    let twice = parse(&["--dimse-remotes", "a=A@h:1,a=B@h:2"]).unwrap();
    assert!(dimse_config(&twice).is_err());
    // without the setting nothing is a destination
    let unnamed = parse(&["--dimse-remotes", "ws=WS1@h:104"]).unwrap();
    let config = dimse_config(&unnamed).unwrap();
    assert!(config.move_destination("WS1").is_none());
    // a destination must be a remote AE, and its AE title that of no other destination
    for (remotes, destinations, why) in [
        ("ws=WS1@h:104", "viewer", "is not a remote AE"),
        ("ws=WS1@h:104", "WS1", "is not a remote AE"),
        (
            "a=WS1@h:104,b=WS1@other:104",
            "a,b",
            "the same AE title WS1",
        ),
        ("a=WS1@h:104", "a,a", "the same AE title WS1"),
    ] {
        let settings = parse(&[
            "--dimse-remotes",
            remotes,
            "--dimse-move-destinations",
            destinations,
        ])
        .unwrap();
        let error = dimse_config(&settings).unwrap_err().to_string();
        assert!(error.contains(why), "{destinations}: {error}");
    }
    let none = parse(&["--dimse-max-associations", "0"]).unwrap();
    assert!(dimse_config(&none).is_err());
}

#[tokio::test]
async fn modalities_lists_the_remote_aes() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(
        dir.path(),
        &[
            "--dimse-remotes",
            "hospital=HOSP_PACS@10.0.0.5:104,ws=WS1@ws.local:11112",
        ],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    assert_eq!(
        get(&app, "/modalities").await,
        (StatusCode::OK, json!(["hospital", "ws"]))
    );
    let (status, expanded) = get(&app, "/modalities?expand").await;
    described::<RemoteAes>(&expanded);
    described::<RemoteAes>(&get(&app, "/modalities").await.1);
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        expanded["hospital"],
        json!({"AET": "HOSP_PACS", "Host": "10.0.0.5", "Port": 104})
    );
}

/// `POST /modalities/{id}/echo`: 200 when the remote AE answers, 502 when it cannot be
/// reached, 404 for an id that is not configured.
#[tokio::test]
async fn echo_tests_the_link_to_a_remote_ae() {
    let dir = tempfile::tempdir().unwrap();
    let peer = start_dimse(&settings_in(dir.path(), &["--dimse-ae-title", "PEER"])).await;
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let remotes = format!(
        "peer=PEER@127.0.0.1:{},gone=GONE@127.0.0.1:{}",
        peer.port(),
        closed.port()
    );
    let other = tempfile::tempdir().unwrap();
    let settings = settings_in(
        other.path(),
        &["--dimse-remotes", &remotes, "--dimse-timeout", "2"],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let echo = |id: &str| {
        Request::post(format!("/modalities/{id}/echo"))
            .body(Body::empty())
            .unwrap()
    };

    let (status, body) = send(&app, echo("peer")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["Status"], "Success");
    assert_eq!(body["AET"], "PEER");
    described::<Echo>(&body);

    let (status, body) = send(&app, echo("gone")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["Message"], "C-ECHO to gone failed");
    described::<Error>(&body);

    assert_eq!(send(&app, echo("nobody")).await.0, StatusCode::NOT_FOUND);
}

/// C-STORE goes into the same archive the HTTP side serves: the study is in `/studies`
/// and in the web interface's study list.
#[tokio::test]
async fn a_c_store_shows_in_the_rest_api_and_the_study_list() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let addr = start_dimse(&settings).await;
    let series = SyntheticSeries::new(91, 1);
    let file = series.slice_bytes(0);
    let meta = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
    let data_set = &file[144 + meta..];

    let mut association = ClientAssociationOptions::new()
        .calling_ae_title("STORESCU")
        .called_ae_title("PACSRS")
        .with_presentation_context(CT, vec!["1.2.840.10008.1.2.1"])
        .establish_async(addr)
        .await
        .unwrap();
    let context = association.presentation_contexts()[0].id;
    let rq = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, CT)
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, 1)
        .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, &series.instance_uid(0));
    message::send(&mut association, context, &rq, Some(data_set))
        .await
        .unwrap();
    let Incoming::Message(rsp) = message::receive(&mut association, 0).await.unwrap() else {
        panic!("no C-STORE-RSP");
    };
    assert_eq!(rsp.command.status(), Ok(pacs_dimse::Status::SUCCESS));
    association.release().await.unwrap();

    // the HTTP side opens the same index afterwards, as the binary shares one archive
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (_, studies) = get(&app, "/studies?expand").await;
    assert_eq!(
        studies[0]["MainDicomTags"]["StudyInstanceUID"],
        series.study_uid()
    );
    let (_, page) = get(&app, "/ui/api/studies").await;
    assert_eq!(page["Total"], 1);
}

/// A query SCP for `/modalities/{id}/studies`: answers one C-FIND with a Pending response per
/// Patient ID in `patients`, then Success, or Cancel once a C-CANCEL comes. Returns the port
/// and the Patient ID the request asked for.
async fn query_scp(patients: &'static [&'static str]) -> (u16, tokio::task::JoinHandle<String>) {
    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;
    use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
    use dicom_object::InMemDicomObject;
    use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
    use dicom_ul::ServerAssociationOptions;
    use pacs_dimse::query::STUDY_ROOT_FIND;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut association = ServerAssociationOptions::new()
            .ae_title("QRSCP")
            .with_abstract_syntax(STUDY_ROOT_FIND)
            .establish_async(socket)
            .await
            .unwrap();
        let context = association.presentation_contexts()[0].clone();
        let ts = TransferSyntaxRegistry
            .get(&context.transfer_syntax)
            .unwrap();
        let Incoming::Message(rq) = message::receive(&mut association, 1 << 20).await.unwrap()
        else {
            panic!("no C-FIND-RQ");
        };
        let pacs_dimse::message::DataSet::Received(bytes) = rq.data else {
            panic!()
        };
        let asked = InMemDicomObject::read_dataset_with_ts(&bytes[..], ts).unwrap();
        let asked = asked
            .element(tags::PATIENT_ID)
            .unwrap()
            .to_str()
            .unwrap()
            .trim()
            .to_owned();
        let rsp = |status: u16, data: bool| {
            Command::new()
                .with_ui(tag::AFFECTED_SOP_CLASS_UID, STUDY_ROOT_FIND)
                .with_us(tag::COMMAND_FIELD, field::C_FIND_RSP)
                .with_us(
                    tag::MESSAGE_ID_BEING_RESPONDED_TO,
                    rq.command.message_id().unwrap(),
                )
                .with_us(
                    tag::COMMAND_DATA_SET_TYPE,
                    if data { 0x0102 } else { 0x0101 },
                )
                .with_us(tag::STATUS, status)
        };
        let mut cancelled = false;
        for (i, patient) in patients.iter().enumerate() {
            let mut study = InMemDicomObject::new_empty();
            study.put(DataElement::new(
                tags::QUERY_RETRIEVE_LEVEL,
                VR::CS,
                PrimitiveValue::from("STUDY"),
            ));
            study.put(DataElement::new(
                tags::PATIENT_ID,
                VR::LO,
                PrimitiveValue::from(*patient),
            ));
            study.put(DataElement::new(
                tags::STUDY_INSTANCE_UID,
                VR::UI,
                PrimitiveValue::from(format!("2.25.{}", i + 1)),
            ));
            let mut out = Vec::new();
            study.write_dataset_with_ts(&mut out, ts).unwrap();
            message::send(&mut association, context.id, &rsp(0xFF00, true), Some(&out))
                .await
                .unwrap();
            // a cancel, if one comes, is read before the next match
            if let Ok(Ok(Incoming::Message(m))) = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                message::receive(&mut association, 0),
            )
            .await
            {
                cancelled = m.command.command_field() == Ok(field::C_CANCEL_RQ);
                break;
            }
        }
        let last = if cancelled { 0xFE00 } else { 0x0000 };
        message::send(&mut association, context.id, &rsp(last, false), None)
            .await
            .unwrap();
        let _ = message::receive(&mut association, 0).await;
        asked
    });
    (port, task)
}

/// `GET /modalities/{id}/studies`: the C-FIND's matches as a DICOM JSON array.
#[tokio::test]
async fn studies_searches_a_remote_ae_by_c_find() {
    let (port, scp) = query_scp(&["P1", "P1"]).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(dir.path(), &["--dimse-remotes", &remotes]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get("/modalities/qr/studies?PatientID=P1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/dicom+json");
    assert!(response.headers().get("warning").is_none());
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let studies: Value = serde_json::from_slice(&body).unwrap();
    described::<Vec<DicomJson>>(&studies);
    assert_eq!(studies.as_array().unwrap().len(), 2);
    assert_eq!(studies[1]["0020000D"]["Value"][0], "2.25.2");
    assert_eq!(scp.await.unwrap(), "P1");
}

/// Past `limit`, the search is cancelled and a Warning header says more studies match.
#[tokio::test]
async fn studies_stops_at_the_limit_with_a_warning() {
    let (port, _scp) = query_scp(&["P1", "P1", "P1"]).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(dir.path(), &["--dimse-remotes", &remotes]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let response = app
        .oneshot(
            Request::get("/modalities/qr/studies?PatientID=P1&limit=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let warning = response.headers()["warning"].to_str().unwrap().to_owned();
    assert!(
        warning.starts_with("299 pacs-rs") && warning.contains("limit of 1"),
        "{warning}"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<Value>(&body)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// 400 for keys that break the matching rules or are not search keys, before any
/// association; 404 for an unknown id; 502 when the remote AE cannot be reached.
#[tokio::test]
async fn studies_refuses_bad_queries_and_reports_unreachable_archives() {
    let dir = tempfile::tempdir().unwrap();
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let remotes = format!("gone=GONE@127.0.0.1:{}", closed.port());
    let settings = settings_in(
        dir.path(),
        &["--dimse-remotes", &remotes, "--dimse-timeout", "2"],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    for bad in [
        "StudyDate=2026-01-01",
        "StudyDate=20260131-20260101",
        "PatientID=A%5CB",
        "Modality=CT",
        "limit=0",
        "limit=5000",
    ] {
        let (status, _) = get(&app, &format!("/modalities/gone/studies?{bad}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(
        get(&app, "/modalities/nobody/studies").await.0,
        StatusCode::NOT_FOUND
    );
    let (status, body) = get(&app, "/modalities/gone/studies?PatientID=P1").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["Message"], "C-FIND to gone failed");
}

/// A move SCP for `/modalities/{id}/move`: answers one C-MOVE with `status` and these
/// Completed and Failed counts, and returns the Move Destination it was given.
async fn move_scp(
    status: u16,
    completed: u16,
    failed: u16,
) -> (u16, tokio::task::JoinHandle<String>) {
    use dicom_ul::ServerAssociationOptions;
    use pacs_dimse::scu::retrieve::STUDY_ROOT_MOVE;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut association = ServerAssociationOptions::new()
            .ae_title("QRSCP")
            .with_abstract_syntax(STUDY_ROOT_MOVE)
            .establish_async(socket)
            .await
            .unwrap();
        let context = association.presentation_contexts()[0].id;
        let Incoming::Message(rq) = message::receive(&mut association, 1 << 20).await.unwrap()
        else {
            panic!("no C-MOVE-RQ");
        };
        let destination = rq.command.text(tag::MOVE_DESTINATION).unwrap().to_owned();
        let rsp = Command::new()
            .with_ui(tag::AFFECTED_SOP_CLASS_UID, STUDY_ROOT_MOVE)
            .with_us(tag::COMMAND_FIELD, field::C_MOVE_RSP)
            .with_us(
                tag::MESSAGE_ID_BEING_RESPONDED_TO,
                rq.command.message_id().unwrap(),
            )
            .with_us(tag::COMMAND_DATA_SET_TYPE, 0x0101)
            .with_us(tag::STATUS, status)
            .with_us(tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed)
            .with_us(tag::NUMBER_OF_FAILED_SUB_OPERATIONS, failed)
            .with_us(tag::NUMBER_OF_WARNING_SUB_OPERATIONS, 0);
        message::send(&mut association, context, &rsp, None)
            .await
            .unwrap();
        let _ = message::receive(&mut association, 0).await;
        destination
    });
    (port, task)
}

fn move_request(id: &str, body: &str) -> Request<Body> {
    Request::post(format!("/modalities/{id}/move"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

/// `POST /modalities/{id}/move`: the remote AE is asked to send the study to pacs-rs's own
/// AE title; the final status and counts come back.
#[tokio::test]
async fn move_pulls_a_study_to_pacs_rs() {
    let (port, scp) = move_scp(0x0000, 3, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(
        dir.path(),
        &[
            "--dimse-remotes",
            &remotes,
            "--dimse-listen",
            "127.0.0.1:0",
            "--dimse-ae-title",
            "RESEARCH",
        ],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, body) = send(
        &app,
        move_request("qr", r#"{"StudyInstanceUID": "2.25.1"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["Status"], "Success");
    assert_eq!(
        (body["Completed"].as_u64(), body["Failed"].as_u64()),
        (Some(3), Some(0))
    );
    described::<Moved>(&body);
    assert_eq!(scp.await.unwrap(), "RESEARCH");
}

/// A study the archive does not have: Success with nothing sent, and a Warning header says
/// so rather than leave a client to believe the study arrived.
#[tokio::test]
async fn move_of_a_study_the_archive_lacks_warns() {
    let (port, _scp) = move_scp(0x0000, 0, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(
        dir.path(),
        &["--dimse-remotes", &remotes, "--dimse-listen", "127.0.0.1:0"],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let response = app
        .oneshot(move_request("qr", r#"{"StudyInstanceUID": "2.25.404"}"#))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let warning = response.headers()["warning"].to_str().unwrap();
    assert!(warning.contains("sent nothing"), "{warning}");
}

/// Table C.4-2 A801: the hospital does not know pacs-rs; 502 says it has to be registered.
#[tokio::test]
async fn move_to_an_archive_that_does_not_know_pacs_rs_is_502() {
    let (port, _scp) = move_scp(0xA801, 0, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(
        dir.path(),
        &["--dimse-remotes", &remotes, "--dimse-listen", "127.0.0.1:0"],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, body) = send(
        &app,
        move_request("qr", r#"{"StudyInstanceUID": ["2.25.1"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(
        body["Details"].as_str().unwrap().contains("registered"),
        "{body}"
    );
}

/// Without a DIMSE port nothing moved could arrive: 409, before any association. A body
/// without valid UIDs is 400, an unknown id 404.
#[tokio::test]
async fn move_refuses_what_cannot_work() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &["--dimse-remotes", "qr=QRSCP@127.0.0.1:9"]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, body) = send(
        &app,
        move_request("qr", r#"{"StudyInstanceUID": "2.25.1"}"#),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        send(&app, move_request("nobody", "{}")).await.0,
        StatusCode::NOT_FOUND
    );

    let listening = settings_in(
        dir.path(),
        &[
            "--dimse-remotes",
            "qr=QRSCP@127.0.0.1:9",
            "--dimse-listen",
            "127.0.0.1:0",
        ],
    );
    let app = pacs_server::app(open_archive(&listening).await.unwrap(), &listening).unwrap();
    for bad in [
        "{}",
        r#"{"StudyInstanceUID": 7}"#,
        r#"{"StudyInstanceUID": "not a uid"}"#,
        r#"{"StudyInstanceUID": []}"#,
    ] {
        assert_eq!(
            send(&app, move_request("qr", bad)).await.0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
}

/// A slow move SCP for the jobs: `total` Pending responses, `gap_ms` apart, each one more
/// completed; a C-CANCEL in a gap ends it with Cancel (FE00), else Success. Returns whether
/// a C-CANCEL came.
async fn slow_move_scp(total: u16, gap_ms: u64) -> (u16, tokio::task::JoinHandle<bool>) {
    use dicom_ul::ServerAssociationOptions;
    use pacs_dimse::scu::retrieve::STUDY_ROOT_MOVE;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut association = ServerAssociationOptions::new()
            .ae_title("QRSCP")
            .with_abstract_syntax(STUDY_ROOT_MOVE)
            .establish_async(socket)
            .await
            .unwrap();
        let context = association.presentation_contexts()[0].id;
        let Incoming::Message(rq) = message::receive(&mut association, 1 << 20).await.unwrap()
        else {
            panic!("no C-MOVE-RQ");
        };
        let rsp = |status: u16, remaining: u16, completed: u16| {
            Command::new()
                .with_ui(tag::AFFECTED_SOP_CLASS_UID, STUDY_ROOT_MOVE)
                .with_us(tag::COMMAND_FIELD, field::C_MOVE_RSP)
                .with_us(
                    tag::MESSAGE_ID_BEING_RESPONDED_TO,
                    rq.command.message_id().unwrap(),
                )
                .with_us(tag::COMMAND_DATA_SET_TYPE, 0x0101)
                .with_us(tag::STATUS, status)
                .with_us(tag::NUMBER_OF_REMAINING_SUB_OPERATIONS, remaining)
                .with_us(tag::NUMBER_OF_COMPLETED_SUB_OPERATIONS, completed)
                .with_us(tag::NUMBER_OF_FAILED_SUB_OPERATIONS, 0)
                .with_us(tag::NUMBER_OF_WARNING_SUB_OPERATIONS, 0)
        };
        let mut cancelled = false;
        let mut completed = 0;
        for i in 1..=total {
            completed = i;
            message::send(&mut association, context, &rsp(0xFF00, total - i, i), None)
                .await
                .unwrap();
            if let Ok(Ok(Incoming::Message(m))) = tokio::time::timeout(
                std::time::Duration::from_millis(gap_ms),
                message::receive(&mut association, 0),
            )
            .await
            {
                cancelled = m.command.command_field() == Ok(field::C_CANCEL_RQ);
                break;
            }
        }
        let last = if cancelled {
            rsp(0xFE00, total - completed, completed)
        } else {
            rsp(0x0000, 0, completed)
        };
        message::send(&mut association, context, &last, None)
            .await
            .unwrap();
        let _ = message::receive(&mut association, 0).await;
        cancelled
    });
    (port, task)
}

/// Poll `GET /jobs/{id}` until the job is over (or 5 s).
async fn job_over(app: &Router, path: &str) -> Value {
    for _ in 0..100 {
        let (_, job) = get(app, path).await;
        described::<Job>(&job);
        if job["State"] != "Running" {
            return job;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the job is still running");
}

async fn app_with_remote(dir: &Path, port: u16) -> Router {
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(
        dir,
        &["--dimse-remotes", &remotes, "--dimse-listen", "127.0.0.1:0"],
    );
    app(open_archive(&settings).await.unwrap(), &settings).unwrap()
}

/// `"Asynchronous": true`: 202 with the job's ID and path at once; `GET /jobs/{id}` shows the
/// Pending counts as they come (C.4.2.1.6-9), then the final state.
#[tokio::test]
async fn an_asynchronous_move_is_a_job_with_progress() {
    let (port, _scp) = slow_move_scp(4, 150).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_remote(dir.path(), port).await;
    let (status, started) = send(
        &app,
        move_request(
            "qr",
            r#"{"StudyInstanceUID": "2.25.1", "Asynchronous": true}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    described::<Started>(&started);
    let path = started["Path"].as_str().unwrap().to_owned();
    assert_eq!(path, format!("/jobs/{}", started["ID"].as_str().unwrap()));
    assert_eq!(get(&app, "/jobs").await.1, json!([started["ID"]]));
    let mut seen_running = false;
    for _ in 0..40 {
        let (_, job) = get(&app, &path).await;
        if job["State"] == "Running" && job["Completed"].as_u64().is_some_and(|c| c >= 1) {
            seen_running = true;
            assert!(job["Progress"].as_u64().unwrap() < 100, "{job}");
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(seen_running, "progress is visible while the move runs");
    let job = job_over(&app, &path).await;
    assert_eq!(job["State"], "Success");
    assert_eq!(
        (job["Completed"].as_u64(), job["Progress"].as_u64()),
        (Some(4), Some(100))
    );
    assert_eq!(job["Type"], "C-MOVE");
    assert_eq!(job["StudyInstanceUIDs"], json!(["2.25.1"]));
}

/// `POST /jobs/{id}/cancel` sends C-CANCEL-MOVE-RQ; the job ends Cancelled with the counts
/// of the Cancel response (C.4.2.2.1). A job over cannot be cancelled: 409.
#[tokio::test]
async fn a_running_move_can_be_cancelled() {
    let (port, scp) = slow_move_scp(50, 200).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_remote(dir.path(), port).await;
    let (_, started) = send(
        &app,
        move_request(
            "qr",
            r#"{"StudyInstanceUID": "2.25.1", "Asynchronous": true}"#,
        ),
    )
    .await;
    let path = started["Path"].as_str().unwrap().to_owned();
    let cancel = || {
        Request::post(format!("{path}/cancel"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(send(&app, cancel()).await.0, StatusCode::ACCEPTED);
    let job = job_over(&app, &path).await;
    assert_eq!(job["State"], "Cancelled", "{job}");
    assert!(job["Remaining"].as_u64().unwrap() > 0, "{job}");
    assert!(scp.await.unwrap(), "the SCP got a C-CANCEL");
    assert_eq!(send(&app, cancel()).await.0, StatusCode::CONFLICT);
    assert_eq!(get(&app, "/jobs/999").await.0, StatusCode::NOT_FOUND);
    assert_eq!(get(&app, "/jobs/nonsense").await.0, StatusCode::NOT_FOUND);
}

/// A move that fails in the background ends Failure, with the reason in `Error`.
#[tokio::test]
async fn a_failed_asynchronous_move_says_why() {
    let (port, _scp) = move_scp(0xA801, 0, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_remote(dir.path(), port).await;
    let (_, started) = send(
        &app,
        move_request(
            "qr",
            r#"{"StudyInstanceUID": "2.25.1", "Asynchronous": true}"#,
        ),
    )
    .await;
    let job = job_over(&app, started["Path"].as_str().unwrap()).await;
    assert_eq!(job["State"], "Failure");
    assert!(
        job["Error"].as_str().unwrap().contains("registered"),
        "{job}"
    );
}

/// `"Project": "<name>"` moves the studies to that project's AE title, so they are
/// de-identified on the way in; an unknown project is 400, before any association.
#[tokio::test]
async fn a_move_may_name_a_project() {
    let (port, scp) = move_scp(0x0000, 1, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let projects = dir.path().join("projects.json");
    std::fs::write(
        &projects,
        r#"[{"ae_title": "LIVER_2026", "profile": {"name": "liver-2026"}}]"#,
    )
    .unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let settings = settings_in(
        dir.path(),
        &[
            "--dimse-remotes",
            &remotes,
            "--dimse-listen",
            "127.0.0.1:0",
            "--projects",
            projects.to_str().unwrap(),
        ],
    );
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, _) = send(
        &app,
        move_request(
            "qr",
            r#"{"StudyInstanceUID": "2.25.1", "Project": "nobody"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = send(
        &app,
        move_request(
            "qr",
            r#"{"StudyInstanceUID": "2.25.1", "Project": "liver-2026"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(scp.await.unwrap(), "LIVER_2026");
}

// ---- the audit trail of DIMSE and of the remote AEs (`pacs_server::audit`) ----

fn audited_settings(dir: &Path, extra: &[&str]) -> Settings {
    let trail = dir.join("audit.log");
    let mut all = vec!["--audit-file", trail.to_str().unwrap()];
    all.extend(extra);
    settings_in(dir, &all)
}

/// The XML of the trail's lines, each valid by DICOM's schema when `PACS_TEST_XMLLINT`
/// names `xmllint`; waits for `count` of them, since an association is audited once it
/// is over.
async fn audit_lines(dir: &Path, count: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for _ in 0..100 {
        let text = std::fs::read_to_string(dir.join("audit.log")).unwrap_or_default();
        lines = text
            .lines()
            .map(|line| {
                line.split_once(" - ")
                    .expect("a syslog header")
                    .1
                    .to_owned()
            })
            .collect();
        if lines.len() >= count {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(lines.len(), count, "{lines:#?}");
    if let Ok(xmllint) = std::env::var("PACS_TEST_XMLLINT") {
        let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../pacs-audit/tests/dicom-audit-message.rng");
        for xml in &lines {
            let file = dir.join("message.xml");
            std::fs::write(&file, xml).unwrap();
            let output = std::process::Command::new(&xmllint)
                .arg("--noout")
                .arg("--relaxng")
                .arg(&schema)
                .arg(&file)
                .output()
                .unwrap_or_else(|e| {
                    panic!("PACS_TEST_XMLLINT names {xmllint:?}, which did not run: {e}")
                });
            assert!(
                output.status.success(),
                "{}\n{xml}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    lines
}

fn has(line: &str, parts: &[&str]) {
    for part in parts {
        assert!(line.contains(part), "no {part:?} in\n{line}");
    }
}

/// One association from `calling` to `called` that stores `files` of CT, then releases.
async fn store_over(addr: SocketAddr, calling: &str, called: &str, files: &[(String, Vec<u8>)]) {
    let mut association = ClientAssociationOptions::new()
        .calling_ae_title(calling)
        .called_ae_title(called)
        .with_presentation_context(CT, vec!["1.2.840.10008.1.2.1"])
        .establish_async(addr)
        .await
        .unwrap();
    let context = association.presentation_contexts()[0].id;
    for (id, (sop_instance_uid, file)) in files.iter().enumerate() {
        let meta = u32::from_le_bytes(file[140..144].try_into().unwrap()) as usize;
        let rq = Command::new()
            .with_ui(tag::AFFECTED_SOP_CLASS_UID, CT)
            .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
            .with_us(tag::MESSAGE_ID, id as u16 + 1)
            .with_us(tag::PRIORITY, PRIORITY_MEDIUM)
            .with_us(tag::COMMAND_DATA_SET_TYPE, 0)
            .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, sop_instance_uid);
        message::send(&mut association, context, &rq, Some(&file[144 + meta..]))
            .await
            .unwrap();
        let Incoming::Message(rsp) = message::receive(&mut association, 0).await.unwrap() else {
            panic!("no C-STORE-RSP");
        };
        assert_eq!(rsp.command.status(), Ok(pacs_dimse::Status::SUCCESS));
    }
    association.release().await.unwrap();
}

#[tokio::test]
async fn what_an_association_stored_and_one_that_was_refused_are_in_the_audit_trail() {
    let dir = tempfile::tempdir().unwrap();
    let settings = audited_settings(dir.path(), &[]);
    let archive = open_archive(&settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(pacs_server::serve_dimse_audited(
        listener,
        archive,
        dimse_config(&settings).unwrap(),
        Vec::new(),
        pacs_server::audit::open_audit(&settings).unwrap(),
        std::future::pending(),
    ));

    // Two studies of two patients in one association: a line each, once it is over.
    let first = SyntheticSeries::new(61, 2).with_patient("DOE^JANE", None);
    let second = SyntheticSeries::new(62, 1).with_patient("ROE^RICHARD", None);
    let files = [
        (first.instance_uid(0), first.slice_bytes(0)),
        (second.instance_uid(0), second.slice_bytes(0)),
        (first.instance_uid(1), first.slice_bytes(1)),
    ];
    store_over(addr, "MODALITY1", "PACSRS", &files).await;
    let lines = audit_lines(dir.path(), 2).await;
    let sender = "<ActiveParticipant UserID=\"MODALITY1\" AlternativeUserID=\"AETITLES=MODALITY1\" UserIsRequestor=\"true\" NetworkAccessPointID=\"127.0.0.1\" NetworkAccessPointTypeCode=\"2\"><RoleIDCode csd-code=\"110153\"";
    has(
        &lines[0],
        &[
            "<EventID csd-code=\"110104\"",
            "EventActionCode=\"C\"",
            sender,
            "AlternativeUserID=\"AETITLES=PACSRS\" UserIsRequestor=\"false\"><RoleIDCode csd-code=\"110152\"",
            &format!("ParticipantObjectID=\"{}\"", first.study_uid()),
            &format!("<SOPClass UID=\"{CT}\" NumberOfInstances=\"2\"/>"),
            "<ParticipantObjectName>DOE^JANE</ParticipantObjectName>",
        ],
    );
    has(
        &lines[1],
        &[
            "EventActionCode=\"C\"",
            &format!("ParticipantObjectID=\"{}\"", second.study_uid()),
            &format!("<SOPClass UID=\"{CT}\" NumberOfInstances=\"1\"/>"),
            "ROE^RICHARD",
        ],
    );

    // The same study sent again was held already: a Read. Every association is written.
    store_over(addr, "MODALITY1", "PACSRS", &files[..1]).await;
    let lines = audit_lines(dir.path(), 3).await;
    has(
        &lines[2],
        &[
            "<EventID csd-code=\"110104\"",
            "EventActionCode=\"R\"",
            sender,
        ],
    );

    // An association that stores nothing (an echo would be one) leaves nothing; one to an
    // AE title this server does not have is a node that was not let in, once however
    // often it tries.
    store_over(addr, "MODALITY1", "PACSRS", &[]).await;
    for _ in 0..2 {
        let refused = ClientAssociationOptions::new()
            .calling_ae_title("MODALITY1")
            .called_ae_title("SOMEONE_ELSE")
            .with_presentation_context(CT, vec!["1.2.840.10008.1.2.1"])
            .establish_async(addr)
            .await;
        assert!(refused.is_err());
    }
    let lines = audit_lines(dir.path(), 4).await;
    has(
        &lines[3],
        &[
            "<EventID csd-code=\"110113\"",
            "originalText=\"Node Authentication\"",
            "EventOutcomeIndicator=\"4\"",
            "<ParticipantObjectIdentification ParticipantObjectID=\"127.0.0.1\"",
            "originalText=\"Node ID\"",
            "type=\"Alert Description\"",
        ],
    );
    // ...and nothing came after it
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    audit_lines(dir.path(), 4).await;
}

/// A C-FIND of this server's studies by `calling`, with pacs-rs's own SCU: how many
/// studies came back, or the error.
async fn query_as(
    addr: SocketAddr,
    calling: &str,
    patient_id: &str,
) -> Result<usize, pacs_dimse::FindError> {
    let remote: pacs_dimse::RemoteAe = format!("here=PACSRS@127.0.0.1:{}", addr.port())
        .parse()
        .unwrap();
    let query = pacs_dimse::StudyQuery {
        patient_id: Some(patient_id.to_owned()),
        ..pacs_dimse::StudyQuery::default()
    };
    let mut found = 0;
    pacs_dimse::find_studies(
        &remote,
        &pacs_dimse::AeTitle::new(calling).unwrap(),
        &query,
        std::time::Duration::from_secs(5),
        |_| {
            found += 1;
            std::ops::ControlFlow::Continue(())
        },
    )
    .await?;
    Ok(found)
}

/// pacs-rs queried over DIMSE: its own C-FIND SCU finds a study in its SCP, and the trail
/// has the query as a DICOM one (PS3.15 A.5.3.10), a caller that is not answered as an
/// alert, and neither twice.
#[tokio::test]
async fn a_query_over_dimse_and_one_that_is_refused_are_in_the_audit_trail() {
    let dir = tempfile::tempdir().unwrap();
    let settings = audited_settings(
        dir.path(),
        &["--dimse-remotes", "viewer=VIEWER@127.0.0.1:11112"],
    );
    let archive = open_archive(&settings).await.unwrap();
    let series = SyntheticSeries::new(71, 1).with_patient("DOE^JANE", None);
    archive.ingest(series.slice_bytes(0).into()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(pacs_server::serve_dimse_audited(
        listener,
        archive,
        dimse_config(&settings).unwrap(),
        Vec::new(),
        pacs_server::audit::open_audit(&settings).unwrap(),
        std::future::pending(),
    ));

    assert_eq!(query_as(addr, "VIEWER", "SYN71").await.unwrap(), 1);
    let lines = audit_lines(dir.path(), 1).await;
    has(
        &lines[0],
        &[
            "<EventID csd-code=\"110112\"",
            "EventActionCode=\"E\"",
            "EventOutcomeIndicator=\"0\"",
            "<ActiveParticipant UserID=\"VIEWER\" AlternativeUserID=\"AETITLES=VIEWER\" UserIsRequestor=\"true\" NetworkAccessPointID=\"127.0.0.1\" NetworkAccessPointTypeCode=\"2\"><RoleIDCode csd-code=\"110153\"",
            "AlternativeUserID=\"AETITLES=PACSRS\" UserIsRequestor=\"false\"><RoleIDCode csd-code=\"110152\"",
            "ParticipantObjectID=\"1.2.840.10008.5.1.4.1.2.2.1\" ParticipantObjectTypeCode=\"2\" ParticipantObjectTypeCodeRole=\"3\"",
            "originalText=\"SOP Class UID\"",
            "<ParticipantObjectQuery>",
            // 1.2.840.10008.1.2.1, the transfer syntax the SCU proposes first
            "<ParticipantObjectDetail type=\"TransferSyntax\" value=\"MS4yLjg0MC4xMDAwOC4xLjIuMQ==\"/>",
        ],
    );
    // the Identifier is there as it was sent: the Patient ID asked for is in it
    let query = lines[0]
        .split_once("<ParticipantObjectQuery>")
        .and_then(|(_, rest)| rest.split_once("</ParticipantObjectQuery>"))
        .unwrap()
        .0;
    use base64::Engine;
    let identifier = base64::engine::general_purpose::STANDARD
        .decode(query)
        .unwrap();
    assert!(identifier.windows(5).any(|bytes| bytes == b"SYN71"));

    // the same search again leaves no second line; another search does
    assert_eq!(query_as(addr, "VIEWER", "SYN71").await.unwrap(), 1);
    assert_eq!(query_as(addr, "VIEWER", "NOBODY").await.unwrap(), 0);
    let lines = audit_lines(dir.path(), 2).await;
    has(&lines[1], &["<EventID csd-code=\"110112\""]);

    // a caller this server does not answer: refused, and an alert, once however often
    for _ in 0..2 {
        let refused = query_as(addr, "STRANGER", "SYN71").await.unwrap_err();
        assert!(
            matches!(
                &refused,
                pacs_dimse::FindError::Failed { status, .. }
                    if *status == pacs_dimse::Status::NOT_AUTHORIZED
            ),
            "{refused}"
        );
    }
    let lines = audit_lines(dir.path(), 3).await;
    has(
        &lines[2],
        &[
            "<EventID csd-code=\"110113\"",
            "originalText=\"Access Control Decision\"",
            "EventOutcomeIndicator=\"4\"",
            "UserID=\"STRANGER\" AlternativeUserID=\"AETITLES=STRANGER\" UserIsRequestor=\"false\"",
            "ParticipantObjectID=\"urn:oid:1.2.840.10008.5.1.4.1.2.2.1\"",
            "type=\"Alert Description\"",
        ],
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    audit_lines(dir.path(), 3).await;
}

#[tokio::test]
async fn a_remote_search_and_a_move_are_in_the_audit_trail() {
    use pacs_server::app_with_audit;
    use pacs_server::audit::open_audit;

    let (port, scp) = query_scp(&["P1"]).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port},gone=NOBODY@127.0.0.1:9");
    let extra = [
        "--dimse-remotes",
        remotes.as_str(),
        "--dimse-listen",
        "127.0.0.1:0",
        "--dimse-ae-title",
        "RESEARCH",
    ];
    let settings = audited_settings(dir.path(), &extra);
    let archive = open_archive(&settings).await.unwrap();
    let app = app_with_audit(archive, &settings, None, open_audit(&settings).unwrap()).unwrap();
    let here = "AlternativeUserID=\"AETITLES=RESEARCH\" UserIsRequestor=\"false\">";
    let there = "<ActiveParticipant UserID=\"QRSCP\" AlternativeUserID=\"AETITLES=QRSCP\" UserIsRequestor=\"false\" NetworkAccessPointID=\"127.0.0.1\" NetworkAccessPointTypeCode=\"1\">";
    // the login is off in this test: who asked is known by address alone
    let who = "<ActiveParticipant UserID=\"unknown@pacs-rs\" UserIsRequestor=\"true\"";

    // A search: this server asks the remote AE, for the caller.
    let (status, _) = get(&app, "/modalities/qr/studies?PatientID=P1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scp.await.unwrap(), "P1");
    let lines = audit_lines(dir.path(), 1).await;
    let query = &lines[0];
    has(
        query,
        &[
            "<EventID csd-code=\"110112\"",
            "EventOutcomeIndicator=\"0\"",
            here,
            there,
            who,
            "ParticipantObjectID=\"/modalities/qr/studies\"",
        ],
    );
    let (asks, answers, person) = (
        query.find(here).unwrap(),
        query.find(there).unwrap(),
        query.find(who).unwrap(),
    );
    assert!(asks < answers && answers < person, "{query}");
    assert!(
        query[asks..answers].contains("csd-code=\"110153\"")
            && query[answers..person].contains("csd-code=\"110152\"")
    );
    // what was asked: the request's target, in base64 ("/modalities/qr/studies?PatientID=P1")
    has(
        query,
        &[
            "<ParticipantObjectQuery>L21vZGFsaXRpZXMvcXIvc3R1ZGllcz9QYXRpZW50SUQ9UDE=</ParticipantObjectQuery>",
        ],
    );

    // A remote AE that does not answer was asked all the same: a failure, written.
    let (status, _) = get(&app, "/modalities/gone/studies?PatientID=P2").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let lines = audit_lines(dir.path(), 2).await;
    has(
        &lines[1],
        &[
            "<EventID csd-code=\"110112\"",
            "EventOutcomeIndicator=\"8\"",
            "UserID=\"NOBODY\"",
        ],
    );
    // an AE this server does not know is not called
    assert_eq!(
        get(&app, "/modalities/nobody/studies?PatientID=P1").await.0,
        StatusCode::NOT_FOUND
    );
    audit_lines(dir.path(), 2).await;

    // A move: the remote AE is to send two studies here; a line each, before they arrive.
    let (port, scp) = move_scp(0x0000, 3, 0).await;
    let dir = tempfile::tempdir().unwrap();
    let remotes = format!("qr=QRSCP@127.0.0.1:{port}");
    let extra = [
        "--dimse-remotes",
        remotes.as_str(),
        "--dimse-listen",
        "127.0.0.1:0",
        "--dimse-ae-title",
        "RESEARCH",
    ];
    let settings = audited_settings(dir.path(), &extra);
    let archive = open_archive(&settings).await.unwrap();
    let app = app_with_audit(archive, &settings, None, open_audit(&settings).unwrap()).unwrap();
    // a body that names no study moves nothing
    assert_eq!(
        send(&app, move_request("qr", r#"{"StudyInstanceUID": 7}"#))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    audit_lines(dir.path(), 0).await;
    let (status, body) = send(
        &app,
        move_request("qr", r#"{"StudyInstanceUID": ["2.25.1", "2.25.2"]}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(scp.await.unwrap(), "RESEARCH");
    let lines = audit_lines(dir.path(), 2).await;
    for (line, study) in lines.iter().zip(["2.25.1", "2.25.2"]) {
        has(
            line,
            &[
                "<EventID csd-code=\"110102\"",
                "EventActionCode=\"E\"",
                "EventOutcomeIndicator=\"0\"",
                there,
                here,
                who,
                &format!("ParticipantObjectID=\"{study}\" ParticipantObjectTypeCode=\"2\""),
                // whose it is, the remote AE knows; this server does not yet
                "ParticipantObjectID=\"unknown\" ParticipantObjectTypeCode=\"1\"",
            ],
        );
        // the remote AE sends, this server receives
        let (sends, receives) = (line.find(there).unwrap(), line.find(here).unwrap());
        assert!(sends < receives, "{line}");
        assert!(line[sends..receives].contains("csd-code=\"110153\""));
    }
}
