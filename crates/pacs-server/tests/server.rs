use std::collections::HashMap;
use std::path::Path;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use clap::Parser;
use http_body_util::BodyExt;
use pacs_core::synthetic::SyntheticSeries;
use pacs_dicomweb::multipart;
use pacs_server::{Settings, StorageLocation, app, open_archive, secret, serve};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

fn settings_in(dir: &Path, extra: &[&str]) -> Settings {
    let index = dir.join("db/index.sqlite");
    let storage = format!("file:{}", dir.join("files").display());
    let mut args = vec![
        "pacs-server".to_owned(),
        "--index".into(),
        index.display().to_string(),
        "--storage".into(),
        storage,
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    Settings::try_parse_from(args).unwrap()
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, axum::http::HeaderMap, Bytes) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    (
        status,
        headers,
        response.into_body().collect().await.unwrap().to_bytes(),
    )
}

fn stow_request(uri: &str, files: Vec<Vec<u8>>) -> Request<Body> {
    let boundary = multipart::new_boundary();
    let body = multipart::body(
        &boundary,
        files
            .into_iter()
            .map(|f| ("application/dicom".to_owned(), Bytes::from(f))),
    );
    Request::post(uri)
        .header("host", "pacs.test")
        .header(
            "content-type",
            format!(r#"multipart/related; type="application/dicom"; boundary={boundary}"#),
        )
        .body(Body::from(body))
        .unwrap()
}

#[test]
fn storage_locations() {
    assert_eq!(
        StorageLocation::parse("file:data/files").unwrap(),
        StorageLocation::Directory("data/files".into())
    );
    assert_eq!(
        StorageLocation::parse("s3://dicom").unwrap(),
        StorageLocation::S3 {
            bucket: "dicom".into(),
            prefix: String::new()
        }
    );
    assert_eq!(
        StorageLocation::parse("s3://dicom/pacs-rs/").unwrap(),
        StorageLocation::S3 {
            bucket: "dicom".into(),
            prefix: "pacs-rs".into()
        }
    );
    for bad in ["", "file:", "s3://", "/data", "http://x"] {
        assert!(StorageLocation::parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn secrets_from_the_environment_or_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("key");
    std::fs::write(&file, "from-file\n").unwrap();
    let env: HashMap<&str, String> = [
        ("DIRECT", "direct".to_owned()),
        ("VIA_FILE", file.display().to_string()),
        ("BROKEN_FILE", "/nonexistent/key".to_owned()),
    ]
    .into();
    let lookup = |name: &str| env.get(name).cloned();
    assert_eq!(secret("DIRECT", lookup).unwrap().as_deref(), Some("direct"));
    // VIA is unset, so VIA_FILE names the file; its newline is dropped
    assert_eq!(secret("VIA", lookup).unwrap().as_deref(), Some("from-file"));
    assert_eq!(secret("ABSENT", lookup).unwrap(), None);
    assert!(secret("BROKEN", lookup).is_err());
}

#[tokio::test]
async fn nothing_listening_is_unhealthy() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert!(!pacs_server::healthy(address).await);
}

#[test]
fn defaults() {
    let settings = Settings::try_parse_from(["pacs-server"]).unwrap();
    assert_eq!(settings.listen.to_string(), "127.0.0.1:8080");
    assert_eq!(settings.dicomweb_path, "/dicom-web");
    assert_eq!(settings.storage, "file:data/files");
    assert!(settings.cors_origins.is_empty());
}

#[tokio::test]
async fn health_and_dicomweb_under_its_path_and_data_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let series = SyntheticSeries::new(71, 2);
    {
        let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
        let (status, _, body) =
            call(&app, Request::get("/healthz").body(Body::empty()).unwrap()).await;
        assert_eq!((status, &body[..]), (StatusCode::OK, &b"ok\n"[..]));

        let (status, _, _) =
            call(&app, stow_request("/dicom-web/studies", series.all_bytes())).await;
        assert_eq!(status, StatusCode::OK);
        // STOW is DICOMweb's; at the root, /studies is the REST API's
        let (status, _, _) = call(&app, stow_request("/studies", series.all_bytes())).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }
    assert!(dir.path().join("db/index.sqlite").is_file());
    let files = dir
        .path()
        .join("files")
        .join(series.study_uid())
        .join(series.series_uid());
    assert_eq!(std::fs::read_dir(&files).unwrap().count(), 2);

    // a new process, the same directories
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, _, body) = call(
        &app,
        Request::get("/dicom-web/studies")
            .header("host", "pacs.test")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let studies: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(studies[0]["00201208"]["Value"][0], 2);
    assert_eq!(
        studies[0]["00081190"]["Value"][0],
        format!("http://pacs.test/dicom-web/studies/{}", series.study_uid())
    );
}

#[tokio::test]
async fn web_interface_at_ui_unless_turned_off() {
    let dir = tempfile::tempdir().unwrap();
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    let settings = settings_in(dir.path(), &["--dicomweb-path", "/web"]);
    let with_ui = app(open_archive(&settings).await.unwrap(), &settings).unwrap();

    // relative redirects, so a proxy's path prefix is kept
    let (status, headers, _) = call(&with_ui, get("/")).await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(headers.get("location").unwrap(), "ui/");
    let (status, headers, _) = call(&with_ui, get("/ui")).await;
    assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    assert_eq!(headers.get("location").unwrap(), "ui/");

    // the built page, or the one saying how to build it
    let (status, headers, body) = call(&with_ui, get("/ui/")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let csp = headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert!(String::from_utf8_lossy(&body).contains("<title>pacs-rs</title>"));

    let (status, _, body) = call(&with_ui, get("/ui/config.json")).await;
    assert_eq!(status, StatusCode::OK);
    let config: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(config["dicomweb"], "/web");
    assert_eq!(config["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(
        call(&with_ui, get("/ui/no-such-file.js")).await.0,
        StatusCode::NOT_FOUND
    );

    let settings = settings_in(dir.path(), &["--ui", "false"]);
    let without = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    assert_eq!(call(&without, get("/ui/")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&without, get("/")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn hanging_protocols_are_shared_through_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let req = |method: &str, uri: &str, body: &str| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    };
    let settings = settings_in(dir.path(), &[]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();

    let (status, _, body) = call(&app, req("GET", "/ui/api/protocols", "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"[]");

    let mine = r#"{"id":"user:a1","name":"Chest PA and lateral","layout":1}"#;
    let (status, _, _) = call(&app, req("PUT", "/ui/api/protocols/user:a1", mine)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let renamed = r#"{"id":"user:a1","name":"Chest","layout":1}"#;
    assert_eq!(
        call(&app, req("PUT", "/ui/api/protocols/user:a1", renamed))
            .await
            .0,
        StatusCode::NO_CONTENT
    );

    // refused: an id that is not the path's, not an object, a bad id, too big
    for (uri, body) in [
        ("/ui/api/protocols/user:a1", r#"{"id":"user:b2"}"#),
        ("/ui/api/protocols/user:a1", "[1, 2]"),
        ("/ui/api/protocols/a%20b", r#"{"id":"a b"}"#),
    ] {
        assert_eq!(
            call(&app, req("PUT", uri, body)).await.0,
            StatusCode::BAD_REQUEST,
            "{uri} {body}"
        );
    }
    let huge = format!(r#"{{"id":"user:big","pad":"{}"}}"#, "x".repeat(70_000));
    assert_eq!(
        call(&app, req("PUT", "/ui/api/protocols/user:big", &huge))
            .await
            .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // another browser, after a restart, sees it
    drop(app);
    let app = app_again(&settings).await;
    let (_, _, body) = call(&app, req("GET", "/ui/api/protocols", "")).await;
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "Chest");

    assert_eq!(
        call(&app, req("DELETE", "/ui/api/protocols/user:a1", ""))
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, req("DELETE", "/ui/api/protocols/user:a1", ""))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let settings = settings_in(dir.path(), &["--ui", "false"]);
    let without = app_again(&settings).await;
    assert_eq!(
        call(&without, req("GET", "/ui/api/protocols", "")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn measurements_are_kept_per_study() {
    let dir = tempfile::tempdir().unwrap();
    let req = |method: &str, uri: &str, body: &str| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_owned()))
            .unwrap()
    };
    let settings = settings_in(dir.path(), &[]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let series = SyntheticSeries::new(82, 1);
    let (status, _, body) = call(
        &app,
        Request::post("/instances")
            .body(Body::from(series.slice_bytes(0)))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let study_id = serde_json::from_slice::<Value>(&body).unwrap()["ParentStudy"]
        .as_str()
        .unwrap()
        .to_owned();
    let uri = format!("/ui/api/annotations/{}", series.study_uid());

    let (status, _, body) = call(&app, req("GET", &uri, "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), b"[]");
    let two = r#"[{"annotationUID":"a","metadata":{"toolName":"Length"}},{"annotationUID":"b"}]"#;
    assert_eq!(
        call(&app, req("PUT", &uri, two)).await.0,
        StatusCode::NO_CONTENT
    );

    // refused: not an array, not objects, not a UID, a study that isn't here, too big
    for (uri, body, status) in [
        (uri.as_str(), r#"{"a":1}"#, StatusCode::BAD_REQUEST),
        (uri.as_str(), "[1]", StatusCode::BAD_REQUEST),
        ("/ui/api/annotations/abc", "[]", StatusCode::BAD_REQUEST),
        ("/ui/api/annotations/1.2.3.4", "[]", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            call(&app, req("PUT", uri, body)).await.0,
            status,
            "{uri} {body}"
        );
    }
    let huge = format!(r#"[{{"pad":"{}"}}]"#, "x".repeat(1_100_000));
    assert_eq!(
        call(&app, req("PUT", &uri, &huge)).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // another browser, after a restart, gets them
    drop(app);
    let app = app_again(&settings).await;
    let (_, _, body) = call(&app, req("GET", &uri, "")).await;
    let list: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["metadata"]["toolName"], "Length");

    // an empty list removes them; deleting the study too
    assert_eq!(
        call(&app, req("PUT", &uri, "[]")).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(call(&app, req("GET", &uri, "")).await.2.as_ref(), b"[]");
    assert_eq!(
        call(&app, req("PUT", &uri, two)).await.0,
        StatusCode::NO_CONTENT
    );
    let (status, _, _) = call(&app, req("DELETE", &format!("/studies/{study_id}"), "")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(call(&app, req("GET", &uri, "")).await.2.as_ref(), b"[]");

    let settings = settings_in(dir.path(), &["--ui", "false"]);
    let without = app_again(&settings).await;
    assert_eq!(
        call(&without, req("GET", &uri, "")).await.0,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_study_list_comes_a_page_at_a_time() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    for seed in [83, 84, 85] {
        let bytes = SyntheticSeries::new(seed, 1).slice_bytes(0);
        let request = Request::post("/instances").body(Body::from(bytes)).unwrap();
        assert_eq!(call(&app, request).await.0, StatusCode::OK);
    }
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    let json = |body: Bytes| serde_json::from_slice::<Value>(&body).unwrap();

    let (status, _, body) = call(&app, get("/ui/api/studies?limit=2&sort=patient&order=asc")).await;
    assert_eq!(status, StatusCode::OK);
    let page = json(body);
    assert_eq!(page["Total"], 3);
    assert_eq!(page["Studies"].as_array().unwrap().len(), 2);
    assert_eq!(page["Studies"][0]["Modalities"][0], "CT");
    assert_eq!(page["Studies"][0]["Instances"], 1);
    assert!(page.get("Summary").is_none());
    let (_, _, body) = call(
        &app,
        get("/ui/api/studies?limit=2&offset=2&sort=patient&order=asc"),
    )
    .await;
    assert_eq!(json(body)["Studies"].as_array().unwrap().len(), 1);

    // a word, and the summary of what it finds
    let (_, _, body) = call(&app, get("/ui/api/studies?q=syn84&summary=true")).await;
    let found = json(body);
    assert_eq!(found["Total"], 1);
    assert_eq!(found["Summary"]["Studies"], 1);
    assert_eq!(found["Summary"]["Modalities"][0], json!(["CT", 1]));
    assert_eq!(found["Summary"]["Recent"].as_array().unwrap().len(), 1);
    let (_, _, body) = call(&app, get("/ui/api/studies?modality=MR")).await;
    assert_eq!(json(body)["Total"], 0);

    let (_, _, body) = call(&app, get("/ui/api/studies/facets")).await;
    assert_eq!(json(body)["Modalities"]["CT"], 3);

    for bad in [
        "sort=size",
        "order=up",
        "limit=0",
        "limit=1001",
        "offset=-1",
        "from=2026-01-01",
    ] {
        let (status, _, _) = call(&app, get(&format!("/ui/api/studies?{bad}"))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    let settings = settings_in(dir.path(), &["--ui", "false"]);
    let without = app_again(&settings).await;
    assert_eq!(
        call(&without, get("/ui/api/studies")).await.0,
        StatusCode::NOT_FOUND
    );
}

async fn app_again(settings: &Settings) -> Router {
    app(open_archive(settings).await.unwrap(), settings).unwrap()
}

#[tokio::test]
async fn cors_only_when_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let preflight = || {
        Request::options("/dicom-web/studies")
            .header("origin", "https://viewer.example")
            .header("access-control-request-method", "GET")
            .body(Body::empty())
            .unwrap()
    };
    let settings = settings_in(dir.path(), &[]);
    let plain = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (_, headers, _) = call(&plain, preflight()).await;
    assert!(headers.get("access-control-allow-origin").is_none());

    let settings = settings_in(
        dir.path(),
        &[
            "--cors-origins",
            "https://viewer.example,https://other.example",
        ],
    );
    let with_cors = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let (status, headers, _) = call(&with_cors, preflight()).await;
    assert!(status.is_success());
    assert_eq!(
        headers.get("access-control-allow-origin").unwrap(),
        "https://viewer.example"
    );
}

#[tokio::test]
async fn serves_over_tcp_and_shuts_down_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve(listener, app, async {
        let _ = stopped.await;
    }));

    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.ends_with("ok\n"), "{response}");

    assert!(pacs_server::healthy(address).await);
    let wildcard = std::net::SocketAddr::new([0, 0, 0, 0].into(), address.port());
    assert!(
        pacs_server::healthy(wildcard).await,
        "0.0.0.0 is checked on loopback"
    );

    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .expect("the server stops")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn the_rest_api_is_at_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &[]);
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let series = SyntheticSeries::new(72, 1);
    let (status, _, body) = call(
        &app,
        Request::post("/instances")
            .body(Body::from(series.slice_bytes(0)))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let reply: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["Status"], "Success");
    let (status, _, body) = call(&app, Request::get("/studies").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap()[0],
        reply["ParentStudy"]
    );
}

#[tokio::test]
async fn dicomweb_cannot_take_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let settings = settings_in(dir.path(), &["--dicomweb-path", "/"]);
    let archive = open_archive(&settings).await.unwrap();
    assert!(app(archive, &settings).is_err());
}

#[test]
fn index_locations_and_passwords() {
    use pacs_server::{IndexLocation, with_password, without_password};
    assert_eq!(
        IndexLocation::parse("data/index.sqlite"),
        IndexLocation::Sqlite("data/index.sqlite".into())
    );
    assert_eq!(
        IndexLocation::parse("postgres://pacs@db/pacs"),
        IndexLocation::Postgres("postgres://pacs@db/pacs".into())
    );
    assert_eq!(
        with_password("postgres://pacs@db:5432/pacs", "p@ss:w/rd").unwrap(),
        "postgres://pacs:p%40ss%3Aw%2Frd@db:5432/pacs"
    );
    assert!(with_password("postgres://pacs:x@db/pacs", "y").is_err());
    assert!(with_password("postgres://db/pacs", "y").is_err());
    assert_eq!(
        without_password("postgres://pacs:secret@db/pacs"),
        "postgres://pacs:***@db/pacs"
    );
}

/// The whole server on a Postgres index, when `PACS_TEST_POSTGRES_URL` names a server.
#[tokio::test]
async fn the_server_on_postgres() {
    let Ok(server) = std::env::var("PACS_TEST_POSTGRES_URL") else {
        eprintln!("PACS_TEST_POSTGRES_URL not set; skipping");
        return;
    };
    let server = server.trim_end_matches('/');
    let name = format!("pacs_server_test_{}", std::process::id());
    sqlx::any::install_default_drivers();
    let mut admin =
        <sqlx::AnyConnection as sqlx::Connection>::connect(&format!("{server}/postgres"))
            .await
            .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name}"
    )))
    .execute(&mut admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut admin)
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let url = format!("{server}/{name}");
    let storage = format!("file:{}", dir.path().join("files").display());
    let settings =
        Settings::try_parse_from(["pacs-server", "--index", &url, "--storage", &storage]).unwrap();
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let series = SyntheticSeries::new(73, 3);
    let (status, _, _) = call(&app, stow_request("/dicom-web/studies", series.all_bytes())).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, body) = call(
        &app,
        Request::get(format!(
            "/dicom-web/studies?StudyInstanceUID={}",
            series.study_uid()
        ))
        .header("host", "pacs.test")
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let studies: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(studies[0]["00201208"]["Value"][0], 3);
    assert_eq!(studies[0]["00080061"]["Value"][0], "CT");
    let (_, _, body) = call(
        &app,
        Request::get("/studies?expand").body(Body::empty()).unwrap(),
    )
    .await;
    let rest: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        rest[0]["MainDicomTags"]["StudyInstanceUID"],
        series.study_uid()
    );
}
