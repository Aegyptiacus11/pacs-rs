//! `/trash`: deletions that can still be undone, with `PACS_TRASH_DAYS` above 0 (30 by
//! default): see `pacs_archive::Archive::with_trash`. A deletion forgets the resource at once;
//! its files wait in the store until the entry is restored, emptied, or older than the trash
//! keeps it, when a task of the server empties it.
//!
//! For a role with `delete`, and an account not limited to projects: the trash holds what
//! every project deleted.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use pacs_archive::Archive;
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::Limited;

/// How often the server empties what the trash keeps no longer.
const EMPTY_EVERY: Duration = Duration::from_secs(3600);

/// A deletion in the trash.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrashEntry {
    /// Its id, for restoring or emptying it.
    #[serde(rename = "ID")]
    pub id: String,
    /// When it was deleted, in milliseconds since the Unix epoch.
    pub deleted_ms: i64,
    /// When the server empties it, in milliseconds since the Unix epoch.
    pub emptied_ms: i64,
    /// What the deletion named: `Patient`, `Study`, `Series` or `Instance`; `Modification`
    /// for a patient modified or merged, whose restore undoes the change.
    pub level: String,
    /// Its public ID.
    #[serde(rename = "PublicID")]
    pub public_id: String,
    /// The studies it took from, a part or all: `StudyInstanceUID`, `PatientID`,
    /// `PatientName`, and the `project` the study was in; for a modification, the values
    /// before it, and in `changed` the ones it gave.
    #[schema(value_type = Vec<Object>)]
    pub studies: Value,
    /// Files waiting.
    pub files: i64,
}

/// What a restore did.
#[derive(Debug, Serialize, ToSchema)]
pub struct Restored {
    /// Files indexed again.
    pub restored: usize,
    /// Files left in the trash, each with why: an instance stored again since the deletion
    /// keeps the newer copy.
    pub kept: Vec<Kept>,
}

/// A file a restore left in the trash.
#[derive(Debug, Serialize, ToSchema)]
pub struct Kept {
    /// Where it was before the deletion.
    pub object_key: String,
    /// Why it was left.
    pub reason: String,
}

fn failed(status: StatusCode, detail: &str) -> Response {
    (status, Json(serde_json::json!({ "detail": detail }))).into_response()
}

fn limited(account: Option<axum::Extension<Limited>>) -> Option<Response> {
    account.map(|_| {
        failed(
            StatusCode::FORBIDDEN,
            "The trash holds what every project deleted: not for an account limited to projects",
        )
    })
}

fn parse(id: &str) -> Option<i64> {
    id.parse().ok()
}

/// Lists the trash.
///
/// The deletions that can still be undone, the latest first. Empty when `PACS_TRASH_DAYS` is 0.
#[utoipa::path(
    get,
    path = "/trash",
    tag = "Trash",
    responses(
        (status = 200, description = "The deletions in the trash.", body = Vec<TrashEntry>),
        (status = 403, description = "An account limited to projects.")
    )
)]
async fn list(
    State(archive): State<Archive>,
    account: Option<axum::Extension<Limited>>,
) -> Response {
    if let Some(refused) = limited(account) {
        return refused;
    }
    let keep_ms =
        i64::try_from(archive.trash_days().saturating_mul(86_400_000)).unwrap_or(i64::MAX);
    match archive.index().trash().await {
        Ok(entries) => Json(
            entries
                .into_iter()
                .map(|e| TrashEntry {
                    id: e.id.to_string(),
                    deleted_ms: e.deleted_ms,
                    emptied_ms: e.deleted_ms.saturating_add(keep_ms),
                    level: e.level,
                    public_id: e.public_id,
                    // the annotations are restored, not shown
                    studies: Value::from(
                        e.studies
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|mut s| {
                                if let Some(s) = s.as_object_mut() {
                                    s.remove("annotations");
                                    // what an undone modification needs, not for people
                                    s.remove("rekeyed");
                                }
                                s
                            })
                            .collect::<Vec<_>>(),
                    ),
                    files: e.files,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "reading the trash");
            failed(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The trash could not be read",
            )
        }
    }
}

/// Restores a deletion.
///
/// Its files are indexed again as if received, each study in the project it was in, with the viewer's annotations of a whole study. An instance stored again since the deletion keeps the newer copy, and that file stays in the trash. The entry goes once no file is left in it.
#[utoipa::path(
    post,
    path = "/trash/{id}/restore",
    tag = "Trash",
    params(("id" = String, Path, description = "The entry's id.")),
    responses(
        (status = 200, description = "What was restored, and what was left.", body = Restored),
        (status = 403, description = "An account limited to projects."),
        (status = 404, description = "No such entry."),
        (status = 503, description = "The store or the index did not answer.")
    )
)]
async fn restore(
    State(archive): State<Archive>,
    account: Option<axum::Extension<Limited>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refused) = limited(account) {
        return refused;
    }
    let Some(id) = parse(&id) else {
        return failed(StatusCode::NOT_FOUND, "No such entry in the trash");
    };
    match archive.restore(id).await {
        Ok(Some(done)) => {
            tracing::info!(
                entry = id,
                restored = done.restored,
                kept = done.kept.len(),
                "restored from the trash"
            );
            Json(Restored {
                restored: done.restored,
                kept: done
                    .kept
                    .into_iter()
                    .map(|(object_key, reason)| Kept { object_key, reason })
                    .collect(),
            })
            .into_response()
        }
        Ok(None) => failed(StatusCode::NOT_FOUND, "No such entry in the trash"),
        Err(e) => {
            tracing::error!(entry = id, error = %e, "a restore stopped");
            failed(StatusCode::SERVICE_UNAVAILABLE, &e.to_string())
        }
    }
}

/// Empties a deletion now.
///
/// Its files are removed for good; it cannot be restored after this.
#[utoipa::path(
    delete,
    path = "/trash/{id}",
    tag = "Trash",
    params(("id" = String, Path, description = "The entry's id.")),
    responses(
        (status = 204, description = "Emptied."),
        (status = 403, description = "An account limited to projects."),
        (status = 404, description = "No such entry."),
        (status = 503, description = "The store or the index did not answer.")
    )
)]
async fn empty(
    State(archive): State<Archive>,
    account: Option<axum::Extension<Limited>>,
    Path(id): Path<String>,
) -> Response {
    if let Some(refused) = limited(account) {
        return refused;
    }
    let Some(id) = parse(&id) else {
        return failed(StatusCode::NOT_FOUND, "No such entry in the trash");
    };
    match archive.empty_trash(id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => failed(StatusCode::NOT_FOUND, "No such entry in the trash"),
        Err(e) => {
            tracing::error!(entry = id, error = %e, "emptying a trash entry stopped");
            failed(StatusCode::SERVICE_UNAVAILABLE, &e.to_string())
        }
    }
}

fn described() -> OpenApiRouter<Archive> {
    OpenApiRouter::new()
        .routes(routes!(list))
        .routes(routes!(restore))
        .routes(routes!(empty))
}

/// `/trash`.
pub fn router(archive: Archive) -> axum::Router {
    described().with_state(archive).split_for_parts().0
}

pub(crate) fn api() -> OpenApi {
    described().into_openapi()
}

/// Empty, once an hour, the entries the trash keeps no longer; none with no trash.
pub fn spawn_emptying(archive: Archive) {
    if archive.trash_days() == 0 {
        return;
    }
    tokio::spawn(async move {
        loop {
            match archive.empty_old_trash().await {
                Ok(0) => {}
                Ok(emptied) => tracing::info!(emptied, "old trash entries emptied"),
                Err(error) => {
                    tracing::error!(%error, "emptying the trash stopped; trying again later")
                }
            }
            tokio::time::sleep(EMPTY_EVERY).await;
        }
    });
}
