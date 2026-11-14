//! `/ui/api/annotations/{study}`: the measurements drawn in the web interface's viewer, kept in
//! the index per study (by Study Instance UID), so that they come back when the study is
//! opened again, on any browser. `GET` returns the study's list (a JSON array, empty when it
//! has none), `PUT` replaces it (an empty array removes it). Deleting the study deletes them.
//! Part of the web interface: off with it (`PACS_UI=false`). Behind the login when there is
//! one (`auth`): reading needs a reader, writing an editor.

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pacs_index::Index;
use serde_json::{Value, json};

/// A measurement is a kilobyte or two; this is room for several hundred on one study.
const MAX_BODY: usize = 1024 * 1024;

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "Message": message }))).into_response()
}

/// A DICOM UID: digits and dots, at most 64 characters.
fn valid_uid(uid: &str) -> bool {
    !uid.is_empty() && uid.len() <= 64 && uid.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

async fn get_list(State(index): State<Index>, Path(study): Path<String>) -> Response {
    if !valid_uid(&study) {
        return error(StatusCode::BAD_REQUEST, "Not a Study Instance UID");
    }
    match index.annotations(&study).await {
        Ok(body) => {
            // stored as received; a list that no longer parses is sent as none
            let list = body
                .and_then(|b| serde_json::from_str::<Value>(&b).ok())
                .filter(Value::is_array)
                .unwrap_or_else(|| Value::Array(vec![]));
            ([(header::CACHE_CONTROL, "no-store")], axum::Json(list)).into_response()
        }
        Err(e) => {
            tracing::error!("reading measurements of {study}: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The measurements could not be read",
            )
        }
    }
}

async fn put(State(index): State<Index>, Path(study): Path<String>, body: Bytes) -> Response {
    if !valid_uid(&study) {
        return error(StatusCode::BAD_REQUEST, "Not a Study Instance UID");
    }
    let Ok(Value::Array(list)) = serde_json::from_slice::<Value>(&body) else {
        return error(StatusCode::BAD_REQUEST, "The measurements are a JSON array");
    };
    if !list.iter().all(Value::is_object) {
        return error(StatusCode::BAD_REQUEST, "Each measurement is a JSON object");
    }
    // the canonical text, not the request's own whitespace
    let text = (!list.is_empty()).then(|| Value::Array(list).to_string());
    match index.put_annotations(&study, text.as_deref()).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "No such study"),
        Err(e) => {
            tracing::error!("saving measurements of {study}: {e}");
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The measurements could not be saved",
            )
        }
    }
}

pub fn router(index: Index) -> Router {
    Router::new()
        .route("/ui/api/annotations/{study}", get(get_list).put(put))
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(index)
}
