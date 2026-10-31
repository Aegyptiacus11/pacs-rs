//! The remote AEs pacs-rs can call: `GET /modalities[?expand]`, a C-ECHO to test the link
//! to one (`POST /modalities/{id}/echo`), and the operations on its studies, one module
//! each: [`studies`] (C-FIND), [`retrieve`] (C-MOVE), with moves in the background as
//! [`jobs`].

mod jobs;
mod retrieve;
mod studies;

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pacs_dimse::{AeTitle, Config, RemoteAe};
use pacs_rest::RestError;
use serde_json::{Value, json};

/// What the routes share: the DIMSE configuration, whether pacs-rs listens for DIMSE
/// (without which nothing moved to it can arrive), and the moves running in the background.
pub(crate) struct Remotes {
    pub config: Config,
    pub receiving: bool,
    /// Project names and their AE titles, the destinations a move may name.
    pub projects: Vec<(String, AeTitle)>,
    jobs: jobs::Jobs,
}

pub(crate) fn router(config: Config, receiving: bool, projects: Vec<(String, AeTitle)>) -> Router {
    Router::new()
        .route("/modalities", get(list))
        .route("/modalities/{id}/echo", post(echo))
        .route("/modalities/{id}/studies", get(studies::studies))
        .route("/modalities/{id}/move", post(retrieve::move_studies))
        .route("/jobs", get(jobs::list))
        .route("/jobs/{id}", get(jobs::get))
        .route("/jobs/{id}/cancel", post(jobs::cancel))
        .with_state(Arc::new(Remotes {
            config,
            receiving,
            projects,
            jobs: jobs::Jobs::default(),
        }))
}

fn describe(remote: &RemoteAe) -> Value {
    json!({
        "AET": remote.ae_title.as_str(),
        "Host": remote.host,
        "Port": remote.port,
    })
}

/// The ids, or with `expand` each id's AE title, host and port.
async fn list(
    State(remotes): State<Arc<Remotes>>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    if query.contains_key("expand") {
        let all: serde_json::Map<String, Value> = remotes
            .config
            .remotes
            .iter()
            .map(|r| (r.id.clone(), describe(r)))
            .collect();
        Json(Value::Object(all))
    } else {
        Json(json!(
            remotes
                .config
                .remotes
                .iter()
                .map(|r| &r.id)
                .collect::<Vec<_>>()
        ))
    }
}

/// One C-ECHO from pacs-rs's AE title: 200 with the round trip when the remote AE answers
/// Success, 502 with the reason when it does not, 404 for an unknown id.
async fn echo(State(remotes): State<Arc<Remotes>>, Path(id): Path<String>) -> Response {
    let Some(remote) = remotes.config.remote(&id) else {
        return RestError::NotFound.into_response();
    };
    match pacs_dimse::echo(remote, &remotes.config.ae_title, remotes.config.timeout).await {
        Ok(echo) => Json(json!({
            "Status": "Success",
            "AET": remote.ae_title.as_str(),
            "ElapsedMs": echo.elapsed.as_millis() as u64,
        }))
        .into_response(),
        Err(e) => {
            tracing::info!(remote = %id, error = %e, "C-ECHO failed");
            RestError::Status {
                status: StatusCode::BAD_GATEWAY,
                message: format!("C-ECHO to {id} failed"),
                details: Some(e.to_string()),
            }
            .into_response()
        }
    }
}
