//! `GET /ui/api/server`: what the web interface's Server page shows, for a role with `users`.
//! The archive's totals, the DIMSE associations being served, the jobs, the quarantine, and
//! the warnings and errors logged lately; the page asks again every two seconds.

use std::time::UNIX_EPOCH;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_archive::Archive;
use pacs_store::Store;
use serde_json::{Value, json};

#[derive(Clone)]
struct Page {
    archive: Archive,
    /// `PACS_QUARANTINE`, when set.
    quarantine: Option<Store>,
}

fn ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Finished jobs shown beside the running ones.
const RECENT_JOBS: usize = 5;

async fn server(State(page): State<Page>) -> Response {
    let index = page.archive.index();
    let read = async {
        let totals = index.totals().await?;
        let checks = index.check_counts().await?;
        let trash = index.trash().await?.len();
        let jobs = index.jobs().await?;
        Ok::<_, pacs_index::Error>((totals, checks, trash, jobs))
    };
    let ((studies, instances, bytes), checks, trash, jobs) = match read.await {
        Ok(read) => read,
        Err(error) => {
            tracing::error!(%error, "the server page: the index did not answer");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({"detail": "The index did not answer"})),
            )
                .into_response();
        }
    };
    let (running, finished): (Vec<_>, Vec<_>) =
        jobs.into_iter().partition(|j| j.state == "Running");
    let job = |j: &pacs_index::StoredJob| {
        json!({
            "id": j.id.to_string(),
            "remote": j.remote,
            "studies": j.studies.len(),
            "state": j.state,
            "progress": j.progress,
            "error": j.error,
            "started_ms": j.started_ms,
            "finished_ms": j.finished_ms,
            "restarts": j.restarts,
        })
    };
    let quarantine = match &page.quarantine {
        None => Value::Null,
        Some(store) => match store.list().await {
            Ok(objects) => json!({
                "files": objects.len(),
                "bytes": objects.iter().map(|o| o.size).sum::<u64>(),
                "latest_ms": objects.iter().map(|o| ms(o.modified)).max(),
            }),
            Err(error) => json!({ "error": error.to_string() }),
        },
    };
    let associations: Vec<Value> = pacs_core::metrics::active_associations()
        .into_iter()
        .map(|a| {
            json!({
                "peer": a.peer,
                "calling": a.calling,
                "called": a.called,
                "since_ms": ms(a.since),
                "stored": a.stored,
                "queries": a.queries,
                "retrievals": a.retrievals,
            })
        })
        .collect();
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "archive": {
            "studies": studies,
            "instances": instances,
            "bytes": bytes,
            "files_failed": checks.failed,
            "files_never_checked": checks.never_checked,
            "trash": trash,
        },
        "associations": associations,
        "jobs": running.iter().map(job).chain(finished.iter().rev().take(RECENT_JOBS).map(job)).collect::<Vec<_>>(),
        "quarantine": quarantine,
        "log": crate::metrics::logged(),
    }))
    .into_response()
}

/// `/ui/api/server`.
pub fn router(archive: Archive, quarantine: Option<Store>) -> axum::Router {
    axum::Router::new()
        .route("/ui/api/server", axum::routing::get(server))
        .with_state(Page {
            archive,
            quarantine,
        })
}
