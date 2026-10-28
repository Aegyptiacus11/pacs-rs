//! `POST /tools/find`: search by DICOM attributes at one level.
//!
//! Exact matching on the attributes the index has columns for. Wildcards and ranges
//! answer 400 for now, rather than matching something else.

use axum::Json;
use axum::extract::State;
use bytes::Bytes;
use pacs_index::StudyFilter;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::resources::{instance_json, patient_json, series_json, study_json};
use crate::{AppState, RestError};

#[derive(Debug, Deserialize)]
pub(crate) struct Find {
    #[serde(rename = "Level")]
    level: String,
    #[serde(rename = "Query", default)]
    query: Map<String, Value>,
    #[serde(rename = "Expand", default)]
    expand: bool,
    #[serde(rename = "Limit")]
    limit: Option<i64>,
    #[serde(rename = "Since")]
    since: Option<i64>,
}

fn exact<'a>(query: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, RestError> {
    let Some(value) = query.get(key) else {
        return Ok(None);
    };
    let value = value
        .as_str()
        .ok_or_else(|| RestError::bad_request("Bad request", format!("{key} is not a string")))?;
    if value.contains(['*', '?', '\\']) || (key.ends_with("Date") && value.contains('-')) {
        return Err(RestError::bad_request(
            "Not supported yet",
            format!("only exact matching is supported yet: {key}={value}"),
        ));
    }
    Ok(Some(value))
}

fn only(query: &Map<String, Value>, allowed: &[&str]) -> Result<(), RestError> {
    match query.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(key) => Err(RestError::bad_request(
            "Not supported yet",
            format!("cannot search on {key} at this level yet"),
        )),
        None => Ok(()),
    }
}

/// The body is JSON whatever its Content-Type says, as the reference server reads it.
pub(crate) async fn find(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    let find: Find = serde_json::from_slice(&body)
        .map_err(|e| RestError::bad_request("Bad request", format!("not a search: {e}")))?;
    let index = state.archive.index();
    let q = &find.query;
    let found: Vec<Value> = match find.level.to_ascii_lowercase().as_str() {
        "patient" => {
            only(q, &["PatientID"])?;
            let ids = index
                .patient_public_ids(exact(q, "PatientID")?)
                .await?
                .into_iter()
                .skip(find.since.unwrap_or(0).max(0) as usize)
                .take(find.limit.map_or(usize::MAX, |l| l.max(0) as usize));
            let mut found = Vec::new();
            for id in ids {
                if find.expand {
                    if let Some(p) = index.rest_patient(&id).await? {
                        found.push(patient_json(p));
                    }
                } else {
                    found.push(json!(id));
                }
            }
            found
        }
        "study" => {
            only(
                q,
                &[
                    "StudyInstanceUID",
                    "PatientID",
                    "AccessionNumber",
                    "StudyDate",
                ],
            )?;
            let filter = StudyFilter {
                study_instance_uid: exact(q, "StudyInstanceUID")?.map(str::to_owned),
                patient_id: exact(q, "PatientID")?.map(str::to_owned),
                accession_number: exact(q, "AccessionNumber")?.map(str::to_owned),
                study_date: exact(q, "StudyDate")?.map(str::to_owned),
                limit: find.limit,
                offset: find.since,
            };
            let mut found = Vec::new();
            for study in index.studies(&filter).await? {
                let Some(id) = study.public_id else { continue };
                if find.expand {
                    if let Some(s) = index.rest_study(&id).await? {
                        found.push(study_json(s));
                    }
                } else {
                    found.push(json!(id));
                }
            }
            found
        }
        "series" => {
            only(q, &["SeriesInstanceUID"])?;
            let uid = exact(q, "SeriesInstanceUID")?.ok_or_else(|| {
                RestError::bad_request(
                    "Not supported yet",
                    "a series search needs SeriesInstanceUID",
                )
            })?;
            match index.series_public_id(uid).await? {
                Some(id) if find.expand => index
                    .rest_series(&id)
                    .await?
                    .map(series_json)
                    .into_iter()
                    .collect(),
                Some(id) => vec![json!(id)],
                None => vec![],
            }
        }
        "instance" => {
            only(q, &["SOPInstanceUID"])?;
            let uid = exact(q, "SOPInstanceUID")?.ok_or_else(|| {
                RestError::bad_request(
                    "Not supported yet",
                    "an instance search needs SOPInstanceUID",
                )
            })?;
            match index.rest_instance_by_uid(uid).await? {
                Some(i) if find.expand => vec![instance_json(i)],
                Some(i) => vec![json!(i.public_id)],
                None => vec![],
            }
        }
        other => {
            return Err(RestError::bad_request(
                "Bad request",
                format!("unknown or unsupported level {other:?}"),
            ));
        }
    };
    Ok(Json(Value::Array(found)))
}
