//! Checking the stored files against the index: each file's SHA-256 is taken when it is
//! stored, and a check reads the file back and compares (see `pacs_archive::Archive::
//! check_file`). Three ways to run it:
//!
//! - on a schedule: `spawn_due_checks` checks again, one file at a time, every file whose
//!   last check is older than `PACS_CHECK_FILES_DAYS` (30; 0: never);
//! - on request: `POST /tools/check-files`, for one study at once or the whole archive in
//!   the background, and `GET /tools/check-files` for where it stands and what failed;
//! - from the command line: `pacs-server --check-files` checks everything, says what
//!   failed, and exits 1 if anything did (for cron or a script).
//!
//! A file that fails stays served as it is: nothing is deleted or hidden. The failure is in
//! the log (ERROR), in the index, and listed by `GET /tools/check-files` until a check
//! passes again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_archive::{Archive, CheckError};
use pacs_index::StoredFile;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use utoipa::ToSchema;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Files read in one go from the index.
const PAGE: i64 = 200;
/// How long the scheduled checks rest when nothing is due.
const IDLE: Duration = Duration::from_secs(600);

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// What a check of several files found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, ToSchema)]
pub struct Summary {
    /// Files checked.
    pub checked: u64,
    /// The files that failed, by SOP Instance UID, with what was wrong.
    pub failures: Vec<Failure>,
}

/// A file that failed its check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Failure {
    /// The instance whose file it is.
    #[serde(rename = "SOPInstanceUID")]
    pub sop_instance_uid: String,
    /// What was wrong: changed (another SHA-256), a size other than the index's, or missing
    /// from the store.
    pub failure: String,
}

impl Summary {
    async fn check(&mut self, archive: &Archive, file: &StoredFile) -> Result<(), CheckError> {
        self.checked += 1;
        if let Some(failure) = archive.check_file(file).await? {
            self.failures.push(Failure {
                sop_instance_uid: file.sop_instance_uid.clone(),
                failure,
            });
        }
        Ok(())
    }
}

/// Check every stored file, page after page in SOP Instance UID order, into `summary`,
/// which keeps what was checked when a check cannot be made.
pub async fn check_all(archive: &Archive, summary: &mut Summary) -> Result<(), CheckError> {
    let mut after = String::new();
    loop {
        let files = archive.index().files_after(&after, PAGE).await?;
        let Some(last) = files.last() else {
            return Ok(());
        };
        after = last.sop_instance_uid.clone();
        for file in &files {
            summary.check(archive, file).await?;
        }
    }
}

/// Check the files due: not checked for `days` days. How many were checked.
pub async fn check_due(archive: &Archive, days: u64) -> Result<Summary, CheckError> {
    let before = now_ms() - i64::try_from(days.saturating_mul(86_400_000)).unwrap_or(i64::MAX);
    let mut summary = Summary::default();
    loop {
        let files = archive.index().files_due(before, PAGE).await?;
        if files.is_empty() {
            return Ok(summary);
        }
        for file in &files {
            summary.check(archive, file).await?;
        }
    }
}

/// The scheduled checks, for the life of the server; `days` 0 starts none. A check that
/// cannot be made (the store or the index not answering) is logged and tried again later.
pub fn spawn_due_checks(archive: Archive, days: u64) {
    if days == 0 {
        tracing::info!("scheduled file checks are off (PACS_CHECK_FILES_DAYS=0)");
        return;
    }
    tokio::spawn(async move {
        loop {
            match check_due(&archive, days).await {
                Ok(summary) if summary.checked > 0 => tracing::info!(
                    checked = summary.checked,
                    failed = summary.failures.len(),
                    "scheduled file checks done"
                ),
                Ok(_) => {}
                Err(error) => {
                    tracing::error!(%error, "scheduled file checks stopped; trying again later")
                }
            }
            tokio::time::sleep(IDLE).await;
        }
    });
}

/// The last whole-archive check started on request.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Sweep {
    /// When it started, in milliseconds since the Unix epoch.
    pub started_ms: i64,
    /// When it ended; `null` while it runs.
    pub finished_ms: Option<i64>,
    /// Files checked.
    pub checked: u64,
    /// Files that failed.
    pub failed: u64,
    /// Why it stopped before the end, when it did: the store or the index not answering.
    pub error: Option<String>,
}

#[derive(Clone)]
struct Checks {
    archive: Archive,
    running: Arc<AtomicBool>,
    last: Arc<Mutex<Option<Sweep>>>,
}

/// A file whose last check failed.
#[derive(Debug, Serialize, ToSchema)]
pub struct FailedFile {
    /// Its study.
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: String,
    /// Its series.
    #[serde(rename = "SeriesInstanceUID")]
    pub series_instance_uid: String,
    /// The instance.
    #[serde(rename = "SOPInstanceUID")]
    pub sop_instance_uid: String,
    /// What was wrong.
    pub failure: String,
    /// When it was found, in milliseconds since the Unix epoch.
    pub checked_ms: i64,
}

/// Where the checks stand.
#[derive(Debug, Serialize, ToSchema)]
pub struct Status {
    /// Stored files.
    pub files: i64,
    /// Stored before hashes were kept, and not checked since.
    pub never_checked: i64,
    /// Files whose last check failed.
    pub failed: i64,
    /// Whether a whole-archive check started on request is running.
    pub running: bool,
    /// The last whole-archive check started on request, since the server started.
    pub last_sweep: Option<Sweep>,
    /// Every file whose last check failed.
    pub failures: Vec<FailedFile>,
}

/// What to check.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CheckRequest {
    /// One study, checked before the answer. Absent: the whole archive, in the background.
    #[serde(rename = "StudyInstanceUID")]
    pub study_instance_uid: Option<String>,
}

fn failed(status: StatusCode, detail: &str) -> Response {
    (status, Json(serde_json::json!({ "detail": detail }))).into_response()
}

/// Where the checks of the stored files stand, and the files that failed.
#[utoipa::path(
    get,
    path = "/tools/check-files",
    tag = "Integrity",
    responses((status = 200, description = "The counts, the last whole-archive check, and every file whose last check failed.", body = Status))
)]
async fn status(State(checks): State<Checks>) -> Response {
    let index = checks.archive.index();
    let (counts, failures) = match (index.check_counts().await, index.failed_files().await) {
        (Ok(counts), Ok(failures)) => (counts, failures),
        (Err(e), _) | (_, Err(e)) => {
            tracing::error!(error = %e, "reading the file checks");
            return failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The index could not be read",
            );
        }
    };
    Json(Status {
        files: counts.files,
        never_checked: counts.never_checked,
        failed: counts.failed,
        running: checks.running.load(Ordering::SeqCst),
        last_sweep: checks.last.lock().await.clone(),
        failures: failures
            .into_iter()
            .map(|f| FailedFile {
                study_instance_uid: f.study_instance_uid,
                series_instance_uid: f.series_instance_uid,
                sop_instance_uid: f.sop_instance_uid,
                failure: f.failure,
                checked_ms: f.checked_ms,
            })
            .collect(),
    })
    .into_response()
}

/// Checks stored files against the index now.
///
/// With a `StudyInstanceUID`, that study's files are read and compared before the answer. Without one, the whole archive is checked in the background, one check at a time; `GET /tools/check-files` says how it went.
#[utoipa::path(
    post,
    path = "/tools/check-files",
    tag = "Integrity",
    request_body(content = CheckRequest, description = "The study to check, or nothing for the whole archive."),
    responses(
        (status = 200, description = "The study was checked.", body = Summary),
        (status = 202, description = "The whole archive is being checked."),
        (status = 404, description = "No such study."),
        (status = 409, description = "A whole-archive check is running already."),
        (status = 503, description = "The store or the index did not answer: nothing was found wrong, nothing was checked to the end.")
    )
)]
async fn start(
    State(checks): State<Checks>,
    asked: Result<Json<CheckRequest>, JsonRejection>,
) -> Response {
    let asked = match asked {
        Ok(Json(asked)) => asked,
        // no body at all: the whole archive
        Err(JsonRejection::MissingJsonContentType(_)) => CheckRequest::default(),
        Err(e) => return failed(e.status(), &e.body_text()),
    };
    if let Some(study) = asked.study_instance_uid {
        let files = match checks.archive.index().files_of_study(&study).await {
            Ok(files) if files.is_empty() => {
                return failed(StatusCode::NOT_FOUND, "No such study");
            }
            Ok(files) => files,
            Err(e) => {
                tracing::error!(error = %e, "reading a study's files");
                return failed(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The index could not be read",
                );
            }
        };
        let mut summary = Summary::default();
        for file in &files {
            if let Err(e) = summary.check(&checks.archive, file).await {
                tracing::error!(error = %e, "a study's check stopped");
                return failed(StatusCode::SERVICE_UNAVAILABLE, &e.to_string());
            }
        }
        return Json(summary).into_response();
    }
    if checks.running.swap(true, Ordering::SeqCst) {
        return failed(
            StatusCode::CONFLICT,
            "A whole-archive check is running already",
        );
    }
    let started_ms = now_ms();
    *checks.last.lock().await = Some(Sweep {
        started_ms,
        finished_ms: None,
        checked: 0,
        failed: 0,
        error: None,
    });
    tokio::spawn(async move {
        let mut summary = Summary::default();
        let result = check_all(&checks.archive, &mut summary).await;
        if let Err(e) = &result {
            tracing::error!(error = %e, "the whole-archive check stopped");
        }
        let sweep = Sweep {
            started_ms,
            finished_ms: Some(now_ms()),
            checked: summary.checked,
            failed: summary.failures.len() as u64,
            error: result.err().map(|e| e.to_string()),
        };
        tracing::info!(
            checked = sweep.checked,
            failed = sweep.failed,
            "whole-archive check done"
        );
        *checks.last.lock().await = Some(sweep);
        checks.running.store(false, Ordering::SeqCst);
    });
    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({ "running": true })),
    )
        .into_response()
}

fn described() -> OpenApiRouter<Checks> {
    OpenApiRouter::new().routes(routes!(status, start))
}

/// `/tools/check-files`.
pub fn router(archive: Archive) -> axum::Router {
    let checks = Checks {
        archive,
        running: Arc::new(AtomicBool::new(false)),
        last: Arc::new(Mutex::new(None)),
    };
    let (router, _) = described().with_state(checks).split_for_parts();
    router
}

pub(crate) fn api() -> OpenApi {
    described().into_openapi()
}
