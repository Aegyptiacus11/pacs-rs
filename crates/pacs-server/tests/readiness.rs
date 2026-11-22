//! `GET /readyz`: ready when the index and the store answer, 503 naming the part that does
//! not; open without a token, as `/healthz` is.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_server::auth::is_open;
use pacs_server::{Settings, app, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

async fn server() -> (Router, Archive, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.sqlite").display().to_string();
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings =
        Settings::try_parse_from(["pacs-server", "--index", &index, "--storage", &storage])
            .unwrap();
    let archive = open_archive(&settings).await.unwrap();
    (app(archive.clone(), &settings).unwrap(), archive, dir)
}

async fn readyz(app: &Router) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn a_server_whose_index_and_store_answer_is_ready() {
    let (app, _, _dir) = server().await;
    assert_eq!(
        readyz(&app).await,
        (StatusCode::OK, json!({"index": "ok", "store": "ok"}))
    );
}

#[tokio::test]
async fn an_index_that_does_not_answer_is_named() {
    let (app, archive, _dir) = server().await;
    archive.index().pool().close().await;
    let (status, body) = readyz(&app).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["store"], "ok");
    assert_ne!(body["index"], "ok", "{body}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_store_that_cannot_be_read_is_named() {
    use std::os::unix::fs::PermissionsExt;
    let (app, _, dir) = server().await;
    let files = dir.path().join("files");
    std::fs::set_permissions(&files, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&files).is_ok() {
        eprintln!("running as root: a directory without permissions is still readable; skipping");
        return;
    }
    let (status, body) = readyz(&app).await;
    std::fs::set_permissions(&files, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["index"], "ok");
    assert_ne!(body["store"], "ok", "{body}");
}

#[test]
fn readiness_is_open_like_health() {
    assert!(is_open(&Method::GET, "/readyz"));
    assert!(is_open(&Method::GET, "/healthz"));
}
