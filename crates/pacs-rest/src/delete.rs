//! `DELETE /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}`: the
//! resource, everything under it, and every ancestor left empty. The answer names the
//! closest ancestor still there, as the reference server's does.

use axum::Json;
use axum::extract::{Path, State};
use pacs_index::Level;
use serde_json::{Value, json};

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

pub(crate) async fn patient(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Patient, &id).await
}

pub(crate) async fn study(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Study, &id).await
}

pub(crate) async fn series(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Series, &id).await
}

pub(crate) async fn instance(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    delete(&state, Level::Instance, &id).await
}
