//! Metadata of patients, studies, series and instances, as the reference server has them:
//! values under names, some the server's own (read only), the others a user's or a tool's.
//!
//! - The server's own: `LastUpdate` at every level; and of an instance `IndexInSeries`
//!   (when it has an Instance Number), `ReceptionDate`, `SopClassUid` and `TransferSyntax`.
//! - A user's: 1 to 64 ASCII letters, digits, `_` and `-`, not a name of the server's own;
//!   the value is text, at most [`VALUE_MAX`] bytes.
//!
//! `GET /{level}/{id}/metadata` lists the names, `?expand` the names and values;
//! `GET`, `PUT` (the body is the value) and `DELETE` on `/{level}/{id}/metadata/{name}`.
//! A resource the account does not see is not there (404), as on every route.

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_index::{Index, Level};
use serde_json::{Value, json};

use crate::RestError;
use crate::schema::Error;
use crate::scope::Seen;

/// The largest value of a user's metadata.
pub const VALUE_MAX: usize = 64 * 1024;

/// The names of the server's own metadata, at any level.
const OWN: [&str; 5] = [
    "IndexInSeries",
    "LastUpdate",
    "ReceptionDate",
    "SopClassUid",
    "TransferSyntax",
];

/// The resource's metadata of the server's own; `None` when it is not there.
async fn own(
    index: &Index,
    level: Level,
    id: &str,
) -> Result<Option<BTreeMap<String, String>>, RestError> {
    let mut own = BTreeMap::new();
    match level {
        Level::Patient => {
            let Some(p) = index.rest_patient(id).await? else {
                return Ok(None);
            };
            own.insert("LastUpdate".into(), p.last_update);
        }
        Level::Study => {
            let Some(s) = index.rest_study(id).await? else {
                return Ok(None);
            };
            own.insert("LastUpdate".into(), s.last_update);
        }
        Level::Series => {
            let Some(s) = index.rest_series(id).await? else {
                return Ok(None);
            };
            own.insert("LastUpdate".into(), s.last_update);
        }
        Level::Instance => {
            let Some(i) = index.rest_instance(id).await? else {
                return Ok(None);
            };
            if let Some(n) = i.instance_number {
                own.insert("IndexInSeries".into(), n.to_string());
            }
            own.insert("ReceptionDate".into(), i.reception_date);
            own.insert("SopClassUid".into(), i.sop_class_uid);
            own.insert("TransferSyntax".into(), i.transfer_syntax_uid);
        }
    }
    Ok(Some(own))
}

/// All of a resource's metadata, the server's own and the user's; `None` when it is not
/// there.
async fn all(
    index: &Index,
    level: Level,
    id: &str,
) -> Result<Option<BTreeMap<String, String>>, RestError> {
    let Some(mut values) = own(index, level, id).await? else {
        return Ok(None);
    };
    values.extend(index.metadata_values(id).await?);
    Ok(Some(values))
}

fn user_name(name: &str) -> Result<(), RestError> {
    if OWN.contains(&name) {
        return Err(RestError::Status {
            status: StatusCode::FORBIDDEN,
            message: "Forbidden".into(),
            details: Some(format!("{name} is the server's own metadata, read only")),
        });
    }
    if name.is_empty()
        || name.len() > 64
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(RestError::bad_request(
            "Bad request",
            format!("{name:?} is not a metadata name: 1 to 64 ASCII letters, digits, _ and -"),
        ));
    }
    Ok(())
}

async fn list(
    index: &Index,
    level: Level,
    id: &str,
    expand: bool,
) -> Result<Json<Value>, RestError> {
    let values = all(index, level, id).await?.ok_or(RestError::NotFound)?;
    Ok(Json(if expand {
        json!(values)
    } else {
        json!(values.keys().collect::<Vec<_>>())
    }))
}

async fn one(index: &Index, level: Level, id: &str, name: &str) -> Result<Response, RestError> {
    let values = all(index, level, id).await?.ok_or(RestError::NotFound)?;
    let value = values.get(name).ok_or(RestError::NotFound)?;
    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], value.clone()).into_response())
}

async fn set(
    index: &Index,
    level: Level,
    id: &str,
    name: &str,
    body: Bytes,
) -> Result<StatusCode, RestError> {
    user_name(name)?;
    if body.len() > VALUE_MAX {
        return Err(RestError::bad_request(
            "Bad request",
            format!("a metadata value is at most {VALUE_MAX} bytes"),
        ));
    }
    let value = std::str::from_utf8(&body)
        .map_err(|_| RestError::bad_request("Bad request", "a metadata value is UTF-8 text"))?;
    if own(index, level, id).await?.is_none() {
        return Err(RestError::NotFound);
    }
    index.set_metadata(level, id, name, value).await?;
    Ok(StatusCode::OK)
}

async fn remove(
    index: &Index,
    level: Level,
    id: &str,
    name: &str,
) -> Result<StatusCode, RestError> {
    user_name(name)?;
    if own(index, level, id).await?.is_none() {
        return Err(RestError::NotFound);
    }
    index.remove_metadata(id, name).await?;
    Ok(StatusCode::OK)
}

/// `?expand`: names and values.
fn expand(params: &std::collections::HashMap<String, String>) -> bool {
    params
        .get("expand")
        .is_some_and(|v| !matches!(v.as_str(), "false" | "0"))
}

/// The handlers of one level.
macro_rules! level {
    ($module:ident, $level:expr, $tag:literal, $list:literal, $one:literal,
     $list_doc:literal, $get_doc:literal, $put_doc:literal, $delete_doc:literal) => {
        pub(crate) mod $module {
            use super::*;

            #[doc = $list_doc]
            #[utoipa::path(
                get,
                path = $list,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("expand" = Option<bool>, Query, description = "Names and values instead of names. `?expand` alone means true.")
                ),
                responses(
                    (status = 200, description = "The names, sorted, or an object of names and values.", body = Object),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn metadata(
                Seen(state): Seen,
                Path(id): Path<String>,
                Query(params): Query<std::collections::HashMap<String, String>>,
            ) -> Result<Json<Value>, RestError> {
                list(state.archive.index(), $level, &id, expand(&params)).await
            }

            #[doc = $get_doc]
            #[utoipa::path(
                get,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("name" = String, Path, description = "The metadata's name.")
                ),
                responses(
                    (status = 200, description = "The value, as text.", content_type = "text/plain", body = String),
                    (status = 404, description = "No such resource, or no such metadata; the body is empty.")
                )
            )]
            pub(crate) async fn value(
                Seen(state): Seen,
                Path((id, name)): Path<(String, String)>,
            ) -> Result<Response, RestError> {
                one(state.archive.index(), $level, &id, &name).await
            }

            #[doc = $put_doc]
            #[utoipa::path(
                put,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("name" = String, Path, description = "The metadata's name.")
                ),
                request_body(content = String, content_type = "text/plain", description = "The value: UTF-8 text, at most 64 KiB."),
                responses(
                    (status = 200, description = "Set; the body is empty."),
                    (status = 400, description = "Not a name, or not a value.", body = Error),
                    (status = 403, description = "A name of the server's own metadata.", body = Error),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn put_value(
                Seen(state): Seen,
                Path((id, name)): Path<(String, String)>,
                body: Bytes,
            ) -> Result<StatusCode, RestError> {
                set(state.archive.index(), $level, &id, &name, body).await
            }

            #[doc = $delete_doc]
            #[utoipa::path(
                delete,
                path = $one,
                tag = $tag,
                params(
                    ("id" = String, Path, description = "The resource's identifier."),
                    ("name" = String, Path, description = "The metadata's name.")
                ),
                responses(
                    (status = 200, description = "It has no such metadata now; the body is empty."),
                    (status = 400, description = "Not a name.", body = Error),
                    (status = 403, description = "A name of the server's own metadata.", body = Error),
                    (status = 404, description = "No such resource; the body is empty.")
                )
            )]
            pub(crate) async fn delete_value(
                Seen(state): Seen,
                Path((id, name)): Path<(String, String)>,
            ) -> Result<StatusCode, RestError> {
                remove(state.archive.index(), $level, &id, &name).await
            }
        }
    };
}

level!(
    patients,
    Level::Patient,
    "Patients",
    "/patients/{id}/metadata",
    "/patients/{id}/metadata/{name}",
    "The metadata of a patient: their names, or names and values.",
    "One metadata of a patient, as text.",
    "Set a user's metadata of a patient: the body is the value.",
    "Remove a user's metadata of a patient. Removing one it does not have changes nothing."
);

level!(
    studies,
    Level::Study,
    "Studies",
    "/studies/{id}/metadata",
    "/studies/{id}/metadata/{name}",
    "The metadata of a study: their names, or names and values.",
    "One metadata of a study, as text.",
    "Set a user's metadata of a study: the body is the value.",
    "Remove a user's metadata of a study. Removing one it does not have changes nothing."
);

level!(
    series,
    Level::Series,
    "Series",
    "/series/{id}/metadata",
    "/series/{id}/metadata/{name}",
    "The metadata of a series: their names, or names and values.",
    "One metadata of a series, as text.",
    "Set a user's metadata of a series: the body is the value.",
    "Remove a user's metadata of a series. Removing one it does not have changes nothing."
);

level!(
    instances,
    Level::Instance,
    "Instances",
    "/instances/{id}/metadata",
    "/instances/{id}/metadata/{name}",
    "The metadata of an instance: their names, or names and values.",
    "One metadata of an instance, as text.",
    "Set a user's metadata of an instance: the body is the value.",
    "Remove a user's metadata of an instance. Removing one it does not have changes nothing."
);
