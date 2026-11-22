//! Retrievals run in the background: `GET /jobs`, `GET /jobs/{id}` with the counts of the
//! C-MOVE or C-GET as its Pending responses bring them (PS3.4 C.4.2.1.6-9, C.4.3.1.3), and
//! `POST /jobs/{id}/cancel`, which sends C-CANCEL-MOVE-RQ or C-CANCEL-GET-RQ. A job starts as
//! a C-MOVE and its `Type` becomes `C-GET` when it gives way to one ([`super::retriever`]).
//!
//! Jobs are kept in the index, not in memory. A running move shows it is alive with a
//! heartbeat; one silent for [`STALE_AFTER`] is a stopped server's, and any server on the
//! index takes it up again ([`take_up_stale`]), from the start: what had arrived stays, and
//! comes again as already stored. A move taken up three times, or whose remote AE is no longer
//! configured, ends as a failure that says why. A cancel
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
use pacs_dimse::{AeTitle, MoveError, MoveOutcome, Progress, StatusClass};
use pacs_index::{Index, StoredJob};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;
use super::retriever::{Method, Retriever};

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

pub(crate) fn now_ms() -> i64 {
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
        "PatientID": job.patient_id,
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

/// What a job does with its studies.
enum Work {
    /// Bring them in for `destination`, pacs-rs's AE title or a project's: a C-MOVE or a
    /// C-GET.
    Retrieve {
        patient_id: Option<String>,
        destination: AeTitle,
    },
    /// Send them to the remote AE: C-STOREs.
    Send,
    /// Send their results, re-identified, to the remote AE: C-STOREs of write-back.
    WriteBack,
}

/// The kind a job of sending is kept under.
const SEND: &str = "C-STORE";
/// The kind a job of write-back is kept under.
pub(crate) const WRITE_BACK: &str = "WRITE-BACK";

/// Run job `id` to its end. Beside it, four times a second, its counts go to the index and
/// a cancel asked there comes back: the operation's own callback cannot wait on the index.
async fn run(
    index: Index,
    retriever: Arc<Retriever>,
    id: i64,
    remote_id: String,
    studies: Vec<String>,
    work: Work,
) {
    let Some(remote) = retriever.config.remote(&remote_id) else {
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
                let progress = latest
                    .lock()
                    .map(|mut p| p.take())
                    .unwrap_or(None)
                    .map(|p| progress_json(&p));
                // the heartbeat says this server still runs it
                let written = index.job_heartbeat(id, progress.as_ref()).await;
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
    let on_progress = |progress| {
        if let Ok(mut latest) = latest.lock() {
            *latest = Some(progress);
        }
        if cancel.load(Ordering::Relaxed) {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let (result, method) = match &work {
        Work::Retrieve {
            patient_id,
            destination,
        } => {
            let (result, method) = retriever
                .retrieve(
                    remote,
                    patient_id.as_deref(),
                    &studies,
                    destination,
                    on_progress,
                )
                .await;
            (result, method.name())
        }
        Work::Send => (
            retriever.send(remote, &index, &studies, on_progress).await,
            SEND,
        ),
        Work::WriteBack => (
            retriever
                .write_back(remote, &index, &studies, on_progress)
                .await,
            WRITE_BACK,
        ),
    };
    done.store(true, Ordering::Relaxed);
    ticker.abort();
    if method == Method::Get.name()
        && let Err(e) = index.job_kind(id, method).await
    {
        tracing::error!(job = id, error = %e, "a job's operation was not written to the index");
    }
    if let Err(e) = &result {
        tracing::info!(remote = %remote_id, job = id, error = %e, operation = method, "a job failed");
    }
    finish(&index, id, result).await;
}

/// Run a C-MOVE as job; answers at once with its id and path.
pub(super) async fn spawn_move(
    remotes: Arc<Remotes>,
    remote_id: String,
    patient_id: Option<String>,
    studies: Vec<String>,
    destination: AeTitle,
) -> Response {
    let id = match remotes
        .index
        .create_job(
            "C-MOVE",
            &remote_id,
            destination.as_str(),
            patient_id.as_deref(),
            &studies,
        )
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
        remotes.retriever.clone(),
        id,
        remote_id,
        studies,
        Work::Retrieve {
            patient_id,
            destination,
        },
    ));
    (
        StatusCode::ACCEPTED,
        Json(json!({"ID": id.to_string(), "Path": format!("/jobs/{id}")})),
    )
        .into_response()
}

/// Start a job of sending `studies` to `remote_id`: kept in the index, then run. Its id.
pub(crate) async fn start_send(
    index: Index,
    retriever: Arc<Retriever>,
    remote_id: String,
    studies: Vec<String>,
) -> Result<i64, pacs_index::Error> {
    let destination = retriever
        .config
        .remote(&remote_id)
        .map(|remote| remote.ae_title.to_string())
        .unwrap_or_default();
    let id = index
        .create_job(SEND, &remote_id, &destination, None, &studies)
        .await?;
    tokio::spawn(run(index, retriever, id, remote_id, studies, Work::Send));
    Ok(id)
}

/// Start a job that writes back the results of `studies`, projects' de-identified studies,
/// to `remote_id`; its id.
pub(crate) async fn start_write_back(
    index: Index,
    retriever: Arc<Retriever>,
    remote_id: String,
    studies: Vec<String>,
) -> Result<i64, pacs_index::Error> {
    let destination = retriever
        .config
        .remote(&remote_id)
        .map(|remote| remote.ae_title.to_string())
        .unwrap_or_default();
    let id = index
        .create_job(WRITE_BACK, &remote_id, &destination, None, &studies)
        .await?;
    tokio::spawn(run(
        index,
        retriever,
        id,
        remote_id,
        studies,
        Work::WriteBack,
    ));
    Ok(id)
}

/// Send studies as a job; answers at once with its id and path.
pub(super) async fn spawn_send(
    remotes: Arc<Remotes>,
    remote_id: String,
    studies: Vec<String>,
) -> Response {
    let started = start_send(
        remotes.index.clone(),
        remotes.retriever.clone(),
        remote_id,
        studies,
    )
    .await;
    match started {
        Ok(id) => (
            StatusCode::ACCEPTED,
            Json(json!({"ID": id.to_string(), "Path": format!("/jobs/{id}")})),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "a job could not be written to the index");
            RestError::Status {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "the job could not be kept".into(),
                details: None,
            }
            .into_response()
        }
    }
}

/// A running job whose heartbeat stopped this long ago is a stopped server's.
pub const STALE_AFTER: Duration = Duration::from_secs(30);
/// How often a server looks for them.
const SWEEP_EVERY: Duration = Duration::from_secs(10);

/// Take up the jobs a stopped server left running, its heartbeat silent for `stale_after`:
/// each is claimed first, so that of several servers on one index only one takes it, then
/// starts again or ends as a failure that says why it cannot. How many start again.
pub async fn take_up_stale(
    index: &Index,
    retriever: Arc<Retriever>,
    stale_after: Duration,
) -> Result<usize, pacs_index::Error> {
    let config = &retriever.config;
    let before = now_ms() - i64::try_from(stale_after.as_millis()).unwrap_or(i64::MAX);
    let mut resumed = 0;
    for job in index.stale_jobs(before).await? {
        if !index.claim_job(job.id, before).await? {
            continue;
        }
        // the claim counted this start among the restarts
        let restarts = job.restarts + 1;
        let refuse = if restarts > MAX_RESTARTS {
            Some(format!(
                "stopped by a restart of the server {restarts} times; not taken up again"
            ))
        } else if config.remote(&job.remote).is_none() {
            Some(format!(
                "the server restarted, and the remote AE {} is no longer configured",
                job.remote
            ))
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
                tracing::info!(job = job.id, remote = %job.remote, restarts, "a job left running is taken up again");
                let work = if job.kind == SEND {
                    Work::Send
                } else if job.kind == WRITE_BACK {
                    Work::WriteBack
                } else {
                    Work::Retrieve {
                        patient_id: job.patient_id.clone(),
                        destination,
                    }
                };
                tokio::spawn(run(
                    index.clone(),
                    retriever.clone(),
                    job.id,
                    job.remote.clone(),
                    job.studies.clone(),
                    work,
                ));
                resumed += 1;
            }
        }
    }
    Ok(resumed)
}

/// Look for jobs left running every ten seconds, for the life of the server: one stopped by
/// this server's own restart is taken up once its heartbeat is stale, as is one of another
/// server on the same index.
pub fn spawn_sweeps(index: Index, retriever: Arc<Retriever>) {
    tokio::spawn(async move {
        loop {
            match take_up_stale(&index, retriever.clone(), STALE_AFTER).await {
                Ok(0) => {}
                Ok(resumed) => tracing::info!(resumed, "moves left running taken up again"),
                Err(error) => {
                    tracing::error!(%error, "looking for jobs left running failed; trying again")
                }
            }
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
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
