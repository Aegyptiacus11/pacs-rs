//! The DIMSE side as pacs-server runs it: its settings, the `/modalities` routes (echo and a
//! C-FIND search), and an instance received by C-STORE showing in the REST API and the web
//! interface's list.

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
        ],
    );
    let config = dimse_config(&settings).unwrap();
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

    let (status, body) = send(&app, echo("gone")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert_eq!(body["Message"], "C-ECHO to gone failed");

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
