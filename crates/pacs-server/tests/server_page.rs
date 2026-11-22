//! `GET /ui/api/server`, what the web interface's Server page shows: the archive's totals,
//! the associations being served, the jobs, the quarantine and the warnings and errors logged.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_server::auth::{Right, right_for};
use pacs_server::metrics::CountErrors;
use pacs_server::{Settings, app, open_archive};
use serde_json::Value;
use tower::ServiceExt;

async fn server(extra: &[&str]) -> (Router, Archive, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.path().join("files").display());
    let mut args = vec!["pacs-server", "--index", &index, "--storage", &storage];
    args.extend(extra);
    let settings = Settings::try_parse_from(args).unwrap();
    let archive = open_archive(&settings).await.unwrap();
    (app(archive.clone(), &settings).unwrap(), archive, dir)
}

async fn page(app: &Router) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::get("/ui/api/server").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn the_page_has_the_archive_what_is_open_and_what_was_logged() {
    let (app, archive, _dir) = server(&[]).await;
    for bytes in RealSeries::new(103, 2).all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    let open = pacs_core::metrics::OpenAssociation::new("10.1.2.3:4242", "CT_ROOM1", "PACSRS");
    open.served(7, 0, 0);
    let subscriber =
        tracing_subscriber::layer::SubscriberExt::with(tracing_subscriber::registry(), CountErrors);
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(target: "pacs_server_page_test", file = "a.dcm", "something to look at");
    });

    let (status, body) = page(&app).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["archive"]["studies"], 1);
    assert_eq!(body["archive"]["instances"], 2);
    assert!(body["archive"]["bytes"].as_i64().unwrap() > 1000);
    assert_eq!(body["archive"]["trash"], 0);
    let mine = body["associations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["peer"] == "10.1.2.3:4242")
        .unwrap_or_else(|| panic!("the open association: {body}"))
        .clone();
    assert_eq!(
        (mine["calling"].as_str(), mine["stored"].as_u64()),
        (Some("CT_ROOM1"), Some(7))
    );
    let logged = body["log"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["target"] == "pacs_server_page_test")
        .unwrap_or_else(|| panic!("the warning: {body}"))
        .clone();
    assert_eq!(logged["level"], "WARN");
    assert_eq!(logged["message"], "something to look at");
    assert_eq!(logged["fields"], "file=a.dcm");
    assert_eq!(body["jobs"], Value::Array(vec![]));
    // no PACS_QUARANTINE
    assert_eq!(body["quarantine"], Value::Null);
    drop(open);
    let (_, after) = page(&app).await;
    assert!(
        !after["associations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["peer"] == "10.1.2.3:4242")
    );
}

#[tokio::test]
async fn a_quarantine_is_counted() {
    let quarantine_dir = tempfile::tempdir().unwrap();
    std::fs::write(quarantine_dir.path().join("held.dcm"), b"0123456789").unwrap();
    let quarantine = format!("file:{}", quarantine_dir.path().display());
    let (app, _, _dir) = server(&["--quarantine", &quarantine]).await;
    let (_, body) = page(&app).await;
    assert_eq!(body["quarantine"]["files"], 1);
    assert_eq!(body["quarantine"]["bytes"], 10);
}

#[tokio::test]
async fn the_page_says_so_when_the_index_does_not_answer() {
    let (app, archive, _dir) = server(&[]).await;
    archive.index().pool().close().await;
    assert_eq!(page(&app).await.0, StatusCode::SERVICE_UNAVAILABLE);
}

#[test]
fn only_who_manages_users_sees_the_server_page() {
    assert_eq!(right_for(&Method::GET, "/ui/api/server"), Right::Admin);
}
