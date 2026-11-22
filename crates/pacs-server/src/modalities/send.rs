//! `POST /modalities/{id}/store`: studies of this archive sent to a remote AE, by C-STORE,
//! without that AE having asked: pacs-rs sending on its own initiative (PS3.4 Annex B).
//!
//! Only to the AEs instances may be sent to (`PACS_DIMSE_MOVE_DESTINATIONS`): an archive this
//! server only reads from is among the remotes too, and is not written to because someone
//! asked. An account limited to projects sends what it sees, no more.

use std::sync::Arc;
use std::time::Instant;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_dimse::MoveError;
use pacs_index::{Index, InstanceEntry, SeenBy};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;
use super::schema::{Error, Moved, Send, Started};

/// Every instance of `studies`, as `index` sees them; the first study it does not have,
/// as an error.
pub(crate) async fn instances_of(
    index: &Index,
    studies: &[String],
) -> Result<Result<Vec<InstanceEntry>, String>, pacs_index::Error> {
    let mut instances = Vec::new();
    for study in studies {
        let series = index.series(study, None).await?;
        if series.is_empty() {
            return Ok(Err(study.clone()));
        }
        for one in series {
            instances.extend(
                index
                    .instances(study, &one.series_instance_uid, None)
                    .await?,
            );
        }
    }
    Ok(Ok(instances))
}

/// Sends studies to a remote AE.
///
/// A C-STORE of each instance of the studies, on one association pacs-rs opens. The remote AE
/// must be one instances may be sent to (`PACS_DIMSE_MOVE_DESTINATIONS`). The answer, once
/// all is sent, has the counts; with `Asynchronous` it is a job.
#[utoipa::path(
    post,
    path = "/modalities/{id}/store",
    tag = "Remote archives",
    params(("id" = String, Path, description = "The remote AE's id, as `PACS_DIMSE_REMOTES` names it.")),
    request_body(content = Send, description = "The studies."),
    responses(
        (status = 200, description = "All is sent.", body = Moved),
        (status = 202, description = "With `Asynchronous`: the sending started, as a job.", body = Started),
        (status = 400, description = "The body names no valid Study Instance UID.", body = Error),
        (status = 404, description = "No remote AE has this id (the body is empty), or a study is not in the archive, or not in the account's projects.", body = Error),
        (status = 409, description = "The remote AE is not one instances may be sent to.", body = Error),
        (status = 502, description = "The remote AE could not be reached, or stored none of the instances.", body = Error)
    )
)]
pub(super) async fn send(
    State(remotes): State<Arc<Remotes>>,
    seen: Option<Extension<SeenBy>>,
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
    if studies.is_empty() {
        return RestError::bad_request("Bad request", "no Study Instance UID to send")
            .into_response();
    }
    if let Some(bad) = studies.iter().find(|u| !pacs_core::is_valid_uid(u)) {
        return RestError::bad_request("Bad request", format!("{bad:?} is not a UID"))
            .into_response();
    }
    if !config.move_destinations.contains(&remote.id) {
        return RestError::Status {
            status: StatusCode::CONFLICT,
            message: format!("{id} is not an AE instances are sent to"),
            details: Some(format!(
                "list {id} in PACS_DIMSE_MOVE_DESTINATIONS to send it studies"
            )),
        }
        .into_response();
    }
    // what the account sees, and only that
    let index = match &seen {
        Some(Extension(SeenBy(account))) => remotes.index.limited_to(account),
        None => remotes.index.clone(),
    };
    match instances_of(&index, &studies).await {
        Ok(Ok(_)) => {}
        Ok(Err(missing)) => {
            return RestError::Status {
                status: StatusCode::NOT_FOUND,
                message: "No such study".into(),
                details: Some(format!("{missing} is not in the archive")),
            }
            .into_response();
        }
        Err(e) => {
            tracing::error!(error = %e, "reading the studies to send");
            return RestError::Status {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "the studies could not be read".into(),
                details: None,
            }
            .into_response();
        }
    }
    let asynchronous = body
        .as_ref()
        .and_then(|b| b.get("Asynchronous"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if asynchronous {
        return super::jobs::spawn_send(remotes.clone(), id, studies).await;
    }
    let started = Instant::now();
    let result = remotes
        .retriever
        .send(remote, &index, &studies, |_| {
            std::ops::ControlFlow::Continue(())
        })
        .await;
    match result {
        Ok(outcome) => {
            let p = outcome.progress;
            Json(json!({
                "Status": super::retrieve::status_name(outcome.status),
                "Method": "C-STORE",
                "Completed": p.completed,
                "Failed": p.failed,
                "Warning": p.warning,
                "FailedSOPInstanceUIDs": outcome.failed_instances,
                "ElapsedMs": started.elapsed().as_millis() as u64,
            }))
            .into_response()
        }
        Err(MoveError::Request(e)) => RestError::bad_request("Bad request", e).into_response(),
        Err(e) => {
            tracing::info!(remote = %id, error = %e, "a send failed");
            RestError::Status {
                status: StatusCode::BAD_GATEWAY,
                message: format!("C-STORE to {id} failed"),
                details: Some(e.to_string()),
            }
            .into_response()
        }
    }
}
