//! Moves run in the background: `GET /jobs`, `GET /jobs/{id}` with the C-MOVE's counts as its
//! Pending responses bring them (PS3.4 C.4.2.1.6-9), and `POST /jobs/{id}/cancel`, which
//! sends C-CANCEL-MOVE-RQ.
//!
//! Jobs are kept in the index, not in memory: a restart finds the moves it was running and
//! takes them up again ([`resume`]), from the start: what had arrived stays, and comes again
//! as already stored. A move taken up three times, or whose remote AE is no longer
//! configured, or that can no longer arrive here, ends as a failure that says why. A cancel
//! goes through the index too: the move reads it within a quarter of a second, and stops
//! with the remote AE's next response.

use super::schema::Error;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_dimse::{AeTitle, Config, MoveError, MoveOutcome, Progress, StatusClass};
use pacs_index::{Index, StoredJob};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;

/// Takes up a job left running at most this many times; the next start ends it.
pub const MAX_RESTARTS: i64 = 3;
/// How often a running move writes its counts and reads whether to stop: one small write
/// to the index per running move.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn progress_json(p: &Progress) -> Value {
    json!({
        "Remaining": p.remaining,
        "Completed": p.completed,
        "Failed": p.failed,
        "Warning": p.warning,
    })
}

fn count(progress: &Value, name: &str) -> Option<u16> {
    progress
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|c| u16::try_from(c).ok())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn to_json(job: &StoredJob) -> Value {
    let p = &job.progress;
    let (remaining, completed, failed, warning) = (
        count(p, "Remaining"),
        count(p, "Completed"),
        count(p, "Failed"),
        count(p, "Warning"),
    );
    // done out of all: the counts so far plus what remains, while Remaining is known
    let done = [completed, failed, warning]
        .iter()
        .map(|c| u32::from(c.unwrap_or(0)))
        .sum::<u32>();
    let running = job.state == "Running";
    let percent = remaining
        .map(|r| done + u32::from(r))
        .filter(|total| *total > 0)
        .map(|total| (100 * done / total) as u64)
        .or((!running).then_some(100));
    let end = job.finished_ms.unwrap_or_else(now_ms);
    json!({
        "ID": job.id.to_string(),
        "Type": job.kind,
        "Remote": job.remote,
        "StudyInstanceUIDs": job.studies,
        "State": job.state,
        "Progress": percent,
        "Remaining": remaining,
        "Completed": completed,
        "Failed": failed,
        "Warning": warning,
        "FailedSOPInstanceUIDs": job.failed_instances,
        "Error": job.error,
        "ElapsedMs": u64::try_from(end - job.started_ms).unwrap_or(0),
        "Restarts": job.restarts,
    })
}

/// The end of a move, as the job keeps it.
async fn finish(index: &Index, id: i64, result: Result<MoveOutcome, MoveError>) {
    let (state, progress, failed, error) = match result {
        Ok(outcome) => {
            let state = match outcome.status.class() {
                StatusClass::Success => "Success",
                StatusClass::Cancel => "Cancelled",
                _ => "Warning",
            };
            (state, outcome.progress, outcome.failed_instances, None)
        }
        Err(e) => {
            let (progress, failed) = match &e {
                MoveError::Failed {
                    progress,
                    failed_instances,
                    ..
                } => (*progress, failed_instances.clone()),
                _ => (Progress::default(), Vec::new()),
            };
            ("Failure", progress, failed, Some(e.to_string()))
        }
    };
    if let Err(e) = index
        .finish_job(
            id,
            state,
            &progress_json(&progress),
            &failed,
            error.as_deref(),
        )
        .await
    {
        // the move is over either way; a restart would take it up again
        tracing::error!(job = id, error = %e, "the end of a job was not written to the index");
    }
}

/// Run job `id`'s move to its end. Beside it, four times a second, its counts go to the index and
/// a cancel asked there comes back: the move's own callback cannot wait on the index.
async fn run(
    index: Index,
    config: Arc<Config>,
    id: i64,
    remote_id: String,
    studies: Vec<String>,
    destination: AeTitle,
) {
    let Some(remote) = config.remote(&remote_id) else {
        let error = format!("the remote AE {remote_id} is not configured");
        if let Err(e) = index
            .finish_job(id, "Failure", &json!({}), &[], Some(&error))
            .await
        {
            tracing::error!(job = id, error = %e, "the end of a job was not written to the index");
        }
        return;
    };
    let latest = Arc::new(Mutex::new(None::<Progress>));
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (index, latest, cancel, done) =
            (index.clone(), latest.clone(), cancel.clone(), done.clone());
        tokio::spawn(async move {
            while !done.load(Ordering::Relaxed) {
                tokio::time::sleep(PROGRESS_EVERY).await;
                let progress = latest.lock().map(|mut p| p.take()).unwrap_or(None);
                let written = match progress {
                    Some(p) => index.job_progress(id, &progress_json(&p)).await,
                    None => index
                        .job(id)
                        .await
                        .map(|j| j.is_some_and(|j| j.cancel_asked)),
                };
                match written {
                    Ok(asked) if asked => cancel.store(true, Ordering::Relaxed),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!(job = id, error = %e, "a job's counts were not written")
                    }
                }
            }
        })
    };
    let result = pacs_dimse::move_studies(
        remote,
        &config.ae_title,
        &studies,
        &destination,
        config.timeout,
        config.move_timeout,
        |progress| {
            if let Ok(mut latest) = latest.lock() {
                *latest = Some(progress);
            }
            if cancel.load(Ordering::Relaxed) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
    )
    .await;
    done.store(true, Ordering::Relaxed);
    ticker.abort();
    if let Err(e) = &result {
        tracing::info!(remote = %remote_id, job = id, error = %e, "C-MOVE failed");
    }
    finish(&index, id, result).await;
}

/// Run a C-MOVE as job; answers at once with its id and path.
pub(super) async fn spawn_move(
    remotes: Arc<Remotes>,
    remote_id: String,
    studies: Vec<String>,
    destination: AeTitle,
) -> Response {
    let id = match remotes
        .index
        .create_job("C-MOVE", &remote_id, destination.as_str(), &studies)
        .await
    {
        Ok(id) => id,
        Err(e) => {
            tracing::error!(error = %e, "a job could not be written to the index");
            return RestError::Status {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "the job could not be kept".into(),
                details: None,
            }
            .into_response();
        }
    };
    tokio::spawn(run(
        remotes.index.clone(),
        Arc::new(remotes.config.clone()),
        id,
        remote_id,
        studies,
        destination,
    ));
    (
        StatusCode::ACCEPTED,
        Json(json!({"ID": id.to_string(), "Path": format!("/jobs/{id}")})),
    )
        .into_response()
}

/// Take up the jobs a stopped server left running: each starts again, or ends as a failure
/// that says why it cannot. `receiving`: whether this server listens for DIMSE, without
/// which nothing moved here can arrive. Call once, at the start.
pub async fn resume(
    index: &Index,
    config: Arc<Config>,
    receiving: bool,
) -> Result<usize, pacs_index::Error> {
    let jobs = index.running_jobs().await?;
    let mut resumed = 0;
    for job in jobs {
        let refuse = if job.restarts >= MAX_RESTARTS {
            Some(format!(
                "stopped by a restart of the server {} times; not taken up again",
                job.restarts + 1
            ))
        } else if config.remote(&job.remote).is_none() {
            Some(format!(
                "the server restarted, and the remote AE {} is no longer configured",
                job.remote
            ))
        } else if !receiving {
            Some(
                "the server restarted without DIMSE (PACS_DIMSE_LISTEN): nothing moved can arrive"
                    .into(),
            )
        } else {
            None
        };
        let destination = AeTitle::new(&job.destination);
        match (refuse, destination) {
            (Some(error), _) => {
                tracing::warn!(job = job.id, %error, "a job left running ends");
                index
                    .finish_job(
                        job.id,
                        "Failure",
                        &job.progress,
                        &job.failed_instances,
                        Some(&error),
                    )
                    .await?;
            }
            (None, Err(e)) => {
                let error = format!(
                    "its destination {:?} is not an AE title: {e}",
                    job.destination
                );
                index
                    .finish_job(
                        job.id,
                        "Failure",
                        &job.progress,
                        &job.failed_instances,
                        Some(&error),
                    )
                    .await?;
            }
            (None, Ok(destination)) => {
                index.restart_job(job.id).await?;
                tracing::info!(job = job.id, remote = %job.remote, restarts = job.restarts + 1, "a job left running is taken up again");
                tokio::spawn(run(
                    index.clone(),
                    config.clone(),
                    job.id,
                    job.remote.clone(),
                    job.studies.clone(),
                    destination,
                ));
                resumed += 1;
            }
        }
    }
    Ok(resumed)
}

fn index_failed(e: pacs_index::Error) -> Response {
    tracing::error!(error = %e, "reading the jobs");
    RestError::Status {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: "the jobs could not be read".into(),
        details: None,
    }
    .into_response()
}

/// Lists the jobs.
///
/// The moves running in the background and the latest 100 that are over, oldest first. They are kept in the index: a restart takes up the ones it was running.
#[utoipa::path(
    get,
    path = "/jobs",
    tag = "Jobs",
    responses((status = 200, description = "The jobs' ids.", body = Vec<String>))
)]
pub(super) async fn list(State(remotes): State<Arc<Remotes>>) -> Response {
    match remotes.index.jobs().await {
        Ok(jobs) => {
            let ids: Vec<String> = jobs.iter().map(|j| j.id.to_string()).collect();
            Json(json!(ids)).into_response()
        }
        Err(e) => index_failed(e),
    }
}

/// A job: its state, progress and counts.
#[utoipa::path(
    get,
    path = "/jobs/{id}",
    tag = "Jobs",
    params(("id" = String, Path, description = "The job's id.")),
    responses(
        (status = 200, description = "The job.", body = super::schema::Job),
        (status = 404, description = "No job has this id; the body is empty.")
    )
)]
pub(super) async fn get(State(remotes): State<Arc<Remotes>>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return RestError::NotFound.into_response();
    };
    match remotes.index.job(id).await {
        Ok(Some(job)) => Json(to_json(&job)).into_response(),
        Ok(None) => RestError::NotFound.into_response(),
        Err(e) => index_failed(e),
    }
}

/// Asks a running move to stop.
///
/// A C-CANCEL goes out with the remote AE's next response, and the job ends `Cancelled`
/// once that AE answers so (PS3.4 C.4.2.2.1): what it had sent by then stays.
#[utoipa::path(
    post,
    path = "/jobs/{id}/cancel",
    tag = "Jobs",
    params(("id" = String, Path, description = "The job's id.")),
    responses(
        (status = 202, description = "Asked to stop; the job as it is now.", body = super::schema::Job),
        (status = 404, description = "No job has this id; the body is empty."),
        (status = 409, description = "The job is over already.", body = Error)
    )
)]
pub(super) async fn cancel(
    State(remotes): State<Arc<Remotes>>,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<i64>() else {
        return RestError::NotFound.into_response();
    };
    match remotes.index.ask_cancel(id).await {
        Ok(None) => RestError::NotFound.into_response(),
        Ok(Some(job)) if job.state != "Running" => RestError::Status {
            status: StatusCode::CONFLICT,
            message: format!("job {id} is over ({})", job.state),
            details: None,
        }
        .into_response(),
        Ok(Some(job)) => (StatusCode::ACCEPTED, Json(to_json(&job))).into_response(),
        Err(e) => index_failed(e),
    }
}
