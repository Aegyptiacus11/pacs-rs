//! The remote AEs pacs-rs can call: `GET /modalities[?expand]`, a C-ECHO to test the link
//! to one (`POST /modalities/{id}/echo`), and the operations on its studies, one module
//! each: [`studies`] (C-FIND), [`retrieve`] (C-MOVE), with moves in the background as
//! [`jobs`].

mod jobs;
mod retrieve;
pub mod schema;
mod studies;

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use pacs_dimse::{AeTitle, Config, RemoteAe};
use pacs_rest::RestError;
use serde_json::{Value, json};
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use schema::{Echo, Error, RemoteAes};

/// What the routes share: the DIMSE configuration, whether pacs-rs listens for DIMSE
/// (without which nothing moved to it can arrive), and the moves running in the background.
pub(crate) struct Remotes {
    pub config: Config,
    pub receiving: bool,
    /// Project names and their AE titles, the destinations a move may name.
    pub projects: Vec<(String, AeTitle)>,
    jobs: jobs::Jobs,
}

/// The routes, each one by its `#[utoipa::path]`, so that none is served that the API's
/// description does not have.
fn described() -> OpenApiRouter<Arc<Remotes>> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(echo))
        .routes(routes!(studies::studies))
        .routes(routes!(retrieve::move_studies))
        .routes(routes!(jobs::list))
        .routes(routes!(jobs::get))
        .routes(routes!(jobs::cancel))
}

/// The description alone, for writing the reference.
pub(crate) fn api() -> OpenApi {
    described().into_openapi()
}

pub(crate) fn router(config: Config, receiving: bool, projects: Vec<(String, AeTitle)>) -> Router {
    described()
        .with_state(Arc::new(Remotes {
            config,
            receiving,
            projects,
            jobs: jobs::Jobs::default(),
        }))
        .split_for_parts()
        .0
}

fn describe(remote: &RemoteAe) -> Value {
    json!({
        "AET": remote.ae_title.as_str(),
        "Host": remote.host,
        "Port": remote.port,
    })
}

/// Lists the remote AEs.
///
/// The AEs of `PACS_DIMSE_REMOTES`: those pacs-rs can test, search and pull studies from.
#[utoipa::path(
    get,
    path = "/modalities",
    tag = "Remote archives",
    params(("expand" = Option<bool>, Query, description = "Answer with each AE's title, host and port, by id, instead of the ids.")),
    responses((status = 200, description = "The ids, or the AEs by id.", body = RemoteAes))
)]
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

/// Tests the link to a remote AE.
///
/// One C-ECHO from pacs-rs's AE title.
#[utoipa::path(
    post,
    path = "/modalities/{id}/echo",
    tag = "Remote archives",
    params(("id" = String, Path, description = "The remote AE's id, as `PACS_DIMSE_REMOTES` names it.")),
    responses(
        (status = 200, description = "The remote AE answered Success.", body = Echo),
        (status = 404, description = "No remote AE has this id; the body is empty."),
        (status = 502, description = "It could not be reached, refused the association, or answered otherwise; `Details` says which.", body = Error)
    )
)]
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
