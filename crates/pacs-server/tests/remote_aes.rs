//! Remote AEs added, changed and removed through the REST API (`PUT` and `DELETE
//! /modalities/{id}`): used at once by the REST routes and by the DIMSE server, which share
//! one directory with them; kept across a restart; refused when they cannot be.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_dimse::{AeTitle, RemoteAe};
use pacs_server::api::remote::RemoteAes;
use pacs_server::auth::{Right, right_for};
use pacs_server::{Settings, app, dimse_config, open_archive, serve_dimse};
use serde_json::{Value, json};
use tower::ServiceExt;

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

async fn call(app: &Router, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A pacs-rs answering DIMSE as `title`, its own archive in `dir`.
async fn peer(dir: &Path, title: &str) -> SocketAddr {
    let settings = settings_in(dir, &["--dimse-ae-title", title]);
    let archive = open_archive(&settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve_dimse(
        listener,
        archive,
        dimse_config(&settings).unwrap(),
        Vec::new(),
        std::future::pending(),
    ));
    addr
}

/// The REST app and the DIMSE server of one pacs-rs, from the same settings, as `main` starts
/// them: the DIMSE server answers only the remote AEs it knows.
async fn server(dir: &Path, extra: &[&str]) -> (Router, SocketAddr) {
    let mut args = vec!["--dimse-known-callers-only", "--dimse-timeout", "3"];
    args.extend_from_slice(extra);
    let settings = settings_in(dir, &args);
    let archive = open_archive(&settings).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve_dimse(
        listener,
        archive.clone(),
        dimse_config(&settings).unwrap(),
        Vec::new(),
        std::future::pending(),
    ));
    (app(archive, &settings).unwrap(), addr)
}

/// Whether an association from `calling` to `server` is accepted (a C-ECHO succeeds).
async fn answers(server: SocketAddr, calling: &str) -> bool {
    let target = RemoteAe {
        id: "server".into(),
        ae_title: AeTitle::new("PACSRS").unwrap(),
        host: "127.0.0.1".into(),
        port: server.port(),
        secure: false,
        tls: None,
    };
    pacs_dimse::echo(
        &target,
        &AeTitle::new(calling).unwrap(),
        Duration::from_secs(3),
    )
    .await
    .is_ok()
}

#[tokio::test]
async fn a_remote_ae_added_is_used_at_once_kept_across_a_restart_and_removed() {
    let (peer_dir, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let peer = peer(peer_dir.path(), "PEER").await;
    let (app, dimse) = server(dir.path(), &[]).await;
    // unknown to the server: its DIMSE side turns PEER away, its REST side has no "peer"
    assert!(!answers(dimse, "PEER").await);
    assert_eq!(
        call(&app, Method::POST, "/modalities/peer/echo", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let body = json!({"AET": "PEER", "Host": "127.0.0.1", "Port": peer.port(), "MayReceive": true});
    let (status, added) = call(&app, Method::PUT, "/modalities/peer", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{added}");
    assert_eq!(
        added,
        json!({"AET": "PEER", "Host": "127.0.0.1", "Port": peer.port(), "TLS": false, "MayReceive": true, "Fixed": false})
    );
    // used at once, by both sides, with no restart
    let (status, echo) = call(&app, Method::POST, "/modalities/peer/echo", None).await;
    assert_eq!(status, StatusCode::OK, "{echo}");
    assert!(answers(dimse, "PEER").await);
    let (_, listed) = call(&app, Method::GET, "/modalities?expand", None).await;
    serde_json::from_value::<RemoteAes>(listed.clone()).unwrap();
    assert_eq!(listed["peer"]["Fixed"], false);

    // changed: no longer allowed to receive
    let body = json!({"AET": "PEER", "Host": "127.0.0.1", "Port": peer.port()});
    assert_eq!(
        call(&app, Method::PUT, "/modalities/peer", Some(body))
            .await
            .1["MayReceive"],
        false
    );

    // the next start finds it again
    let (again, _) = server(dir.path(), &[]).await;
    assert_eq!(
        call(&again, Method::GET, "/modalities", None).await.1,
        json!(["peer"])
    );

    assert_eq!(
        call(&again, Method::DELETE, "/modalities/peer", None)
            .await
            .0,
        StatusCode::OK
    );
    assert_eq!(
        call(&again, Method::DELETE, "/modalities/peer", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(&again, Method::POST, "/modalities/peer/echo", None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (third, dimse) = server(dir.path(), &[]).await;
    assert_eq!(
        call(&third, Method::GET, "/modalities", None).await.1,
        json!([])
    );
    assert!(!answers(dimse, "PEER").await);
}

#[tokio::test]
async fn what_cannot_be_a_remote_ae_is_refused_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = server(
        dir.path(),
        &[
            "--dimse-remotes",
            "hospital=HOSP_PACS@10.0.0.5:104",
            "--dimse-move-destinations",
            "hospital",
        ],
    )
    .await;
    let put = |id: &str, body: Value| (format!("/modalities/{id}"), body);
    for ((uri, body), status, why) in [
        (
            put(
                "hospital",
                json!({"AET": "X", "Host": "10.0.0.6", "Port": 104}),
            ),
            StatusCode::CONFLICT,
            "change it there",
        ),
        (
            put(
                "a%20b",
                json!({"AET": "X", "Host": "10.0.0.6", "Port": 104}),
            ),
            StatusCode::BAD_REQUEST,
            "is not an id",
        ),
        (
            put(
                "ws",
                json!({"AET": "WAY_TOO_LONG_TITLE_X", "Host": "10.0.0.6", "Port": 104}),
            ),
            StatusCode::BAD_REQUEST,
            "WAY_TOO_LONG",
        ),
        (
            put("ws", json!({"AET": "WS", "Host": "ws local", "Port": 104})),
            StatusCode::BAD_REQUEST,
            "neither a host name",
        ),
        (
            put("ws", json!({"AET": "WS", "Host": "10.0.0.6", "Port": 0})),
            StatusCode::BAD_REQUEST,
            "port 0",
        ),
        (
            put(
                "ws",
                json!({"AET": "WS", "Host": "10.0.0.6", "Port": 104, "TLS": true}),
            ),
            StatusCode::BAD_REQUEST,
            "no DIMSE TLS",
        ),
        (
            put("ws", json!({"AET": "WS", "Host": "10.0.0.6"})),
            StatusCode::BAD_REQUEST,
            "not a remote AE",
        ),
        (
            put(
                "ws",
                json!({"AET": "HOSP_PACS", "Host": "10.0.0.6", "Port": 104, "MayReceive": true}),
            ),
            StatusCode::CONFLICT,
            "could not tell them apart",
        ),
    ] {
        let (got, answer) = call(&app, Method::PUT, &uri, Some(body.clone())).await;
        assert_eq!(got, status, "{uri} {body}: {answer}");
        assert!(
            answer["Details"].as_str().unwrap_or_default().contains(why),
            "{uri} {body}: {answer}"
        );
    }
    // the same AE title, not receiving: two AEs may share one to be searched
    let body = json!({"AET": "HOSP_PACS", "Host": "10.0.0.6", "Port": 104});
    assert_eq!(
        call(&app, Method::PUT, "/modalities/mirror", Some(body))
            .await
            .0,
        StatusCode::OK
    );
    let (status, answer) = call(&app, Method::DELETE, "/modalities/hospital", None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        answer["Details"]
            .as_str()
            .unwrap()
            .contains("remove it there")
    );
    assert_eq!(
        call(&app, Method::GET, "/modalities", None).await.1,
        json!(["hospital", "mirror"])
    );
}

#[tokio::test]
async fn one_that_forwarding_sends_to_is_not_removed() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = server(dir.path(), &[]).await;
    let body = json!({"AET": "WS", "Host": "10.0.0.6", "Port": 104, "MayReceive": true});
    assert_eq!(
        call(&app, Method::PUT, "/modalities/ws", Some(body))
            .await
            .0,
        StatusCode::OK
    );
    // started again, forwarding what arrives to it
    let (app, _) = server(dir.path(), &["--forward", "ws"]).await;
    let (status, answer) = call(&app, Method::DELETE, "/modalities/ws", None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert!(answer["Details"].as_str().unwrap().contains("PACS_FORWARD"));
}

#[tokio::test]
async fn an_id_in_both_the_settings_and_the_index_stops_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let (app, _) = server(dir.path(), &[]).await;
    let body = json!({"AET": "WS", "Host": "10.0.0.6", "Port": 104});
    assert_eq!(
        call(&app, Method::PUT, "/modalities/ws", Some(body))
            .await
            .0,
        StatusCode::OK
    );
    let settings = settings_in(dir.path(), &["--dimse-remotes", "ws=OTHER@10.0.0.7:104"]);
    let Err(error) = open_archive(&settings).await else {
        panic!("the start is not refused");
    };
    assert!(
        format!("{error:#}")
            .contains("ws is in PACS_DIMSE_REMOTES and was also added in the web interface"),
        "{error:#}"
    );
}

#[test]
fn changing_remote_aes_needs_an_administrator() {
    assert_eq!(right_for(&Method::PUT, "/modalities/ws"), Right::Admin);
    assert_eq!(right_for(&Method::DELETE, "/modalities/ws"), Right::Admin);
    // using them stays with `remote`
    assert_eq!(right_for(&Method::GET, "/modalities"), Right::Remote);
    assert_eq!(
        right_for(&Method::POST, "/modalities/ws/echo"),
        Right::Remote
    );
    assert_eq!(
        right_for(&Method::POST, "/modalities/ws/store"),
        Right::Remote
    );
}
