//! Moves run in the background: `GET /jobs`, `GET /jobs/{id}` with the C-MOVE's counts as its
//! Pending responses bring them (PS3.4 C.4.2.1.6-9), and `POST /jobs/{id}/cancel`, which
//! sends C-CANCEL-MOVE-RQ. Jobs live in memory: a restart forgets them, and only the latest
//! finished ones are kept.

use std::collections::BTreeMap;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_dimse::{MoveError, MoveOutcome, Progress, StatusClass};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;

/// Finished jobs kept for `GET /jobs/{id}`; older ones are forgotten.
const KEPT_FINISHED: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobState {
    Running,
    Success,
    /// Some sub-operations failed or warned.
    Warning,
    Cancelled,
    Failure,
}

impl JobState {
    fn name(self) -> &'static str {
        match self {
            JobState::Running => "Running",
            JobState::Success => "Success",
            JobState::Warning => "Warning",
            JobState::Cancelled => "Cancelled",
            JobState::Failure => "Failure",
        }
    }
}

struct Job {
    remote: String,
    studies: Vec<String>,
    state: JobState,
    progress: Progress,
    failed_instances: Vec<String>,
    error: Option<String>,
    started: Instant,
    elapsed: Option<Duration>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn to_json(&self, id: u64) -> Value {
        let p = self.progress;
        // done out of all: the counts so far plus what remains, while Remaining is known
        let done = [p.completed, p.failed, p.warning]
            .iter()
            .map(|c| u32::from(c.unwrap_or(0)))
            .sum::<u32>();
        let percent = p
            .remaining
            .map(|r| done + u32::from(r))
            .filter(|total| *total > 0)
            .map(|total| (100 * done / total) as u64)
            .or((self.state != JobState::Running).then_some(100));
        json!({
            "ID": id.to_string(),
            "Type": "C-MOVE",
            "Remote": self.remote,
            "StudyInstanceUIDs": self.studies,
            "State": self.state.name(),
            "Progress": percent,
            "Remaining": p.remaining,
            "Completed": p.completed,
            "Failed": p.failed,
            "Warning": p.warning,
            "FailedSOPInstanceUIDs": self.failed_instances,
            "Error": self.error,
            "ElapsedMs": self.elapsed.unwrap_or_else(|| self.started.elapsed()).as_millis() as u64,
        })
    }
}

/// The jobs of this process, by id.
#[derive(Default)]
pub(crate) struct Jobs {
    table: Mutex<(u64, BTreeMap<u64, Job>)>,
}

impl Jobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, (u64, BTreeMap<u64, Job>)> {
        self.table.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn start(&self, remote: &str, studies: &[String]) -> (u64, Arc<AtomicBool>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut table = self.lock();
        table.0 += 1;
        let id = table.0;
        table.1.insert(
            id,
            Job {
                remote: remote.to_owned(),
                studies: studies.to_vec(),
                state: JobState::Running,
                progress: Progress::default(),
                failed_instances: Vec::new(),
                error: None,
                started: Instant::now(),
                elapsed: None,
                cancel: cancel.clone(),
            },
        );
        (id, cancel)
    }

    fn update(&self, id: u64, f: impl FnOnce(&mut Job)) {
        if let Some(job) = self.lock().1.get_mut(&id) {
            f(job);
        }
    }

    fn finish(&self, id: u64, result: Result<MoveOutcome, MoveError>) {
        let mut table = self.lock();
        if let Some(job) = table.1.get_mut(&id) {
            job.elapsed = Some(job.started.elapsed());
            match result {
                Ok(outcome) => {
                    job.progress = outcome.progress;
                    job.failed_instances = outcome.failed_instances;
                    job.state = match outcome.status.class() {
                        StatusClass::Success => JobState::Success,
                        StatusClass::Cancel => JobState::Cancelled,
                        _ => JobState::Warning,
                    };
                }
                Err(e) => {
                    if let MoveError::Failed {
                        progress,
                        failed_instances,
                        ..
                    } = &e
                    {
                        job.progress = *progress;
                        job.failed_instances = failed_instances.clone();
                    }
                    job.error = Some(e.to_string());
                    job.state = JobState::Failure;
                }
            }
        }
        let finished: Vec<u64> = table
            .1
            .iter()
            .filter(|(_, j)| j.state != JobState::Running)
            .map(|(id, _)| *id)
            .collect();
        for old in finished
            .iter()
            .take(finished.len().saturating_sub(KEPT_FINISHED))
        {
            table.1.remove(old);
        }
    }
}

/// Run a C-MOVE as job; answers at once with its id and path.
pub(super) fn spawn_move(
    remotes: Arc<Remotes>,
    remote_id: String,
    studies: Vec<String>,
    destination: pacs_dimse::AeTitle,
) -> Response {
    let (id, cancel) = remotes.jobs.start(&remote_id, &studies);
    tokio::spawn(async move {
        let config = &remotes.config;
        let Some(remote) = config.remote(&remote_id) else {
            return;
        };
        let result = pacs_dimse::move_studies(
            remote,
            &config.ae_title,
            &studies,
            &destination,
            config.timeout,
            config.move_timeout,
            |progress| {
                remotes.jobs.update(id, |job| job.progress = progress);
                if cancel.load(Ordering::Relaxed) {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .await;
        if let Err(e) = &result {
            tracing::info!(remote = %remote_id, job = id, error = %e, "C-MOVE failed");
        }
        remotes.jobs.finish(id, result);
    });
    (
        StatusCode::ACCEPTED,
        Json(json!({"ID": id.to_string(), "Path": format!("/jobs/{id}")})),
    )
        .into_response()
}

/// The ids of the jobs kept, oldest first.
pub(super) async fn list(State(remotes): State<Arc<Remotes>>) -> Json<Value> {
    let ids: Vec<String> = remotes.jobs.lock().1.keys().map(u64::to_string).collect();
    Json(json!(ids))
}

pub(super) async fn get(State(remotes): State<Arc<Remotes>>, Path(id): Path<String>) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return RestError::NotFound.into_response();
    };
    match remotes.jobs.lock().1.get(&id) {
        Some(job) => Json(job.to_json(id)).into_response(),
        None => RestError::NotFound.into_response(),
    }
}

/// Ask a running move to stop: C-CANCEL-MOVE-RQ goes out with the next Pending response, and
/// the job ends Cancelled once the remote AE answers Cancel (C.4.2.2.1). 409 for a job
/// already over.
pub(super) async fn cancel(
    State(remotes): State<Arc<Remotes>>,
    Path(id): Path<String>,
) -> Response {
    let Ok(id) = id.parse::<u64>() else {
        return RestError::NotFound.into_response();
    };
    match remotes.jobs.lock().1.get(&id) {
        None => RestError::NotFound.into_response(),
        Some(job) if job.state != JobState::Running => RestError::Status {
            status: StatusCode::CONFLICT,
            message: format!("job {id} is over ({})", job.state.name()),
            details: None,
        }
        .into_response(),
        Some(job) => {
            job.cancel.store(true, Ordering::Relaxed);
            (StatusCode::ACCEPTED, Json(job.to_json(id))).into_response()
        }
    }
}
