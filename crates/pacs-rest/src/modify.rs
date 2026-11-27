//! `POST /patients/{id}/modify` and `/patients/{id}/merge`: a patient's name, ID, birth date
//! or sex changed in every stored file, or a patient merged into another by taking theirs.
//! The files they replace wait in the trash, whose restore undoes the change.

use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use bytes::Bytes;
use pacs_archive::{Modified, ModifyError};
use pacs_core::patient::PatientChange;
use serde_json::{Value, json};

use crate::schema::{Error, Merge, Modification, ModifyPatient};
use crate::scope::Seen;
use crate::{AppState, RestError};

fn refused(error: ModifyError) -> RestError {
    match error {
        ModifyError::Invalid(why) => RestError::bad_request("Bad request", why),
        ModifyError::Project(..) => RestError::Status {
            status: StatusCode::CONFLICT,
            message: "Not modified".into(),
            details: Some(error.to_string()),
        },
        ModifyError::File(..) => RestError::internal(error),
        ModifyError::Store(e) => e.into(),
        ModifyError::Index(e) => e.into(),
    }
}

/// An account limited to projects sees de-identified copies only, which are not modified.
fn whole(state: &AppState) -> Result<(), RestError> {
    match state.archive.index().account() {
        Some(_) => Err(RestError::Status {
            status: StatusCode::FORBIDDEN,
            message: "Forbidden".into(),
            details: Some(
                "an account limited to projects sees de-identified copies, which are not modified"
                    .into(),
            ),
        }),
        None => Ok(()),
    }
}

fn answer(done: Modified) -> Json<Value> {
    Json(json!({
        "ID": done.patient,
        "Path": format!("/patients/{}", done.patient),
        "Instances": done.instances,
        "Trash": done.trash.map(|id| id.to_string()),
    }))
}

/// Modifies a patient.
///
/// Patient's Name, Patient ID, Patient's Birth Date or Patient's Sex, each given, is written
/// into every stored file of the patient; the rest of each file, pixel data and UIDs included,
/// stays as it is. A new Patient ID gives the patient, their studies, series and instances new
/// identifiers, which the answer names: the old ones are gone, and labels and metadata move
/// to the new ones. The files replaced wait in the trash, and restoring that entry
/// (`POST /trash/{id}/restore`) undoes the change. A project's de-identified copy is not
/// modified.
#[utoipa::path(
    post,
    path = "/patients/{id}/modify",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    request_body(content = ModifyPatient, description = "The new values; an attribute left out stays as it is."),
    responses(
        (status = 200, description = "Modified: the patient's identifier now, and the trash entry that undoes it.", body = Modification),
        (status = 400, description = "No value given, or one that does not fit its value representation.", body = Error),
        (status = 403, description = "An account limited to projects.", body = Error),
        (status = 404, description = "No patient has this identifier; the body is empty."),
        (status = 409, description = "A study of the patient is a project's de-identified copy.", body = Error)
    )
)]
pub(crate) async fn patient(
    Seen(state): Seen,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    whole(&state)?;
    let asked: ModifyPatient = serde_json::from_slice(&body).map_err(|e| {
        RestError::bad_request("Bad request", format!("not a patient modification: {e}"))
    })?;
    let change = PatientChange {
        name: asked.patient_name,
        id: asked.patient_id,
        birth_date: asked.patient_birth_date,
        sex: asked.patient_sex,
    };
    let done = state
        .archive
        .modify_patient(&id, &change)
        .await
        .map_err(refused)?
        .ok_or(RestError::NotFound)?;
    Ok(answer(done))
}

/// Merges a patient into another.
///
/// The patient's files take the other's Patient's Name, Patient ID, Birth Date and Sex, as
/// the other's latest study has them, and with them the other's identifier: their studies
/// are the other's from then on. As for a modification, the files replaced wait in the trash,
/// and restoring that entry undoes the merge.
#[utoipa::path(
    post,
    path = "/patients/{id}/merge",
    tag = "Patients",
    params(("id" = String, Path, description = "The identifier of the patient merged, who goes.")),
    request_body(content = Merge, description = "The patient kept."),
    responses(
        (status = 200, description = "Merged: the patient kept, and the trash entry that undoes it.", body = Modification),
        (status = 400, description = "Not a merge, or a patient merged into themselves.", body = Error),
        (status = 403, description = "An account limited to projects.", body = Error),
        (status = 404, description = "Either patient is not there; the body is empty."),
        (status = 409, description = "A study of the patient merged is a project's de-identified copy.", body = Error)
    )
)]
pub(crate) async fn merge(
    Seen(state): Seen,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    whole(&state)?;
    let asked: Merge = serde_json::from_slice(&body)
        .map_err(|e| RestError::bad_request("Bad request", format!("not a merge: {e}")))?;
    let done = state
        .archive
        .merge_patients(&id, &asked.into)
        .await
        .map_err(refused)?
        .ok_or(RestError::NotFound)?;
    Ok(answer(done))
}
