//! The remote AEs pacs-rs can call: `GET /modalities[?expand]`, a C-ECHO to test the link
//! to one (`POST /modalities/{id}/echo`), and the operations on its studies, one module
//! each: [`studies`] (C-FIND).

mod studies;

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use pacs_dimse::{Config, RemoteAe};
use pacs_rest::RestError;
use serde_json::{Value, json};

pub(crate) fn router(config: Config) -> Router {
    Router::new()
        .route("/modalities", get(list))
        .route("/modalities/{id}/echo", post(echo))
        .route("/modalities/{id}/studies", get(studies::studies))
        .with_state(Arc::new(config))
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
    State(config): State<Arc<Config>>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    if query.contains_key("expand") {
        let all: serde_json::Map<String, Value> = config
            .remotes
            .iter()
            .map(|r| (r.id.clone(), describe(r)))
            .collect();
        Json(Value::Object(all))
    } else {
        Json(json!(
            config.remotes.iter().map(|r| &r.id).collect::<Vec<_>>()
        ))
    }
}

/// One C-ECHO from pacs-rs's AE title: 200 with the round trip when the remote AE answers
/// Success, 502 with the reason when it does not, 404 for an unknown id.
async fn echo(State(config): State<Arc<Config>>, Path(id): Path<String>) -> Response {
    let Some(remote) = config.remote(&id) else {
        return RestError::NotFound.into_response();
    };
    match pacs_dimse::echo(remote, &config.ae_title, config.timeout).await {
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
