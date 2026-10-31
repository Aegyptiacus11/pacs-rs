//! The DIMSE side as pacs-server runs it: its settings, and an instance received by C-STORE
//! showing in the REST API and the web interface's list.

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
use serde_json::Value;
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
