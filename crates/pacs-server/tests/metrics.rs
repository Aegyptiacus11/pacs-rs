//! `GET /metrics` (`PACS_METRICS_LISTEN`): the Prometheus text format, what the server
//! counted as it went, and what the archive holds.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_archive::Archive;
use pacs_core::real::RealSeries;
use pacs_server::metrics::{CountErrors, render, router};
use pacs_server::{Settings, app, open_archive};
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

async fn get(app: &Router, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let response = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (status, headers) = (response.status(), response.headers().clone());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, String::from_utf8(bytes.to_vec()).unwrap())
}

/// The value of one series on the page; panics when it is not there.
fn value(page: &str, series: &str) -> f64 {
    page.lines()
        .find_map(|line| line.strip_prefix(&format!("{series} ")))
        .unwrap_or_else(|| panic!("no {series} in\n{page}"))
        .parse()
        .unwrap()
}

/// Each sample is `name{labels} value`, and each family has its HELP and TYPE first.
fn well_formed(page: &str) {
    let mut typed = std::collections::HashSet::new();
    for line in page.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let (name, kind) = rest.split_once(' ').unwrap();
            assert!(["counter", "gauge"].contains(&kind), "{line}");
            typed.insert(name.to_owned());
            continue;
        }
        if line.starts_with("# HELP ") {
            continue;
        }
        let (series, number) = line.rsplit_once(' ').unwrap_or_else(|| panic!("{line}"));
        number
            .parse::<f64>()
            .unwrap_or_else(|_| panic!("not a number: {line}"));
        let name = series.split('{').next().unwrap();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "{line}"
        );
        assert!(typed.contains(name), "a sample before its TYPE: {line}");
        if let Some(labels) = series.strip_prefix(name) {
            assert!(
                labels.is_empty() || (labels.starts_with('{') && labels.ends_with('}')),
                "{line}"
            );
        }
    }
}

#[tokio::test]
async fn the_page_counts_what_the_server_did_and_what_the_archive_holds() {
    let (app, archive, _dir) = server().await;
    let before = render(&archive).await.unwrap();
    let added_before = value(&before, "pacs_instances_stored_total{outcome=\"added\"}");
    let again_before = value(
        &before,
        "pacs_instances_stored_total{outcome=\"already_stored\"}",
    );
    let series = RealSeries::new(102, 2);
    for bytes in series.all_bytes() {
        archive.ingest(bytes.into()).await.unwrap();
    }
    archive.ingest(series.slice_bytes(0).into()).await.unwrap();
    assert_eq!(get(&app, "/studies").await.0, StatusCode::OK);
    assert_eq!(get(&app, "/studies/nope").await.0, StatusCode::NOT_FOUND);
    // an ERROR logged, through the layer the server installs
    let subscriber =
        tracing_subscriber::layer::SubscriberExt::with(tracing_subscriber::registry(), CountErrors);
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!(target: "pacs_metrics_test::here", "an error, for the count");
    });

    let page = render(&archive).await.unwrap();
    well_formed(&page);
    assert_eq!(value(&page, "pacs_studies"), 1.0);
    assert_eq!(value(&page, "pacs_instances"), 2.0);
    assert!(value(&page, "pacs_storage_bytes") > 1000.0);
    assert_eq!(value(&page, "pacs_files_failed"), 0.0);
    assert_eq!(value(&page, "pacs_trash_entries"), 0.0);
    // the counters are the process's: at least what this test did
    assert!(value(&page, "pacs_instances_stored_total{outcome=\"added\"}") >= added_before + 2.0);
    assert!(
        value(
            &page,
            "pacs_instances_stored_total{outcome=\"already_stored\"}"
        ) >= again_before + 1.0
    );
    assert!(
        value(
            &page,
            "pacs_http_requests_total{method=\"GET\",status=\"2xx\"}"
        ) >= 1.0
    );
    assert!(
        value(
            &page,
            "pacs_http_requests_total{method=\"GET\",status=\"4xx\"}"
        ) >= 1.0
    );
    assert!(value(&page, "pacs_errors_total{crate=\"pacs_metrics_test\"}") >= 1.0);
    assert!(page.contains(&format!(
        "pacs_build_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    )));
}

#[tokio::test]
async fn the_listener_serves_the_text_format_and_says_when_the_index_does_not_answer() {
    let (_, archive, _dir) = server().await;
    let metrics = router(archive.clone());
    let (status, headers, body) = get(&metrics, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers["content-type"],
        "text/plain; version=0.0.4; charset=utf-8"
    );
    well_formed(&body);
    // only /metrics there: nothing of the archive's data
    assert_eq!(get(&metrics, "/studies").await.0, StatusCode::NOT_FOUND);
    archive.index().pool().close().await;
    assert_eq!(
        get(&metrics, "/metrics").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[test]
fn associations_are_counted_while_they_last() {
    use std::sync::atomic::Ordering;
    let open = || pacs_core::metrics::ASSOCIATIONS_OPEN.load(Ordering::Relaxed);
    let accepted = || pacs_core::metrics::ASSOCIATIONS_ACCEPTED.load(Ordering::Relaxed);
    let (before, before_accepted) = (open(), accepted());
    let association = pacs_core::metrics::OpenAssociation::new("10.0.0.7:4100", "CT1", "PACSRS");
    assert_eq!(open(), before + 1);
    assert_eq!(accepted(), before_accepted + 1);
    association.served(3, 1, 0);
    let listed = pacs_core::metrics::active_associations();
    let mine = listed.iter().find(|a| a.peer == "10.0.0.7:4100").unwrap();
    assert_eq!(
        (mine.calling.as_str(), mine.stored, mine.queries),
        ("CT1", 3, 1)
    );
    drop(association);
    assert_eq!(open(), before);
    assert!(
        !pacs_core::metrics::active_associations()
            .iter()
            .any(|a| a.peer == "10.0.0.7:4100")
    );
}

#[test]
fn the_listener_is_off_unless_given_an_address() {
    assert!(
        Settings::try_parse_from(["pacs-server"])
            .unwrap()
            .metrics_listen
            .is_none()
    );
    let on =
        Settings::try_parse_from(["pacs-server", "--metrics-listen", "127.0.0.1:9100"]).unwrap();
    assert_eq!(on.metrics_listen.unwrap().port(), 9100);
    assert!(Settings::try_parse_from(["pacs-server", "--metrics-listen", "nowhere"]).is_err());
}
