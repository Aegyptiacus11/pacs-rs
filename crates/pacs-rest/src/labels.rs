//! Labels on patients, studies, series and instances: free words a user or a tool puts on a
//! resource to group and find it, as the reference server has them. A label is 1 to 64
//! ASCII letters, digits, `_` and `-` ([`pacs_index::labels::is_label`]).
//!
//! - `GET /{level}/{id}/labels`: the resource's labels, sorted.
//! - `GET /{level}/{id}/labels/{label}`: 200 when it has it, 404 when not; the body is empty.
//! - `PUT /{level}/{id}/labels/{label}`: put it on (once, however often asked).
//! - `DELETE /{level}/{id}/labels/{label}`: take it off.
//! - `GET /tools/labels`: every label in use.
//!
//! A resource the account does not see is not there (404), as on every route.

use axum::Json;
use axum::extract::Path;
use axum::http::StatusCode;
use pacs_index::{Index, Level};
use serde_json::{Value, json};

use crate::RestError;
use crate::schema::Error;
use crate::scope::Seen;

/// Whether the resource is there, as the request's account sees it.
async fn exists(index: &Index, level: Level, id: &str) -> Result<bool, RestError> {
    Ok(match level {
        Level::Patient => index.rest_patient(id).await?.is_some(),
        Level::Study => index.rest_study(id).await?.is_some(),
        Level::Series => index.rest_series(id).await?.is_some(),
        Level::Instance => index.rest_instance(id).await?.is_some(),
    })
}

fn checked(label: &str) -> Result<(), RestError> {
    if pacs_index::labels::is_label(label) {
        Ok(())
    } else {
        Err(RestError::bad_request(
            "Bad request",
            format!(
                "{label:?} is not a label: 1 to {} ASCII letters, digits, _ and -",
                pacs_index::labels::LABEL_MAX
            ),
        ))
    }
}

async fn list(index: &Index, level: Level, id: &str) -> Result<Json<Value>, RestError> {
    if !exists(index, level, id).await? {
        return Err(RestError::NotFound);
    }
    Ok(Json(json!(index.labels(id).await?)))
}

async fn has(index: &Index, level: Level, id: &str, label: &str) -> Result<StatusCode, RestError> {
    if !exists(index, level, id).await? || !index.labels(id).await?.iter().any(|l| l == label) {
        return Err(RestError::NotFound);
    }
    Ok(StatusCode::OK)
}

async fn put(index: &Index, level: Level, id: &str, label: &str) -> Result<StatusCode, RestError> {
    checked(label)?;
    if !exists(index, level, id).await? {
        return Err(RestError::NotFound);
    }
    index.add_label(level, id, label).await?;
    Ok(StatusCode::OK)
}

async fn remove(
    index: &Index,
    level: Level,
    id: &str,
    label: &str,
) -> Result<StatusCode, RestError> {
    checked(label)?;
    if !exists(index, level, id).await? {
        return Err(RestError::NotFound);
    }
    index.remove_label(id, label).await?;
    Ok(StatusCode::OK)
}

/// The handlers of one level, under its path and its tag.
macro_rules! level {
    ($module:ident, $level:expr, $tag:literal, $list:literal, $one:literal,
     $list_doc:literal, $has_doc:literal, $put_doc:literal, $delete_doc:literal) => {
        pub(crate) mod $module {
            use super::*;

            #[doc = $list_doc]
            #[utoipa::path(
                get,
                path = $list,
                tag = $tag,
                params(("id" = String, Path, description = "The resource's identifier.")),
                responses(
                    (status = 200, description = "Its labels.", body = Vec<String>),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn labels(
                Seen(state): Seen,
                Path(id): Path<String>,
            ) -> Result<Json<Value>, RestError> {
                list(state.archive.index(), $level, &id).await
            }

            #[doc = $has_doc]
            #[utoipa::path(
                get,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("label" = String, Path, description = "The label.")
                ),
                responses(
                    (status = 200, description = "It has the label; the body is empty."),
                    (status = 404, description = "No such resource, or it does not have the label; the body is empty.")
                )
            )]
            pub(crate) async fn label(
                Seen(state): Seen,
                Path((id, label)): Path<(String, String)>,
            ) -> Result<StatusCode, RestError> {
                has(state.archive.index(), $level, &id, &label).await
            }

            #[doc = $put_doc]
            #[utoipa::path(
                put,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("label" = String, Path, description = "The label.")
                ),
                responses(
                    (status = 200, description = "It has the label; the body is empty."),
                    (status = 400, description = "Not a label.", body = Error),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn put_label(
                Seen(state): Seen,
                Path((id, label)): Path<(String, String)>,
            ) -> Result<StatusCode, RestError> {
                put(state.archive.index(), $level, &id, &label).await
            }

            #[doc = $delete_doc]
            #[utoipa::path(
                delete,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("label" = String, Path, description = "The label.")
                ),
                responses(
                    (status = 200, description = "It does not have the label; the body is empty."),
                    (status = 400, description = "Not a label.", body = Error),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn delete_label(
                Seen(state): Seen,
                Path((id, label)): Path<(String, String)>,
            ) -> Result<StatusCode, RestError> {
                remove(state.archive.index(), $level, &id, &label).await
            }
        }
    };
}

level!(
    patients,
    Level::Patient,
    "Patients",
    "/patients/{id}/labels",
    "/patients/{id}/labels/{label}",
    "The labels of a patient, sorted.",
    "Whether a patient has a label: 200 if so, 404 if not; the body is empty.",
    "Put a label on a patient: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.",
    "Take a label off a patient. Taking one it does not have changes nothing."
);
level!(
    studies,
    Level::Study,
    "Studies",
    "/studies/{id}/labels",
    "/studies/{id}/labels/{label}",
    "The labels of a study, sorted.",
    "Whether a study has a label: 200 if so, 404 if not; the body is empty.",
    "Put a label on a study: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.",
    "Take a label off a study. Taking one it does not have changes nothing."
);
level!(
    series,
    Level::Series,
    "Series",
    "/series/{id}/labels",
    "/series/{id}/labels/{label}",
    "The labels of a series, sorted.",
    "Whether a series has a label: 200 if so, 404 if not; the body is empty.",
    "Put a label on a series: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.",
    "Take a label off a series. Taking one it does not have changes nothing."
);
level!(
    instances,
    Level::Instance,
    "Instances",
    "/instances/{id}/labels",
    "/instances/{id}/labels/{label}",
    "The labels of an instance, sorted.",
    "Whether an instance has a label: 200 if so, 404 if not; the body is empty.",
    "Put a label on an instance: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.",
    "Take a label off an instance. Taking one it does not have changes nothing."
);

/// Every label in use on what the account sees, sorted.
#[utoipa::path(
    get,
    path = "/tools/labels",
    tag = "Search and archives",
    responses((status = 200, description = "The labels.", body = Vec<String>))
)]
pub(crate) async fn all(Seen(state): Seen) -> Result<Json<Value>, RestError> {
    Ok(Json(json!(state.archive.index().all_labels().await?)))
}
