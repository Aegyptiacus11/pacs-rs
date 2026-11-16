//! `DELETE /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}`: the
//! resource, everything under it, and every ancestor left empty. The answer names the
//! closest ancestor still there, as the reference server's does.

use axum::Json;
use axum::extract::Path;
use pacs_index::Level;
use serde_json::{Value, json};

use crate::schema::Deleted;
use crate::scope::Seen;
use crate::{AppState, RestError};

async fn delete(state: &AppState, level: Level, id: &str) -> Result<Json<Value>, RestError> {
    let deleted = state
        .archive
        .delete(level, id)
        .await?
        .ok_or(RestError::NotFound)?;
    let ancestor = deleted.remaining_ancestor.map(|(level, id)| {
        json!({
            "ID": id,
            "Path": format!("/{}/{id}", level.path()),
            "Type": level.name(),
        })
    });
    Ok(Json(json!({ "RemainingAncestor": ancestor })))
}

/// Deletes a patient.
///
/// Everything under it goes too, and every level above it that is left empty. The index
/// entries are removed first and the files after. There is no undo.
#[utoipa::path(
    delete,
    path = "/patients/{id}",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "Deleted; the closest level still there.", body = Deleted),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Patient, &id).await
}

/// Deletes a study.
///
/// Everything under it goes too, and every level above it that is left empty. The index
/// entries are removed first and the files after. There is no undo.
#[utoipa::path(
    delete,
    path = "/studies/{id}",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    responses(
        (status = 200, description = "Deleted; the closest level still there.", body = Deleted),
        (status = 404, description = "No study has this identifier; the body is empty.")
    )
)]
pub(crate) async fn study(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Study, &id).await
}

/// Deletes a series.
///
/// Everything under it goes too, and every level above it that is left empty. The index
/// entries are removed first and the files after. There is no undo.
#[utoipa::path(
    delete,
    path = "/series/{id}",
    tag = "Series",
    params(("id" = String, Path, description = "The series's identifier.")),
    responses(
        (status = 200, description = "Deleted; the closest level still there.", body = Deleted),
        (status = 404, description = "No series has this identifier; the body is empty.")
    )
)]
pub(crate) async fn series(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Series, &id).await
}

/// Deletes an instance.
///
/// Everything under it goes too, and every level above it that is left empty. The index
/// entries are removed first and the files after. There is no undo.
#[utoipa::path(
    delete,
    path = "/instances/{id}",
    tag = "Instances",
    params(("id" = String, Path, description = "The instance's identifier.")),
    responses(
        (status = 200, description = "Deleted; the closest level still there.", body = Deleted),
        (status = 404, description = "No instance has this identifier; the body is empty.")
    )
)]
pub(crate) async fn instance(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Instance, &id).await
}
