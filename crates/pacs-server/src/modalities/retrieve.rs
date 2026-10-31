//! `POST /modalities/{id}/move`: a remote AE asked, by C-MOVE, to send studies to pacs-rs,
//! where they arrive through its C-STORE SCP like any other.

use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pacs_dimse::{MoveError, Status};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;

/// The body names the studies: `{"StudyInstanceUID": "1.2.3"}` or a list of them. The
/// destination is always pacs-rs's own AE title: this route pulls studies in, it does not
/// send a hospital's studies anywhere else. Answers when the move is over: 200 with the
/// final status (`Success`, `Warning` when some instances failed, named in
/// `FailedSOPInstanceUIDs`) and the counts, and a `Warning` header when nothing was sent; 502 when the remote AE cannot be reached or
/// fails, and says why (a destination it does not know means pacs-rs is not registered
/// there); 409 when pacs-rs is not listening for DIMSE, so nothing could arrive; 400 for a
/// body without valid UIDs; 404 for an unknown id.
pub(super) async fn move_studies(
    State(remotes): State<Arc<Remotes>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    let config = &remotes.config;
    let Some(remote) = config.remote(&id) else {
        return RestError::NotFound.into_response();
    };
    let studies = match body.as_ref().and_then(|b| b.get("StudyInstanceUID")) {
        Some(Value::String(uid)) => vec![uid.clone()],
        Some(Value::Array(uids)) if uids.iter().all(Value::is_string) => uids
            .iter()
            .filter_map(|u| u.as_str().map(str::to_owned))
            .collect(),
        _ => {
            return RestError::bad_request(
                "Bad request",
                r#"the body is {"StudyInstanceUID": "<uid>"} or a list of UIDs"#,
            )
            .into_response();
        }
    };
    if !remotes.receiving {
        return RestError::Status {
            status: StatusCode::CONFLICT,
            message: "pacs-rs is not listening for DIMSE".into(),
            details: Some("set PACS_DIMSE_LISTEN so the remote AE can send the studies".into()),
        }
        .into_response();
    }
    let started = Instant::now();
    let result = pacs_dimse::move_studies(
        remote,
        &config.ae_title,
        &studies,
        &config.ae_title,
        config.timeout,
        config.move_timeout,
        |_| std::ops::ControlFlow::Continue(()),
    )
    .await;
    match result {
        Ok(outcome) => {
            let p = outcome.progress;
            let mut response = Json(json!({
                "Status": status_name(outcome.status),
                "Completed": p.completed,
                "Failed": p.failed,
                "Warning": p.warning,
                "FailedSOPInstanceUIDs": outcome.failed_instances,
                "ElapsedMs": started.elapsed().as_millis() as u64,
            }))
            .into_response();
            // Success with nothing done is what an archive answers for a study it does not
            // have (no match, so no sub-operations)
            let none = |c: Option<u16>| c.unwrap_or(0) == 0;
            if none(p.completed) && none(p.failed) && none(p.warning) {
                response.headers_mut().append(
                    header::WARNING,
                    HeaderValue::from_static(
                        "299 pacs-rs \"the remote AE sent nothing: it has no such study\"",
                    ),
                );
            }
            response
        }
        Err(MoveError::Request(e)) => RestError::bad_request("Bad request", e).into_response(),
        Err(e) => {
            tracing::info!(remote = %id, error = %e, "C-MOVE failed");
            RestError::Status {
                status: StatusCode::BAD_GATEWAY,
                message: format!("C-MOVE from {id} failed"),
                details: Some(e.to_string()),
            }
            .into_response()
        }
    }
}

fn status_name(status: Status) -> &'static str {
    match status.class() {
        pacs_dimse::StatusClass::Success => "Success",
        pacs_dimse::StatusClass::Cancel => "Cancel",
        _ => "Warning",
    }
}
