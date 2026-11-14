//! `/ui/api/protocols`: the web interface's hanging protocols, kept in the index so that every
//! browser on this server shares them. `GET` lists them (a JSON array, the most recently
//! saved first), `PUT /{id}` keeps one (a JSON object whose `id` is the one in the path),
//! `DELETE /{id}` removes one. Part of the web interface: off with it (`PACS_UI=false`).
//! Behind the login when there is one (`auth`): reading needs a reader, writing and removing
//! an editor.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pacs_index::Index;
use serde_json::{Value, json};

/// A protocol is a few hundred bytes; this leaves room and stops abuse.
const MAX_BODY: usize = 64 * 1024;
const MAX_PROTOCOLS: usize = 1000;

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "Message": message }))).into_response()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:.-".contains(&b))
}

async fn list(State(index): State<Index>) -> Response {
    match index.protocols().await {
        Ok(rows) => {
            // stored as received; anything that no longer parses is left out, not sent
            let list: Vec<Value> = rows
                .iter()
                .filter_map(|p| serde_json::from_str(&p.body).ok())
                .collect();
            (
                [(header::CACHE_CONTROL, "no-store")],
                axum::Json(Value::Array(list)),
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("listing protocols: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The protocols could not be read",
            )
        }
    }
}

async fn put(State(index): State<Index>, Path(id): Path<String>, body: Bytes) -> Response {
    if !valid_id(&id) {
        return error(
            StatusCode::BAD_REQUEST,
            "A protocol ID is 1 to 80 letters, digits or _ : . -",
        );
    }
    let Ok(value) = serde_json::from_slice::<Value>(&body) else {
        return error(StatusCode::BAD_REQUEST, "A protocol is a JSON object");
    };
    if !value.is_object() {
        return error(StatusCode::BAD_REQUEST, "A protocol is a JSON object");
    }
    if value["id"].as_str() != Some(id.as_str()) {
        return error(
            StatusCode::BAD_REQUEST,
            "The protocol's id is not the one in the path",
        );
    }
    match index.protocols().await {
        Ok(rows) if rows.len() >= MAX_PROTOCOLS && !rows.iter().any(|p| p.id == id) => {
            return error(
                StatusCode::CONFLICT,
                "This server keeps at most 1000 protocols",
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::error!("counting protocols: {e}");
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The protocol could not be saved",
            );
        }
    }
    // the canonical text, not the request's own whitespace
    match index.put_protocol(&id, &value.to_string()).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => {
            tracing::error!("saving protocol {id}: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The protocol could not be saved",
            )
        }
    }
}

async fn delete(State(index): State<Index>, Path(id): Path<String>) -> Response {
    match index.delete_protocol(&id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "No such protocol"),
        Err(e) => {
            tracing::error!("deleting protocol {id}: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The protocol could not be deleted",
            )
        }
    }
}

pub fn router(index: Index) -> Router {
    Router::new()
        .route("/ui/api/protocols", get(list))
        .route(
            "/ui/api/protocols/{id}",
            axum::routing::put(put).delete(delete),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(index)
}
