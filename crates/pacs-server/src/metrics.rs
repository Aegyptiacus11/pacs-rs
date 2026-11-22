//! `GET /metrics` in the Prometheus text format (0.0.4), on a listener of its own
//! (`PACS_METRICS_LISTEN`, off by default): a scraper reaches it without the login, and it is
//! never on the address users reach by chance.
//!
//! What the server counts as it goes (HTTP requests, DIMSE associations, instances stored,
//! errors logged) is kept in memory since the start; what the archive holds (studies,
//! instances, bytes, files whose check failed, deletions in the trash) is read from the index
//! at each scrape.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header::CONTENT_TYPE};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pacs_archive::Archive;
use pacs_core::metrics as core;

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "DELETE", "other"];
const CLASSES: [&str; 5] = ["1xx", "2xx", "3xx", "4xx", "5xx"];

/// Requests by method and status class, and their time in microseconds.
static REQUESTS: [[AtomicU64; 5]; 5] = [const { [const { AtomicU64::new(0) }; 5] }; 5];
static REQUEST_MICROS: [[AtomicU64; 5]; 5] = [const { [const { AtomicU64::new(0) }; 5] }; 5];
/// ERROR lines logged, by the crate that logged them.
static ERRORS: LazyLock<Mutex<BTreeMap<String, u64>>> = LazyLock::new(Mutex::default);
static STARTED: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Around every request: counted once answered.
pub async fn count_requests(request: Request, next: Next) -> Response {
    LazyLock::force(&STARTED);
    let method = METHODS
        .iter()
        .position(|m| *m == request.method().as_str())
        .unwrap_or(METHODS.len() - 1);
    let started = Instant::now();
    let response = next.run(request).await;
    let class = usize::from(response.status().as_u16() / 100)
        .saturating_sub(1)
        .min(CLASSES.len() - 1);
    REQUESTS[method][class].fetch_add(1, Ordering::Relaxed);
    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    REQUEST_MICROS[method][class].fetch_add(micros, Ordering::Relaxed);
    response
}

/// A logging layer that counts what is logged at ERROR, by the crate it came from.
pub struct CountErrors;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CountErrors {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() == tracing::Level::ERROR {
            let target = event.metadata().target();
            let krate = target.split("::").next().unwrap_or(target).to_owned();
            if let Ok(mut errors) = ERRORS.lock() {
                *errors.entry(krate).or_default() += 1;
            }
        }
    }
}

fn family(out: &mut String, name: &str, kind: &str, help: &str) {
    let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} {kind}");
}

fn escape(label: &str) -> String {
    label
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

/// The page, as a scrape reads it. `None` when the index does not answer.
pub async fn render(archive: &Archive) -> Result<String, pacs_index::Error> {
    let index = archive.index();
    let (studies, instances, bytes) = index.totals().await?;
    let checks = index.check_counts().await?;
    let trash = index.trash().await?.len();
    let mut out = String::new();

    family(
        &mut out,
        "pacs_build_info",
        "gauge",
        "The version of pacs-rs running.",
    );
    let _ = writeln!(
        out,
        "pacs_build_info{{version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    );
    family(
        &mut out,
        "pacs_uptime_seconds",
        "gauge",
        "Seconds since the server started.",
    );
    let _ = writeln!(out, "pacs_uptime_seconds {}", STARTED.elapsed().as_secs());

    family(
        &mut out,
        "pacs_http_requests_total",
        "counter",
        "HTTP requests answered, by method and status class.",
    );
    family(
        &mut out,
        "pacs_http_request_seconds_total",
        "counter",
        "Time spent answering them, in seconds.",
    );
    for (m, method) in METHODS.iter().enumerate() {
        for (c, class) in CLASSES.iter().enumerate() {
            let count = REQUESTS[m][c].load(Ordering::Relaxed);
            if count == 0 {
                continue;
            }
            let seconds = REQUEST_MICROS[m][c].load(Ordering::Relaxed) as f64 / 1e6;
            let _ = writeln!(
                out,
                "pacs_http_requests_total{{method=\"{method}\",status=\"{class}\"}} {count}"
            );
            let _ = writeln!(
                out,
                "pacs_http_request_seconds_total{{method=\"{method}\",status=\"{class}\"}} {seconds}"
            );
        }
    }

    family(
        &mut out,
        "pacs_dimse_associations_open",
        "gauge",
        "DIMSE associations being served.",
    );
    let _ = writeln!(
        out,
        "pacs_dimse_associations_open {}",
        core::ASSOCIATIONS_OPEN.load(Ordering::Relaxed)
    );
    family(
        &mut out,
        "pacs_dimse_associations_total",
        "counter",
        "DIMSE connections, by whether they became an association.",
    );
    let _ = writeln!(
        out,
        "pacs_dimse_associations_total{{outcome=\"accepted\"}} {}",
        core::ASSOCIATIONS_ACCEPTED.load(Ordering::Relaxed)
    );
    let _ = writeln!(
        out,
        "pacs_dimse_associations_total{{outcome=\"refused\"}} {}",
        core::ASSOCIATIONS_REFUSED.load(Ordering::Relaxed)
    );

    family(
        &mut out,
        "pacs_instances_stored_total",
        "counter",
        "Instances received, by what became of them.",
    );
    for (outcome, counter) in [
        ("added", &core::INSTANCES_ADDED),
        ("replaced", &core::INSTANCES_REPLACED),
        ("already_stored", &core::INSTANCES_ALREADY_STORED),
    ] {
        let _ = writeln!(
            out,
            "pacs_instances_stored_total{{outcome=\"{outcome}\"}} {}",
            counter.load(Ordering::Relaxed)
        );
    }

    family(
        &mut out,
        "pacs_errors_total",
        "counter",
        "Lines logged at ERROR, by the crate that logged them.",
    );
    if let Ok(errors) = ERRORS.lock() {
        for (krate, count) in errors.iter() {
            let _ = writeln!(
                out,
                "pacs_errors_total{{crate=\"{}\"}} {count}",
                escape(krate)
            );
        }
    }

    family(&mut out, "pacs_studies", "gauge", "Studies in the archive.");
    let _ = writeln!(out, "pacs_studies {studies}");
    family(
        &mut out,
        "pacs_instances",
        "gauge",
        "Instances in the archive.",
    );
    let _ = writeln!(out, "pacs_instances {instances}");
    family(
        &mut out,
        "pacs_storage_bytes",
        "gauge",
        "Bytes of the stored files the index names (the trash aside).",
    );
    let _ = writeln!(out, "pacs_storage_bytes {bytes}");
    family(
        &mut out,
        "pacs_files_failed",
        "gauge",
        "Files whose last check against their SHA-256 failed.",
    );
    let _ = writeln!(out, "pacs_files_failed {}", checks.failed);
    family(
        &mut out,
        "pacs_files_never_checked",
        "gauge",
        "Files stored before checksums were kept, not checked yet.",
    );
    let _ = writeln!(out, "pacs_files_never_checked {}", checks.never_checked);
    family(
        &mut out,
        "pacs_trash_entries",
        "gauge",
        "Deletions waiting in the trash.",
    );
    let _ = writeln!(out, "pacs_trash_entries {trash}");
    Ok(out)
}

async fn page(State(archive): State<Archive>) -> Response {
    match render(&archive).await {
        Ok(page) => {
            let mut response = page.into_response();
            response.headers_mut().insert(
                CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            );
            response
        }
        Err(error) => {
            tracing::warn!(%error, "metrics: the index did not answer");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "the index did not answer\n",
            )
                .into_response()
        }
    }
}

/// The metrics listener's routes: `GET /metrics` alone.
pub fn router(archive: Archive) -> axum::Router {
    axum::Router::new()
        .route("/metrics", axum::routing::get(page))
        .with_state(archive)
}
