//! `POST /modalities/{id}/move`: a remote AE asked, by C-MOVE, to send studies to pacs-rs,
//! where they arrive through its C-STORE SCP like any other.

use super::schema::{Error, Move, Moved, Started};
use std::sync::Arc;
use std::time::Instant;

use crate::auth::Limited;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pacs_dimse::{MoveError, Status};
use pacs_rest::RestError;
use serde_json::{Value, json};

use super::Remotes;

/// Brings studies in from a remote AE.
///
/// A C-MOVE: the remote AE is asked to send the studies to pacs-rs, which must be listening
/// for DIMSE and be known to that AE as a destination. They arrive through pacs-rs's own
/// C-STORE, as any instance sent to it; sent to a project, they are de-identified on the
/// way in. A `Warning` header says when the remote AE sent nothing, as it does for a study
/// it does not have.
#[utoipa::path(
    post,
    path = "/modalities/{id}/move",
    tag = "Remote archives",
    params(("id" = String, Path, description = "The remote AE's id, as `PACS_DIMSE_REMOTES` names it.")),
    request_body(content = Move, description = "The studies, and where they go."),
    responses(
        (status = 200, description = "The move is over.", body = Moved),
        (status = 202, description = "With `Asynchronous`: the move started, as a job.", body = Started),
        (status = 400, description = "The body names no valid Study Instance UID, or a project that is not one.", body = Error),
        (status = 403, description = "The account is limited to some projects and `Project` is not one of them.", body = Error),
        (status = 404, description = "No remote AE has this id; the body is empty."),
        (status = 409, description = "pacs-rs is not listening for DIMSE, so nothing could arrive.", body = Error),
        (status = 502, description = "The remote AE could not be reached or failed; `Details` says when it does not know pacs-rs as a destination.", body = Error)
    )
)]
pub(super) async fn move_studies(
    State(remotes): State<Arc<Remotes>>,
    limited: Option<Extension<Limited>>,
    Path(id): Path<String>,
    body: Option<Json<Value>>,
) -> Response {
    // An account limited to projects brings studies into one of them only: anything else
    // would arrive where it does not see it, or identified.
    if let Some(Extension(Limited(projects))) = &limited {
        let asked = body
            .as_ref()
            .and_then(|b| b.get("Project"))
            .and_then(Value::as_str);
        if !asked.is_some_and(|name| projects.iter().any(|project| project == name)) {
            return RestError::Status {
                status: StatusCode::FORBIDDEN,
                message: "Limited to projects".into(),
                details: Some(
                    r#"this account moves studies into its own projects only: name one in "Project""#
                        .into(),
                ),
            }
            .into_response();
        }
    }
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
    // the patient the studies are named under, for a remote AE in Patient Root only
    let patient_id = match body.as_ref().and_then(|b| b.get("PatientID")) {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if id.trim().is_empty() => None,
        Some(Value::String(id)) => {
            if let Err(e) = pacs_dimse::scu::retrieve::check_patient_id(id.trim()) {
                return RestError::bad_request("Bad request", e).into_response();
            }
            Some(id.trim().to_owned())
        }
        Some(_) => {
            return RestError::bad_request("Bad request", "PatientID is a string").into_response();
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
    if let Some(bad) = studies.iter().find(|u| !pacs_core::is_valid_uid(u)) {
        return RestError::bad_request("Bad request", format!("{bad:?} is not a UID"))
            .into_response();
    }
    if studies.is_empty() {
        return RestError::bad_request("Bad request", "no Study Instance UID to move")
            .into_response();
    }
    // the studies come to pacs-rs's own AE title, or to a project's to be de-identified
    let destination = match body.as_ref().and_then(|b| b.get("Project")) {
        None => config.ae_title.clone(),
        Some(Value::String(name)) => match remotes.projects.iter().find(|(n, _)| n == name) {
            Some((_, ae_title)) => ae_title.clone(),
            None => {
                return RestError::bad_request(
                    "Bad request",
                    format!("there is no project {name:?}"),
                )
                .into_response();
            }
        },
        Some(_) => {
            return RestError::bad_request("Bad request", "Project is a project's name")
                .into_response();
        }
    };
    let asynchronous = body
        .as_ref()
        .and_then(|b| b.get("Asynchronous"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if asynchronous {
        return super::jobs::spawn_move(remotes.clone(), id, patient_id, studies, destination)
            .await;
    }
    let started = Instant::now();
    let result = pacs_dimse::move_studies(
        remote,
        &config.ae_title,
        patient_id.as_deref(),
        &studies,
        &destination,
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
