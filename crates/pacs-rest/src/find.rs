//! `POST /tools/find`: search by DICOM attributes at one level.
//!
//! Matching is that of PS3.4 C.2.2.2 (`pacs_index::matching`): single value, wild card
//! (`*`, `?`), list of UIDs (separated by `\\` or `,`) and range of dates or times, on
//! the attributes the index keeps with each study. What a level cannot search on answers
//! 400, rather than matching something else.

use axum::Json;
use bytes::Bytes;
use pacs_index::{Key, Kind, Match, StudySearch};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use utoipa::ToSchema;

use crate::RestError;
use crate::resources::{instance_json, patient_json, series_json, study_json};
use crate::schema::{Error, Found};
use crate::scope::Seen;

/// A search.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct Find {
    /// `Patient`, `Study`, `Series` or `Instance`, in any case.
    #[serde(rename = "Level")]
    level: String,
    /// Attributes by keyword and the value each must equal: `{"PatientID": "SYN1"}`.
    /// Empty or left out, a patient or study search finds them all; a series or instance
    /// search needs its UID.
    #[serde(rename = "Query", default)]
    #[schema(value_type = Object)]
    query: Map<String, Value>,
    /// Answer with the resources instead of their identifiers.
    #[serde(rename = "Expand", default)]
    expand: bool,
    /// At most this many.
    #[serde(rename = "Limit")]
    limit: Option<i64>,
    /// Skip this many first.
    #[serde(rename = "Since")]
    since: Option<i64>,
}

/// The attributes a search may match at the study level, as (keyword, tag, how its values
/// match): those the index keeps with each study.
const STUDY_KEYS: &[(&str, &str, Kind)] = &[
    ("StudyDate", "00080020", Kind::Date),
    ("StudyTime", "00080030", Kind::Time),
    ("AccessionNumber", "00080050", Kind::Text),
    ("ModalitiesInStudy", "00080061", Kind::Text),
    ("ReferringPhysicianName", "00080090", Kind::Name),
    ("PatientName", "00100010", Kind::Name),
    ("PatientID", "00100020", Kind::Text),
    ("PatientBirthDate", "00100030", Kind::Date),
    ("PatientSex", "00100040", Kind::Text),
    ("StudyInstanceUID", "0020000D", Kind::Uid),
    ("StudyID", "00200010", Kind::Text),
    ("StudyDescription", "00081030", Kind::Text),
];
/// And at the patient level: a patient is found by any of their studies.
const PATIENT_KEYS: &[(&str, &str, Kind)] = &[
    ("PatientName", "00100010", Kind::Name),
    ("PatientID", "00100020", Kind::Text),
    ("PatientBirthDate", "00100030", Kind::Date),
    ("PatientSex", "00100040", Kind::Text),
];

/// The matching keys of a query, each attribute one of `allowed`. An empty value, or `*`,
/// asks nothing. An attribute this level does not know, a value that is not text, or one
/// that is not a match for its attribute (a range that ends before it starts, a UID that
/// is not one) is refused, never read as something else.
fn keys(
    query: &Map<String, Value>,
    allowed: &[(&'static str, &'static str, Kind)],
) -> Result<Vec<Key>, RestError> {
    let mut keys = Vec::new();
    for (name, value) in query {
        let Some((_, tag, kind)) = allowed.iter().find(|(keyword, _, _)| name == keyword) else {
            return Err(RestError::bad_request(
                "Not supported yet",
                format!("cannot search on {name} at this level yet"),
            ));
        };
        let value = value.as_str().ok_or_else(|| {
            RestError::bad_request("Bad request", format!("{name} is not a string"))
        })?;
        if let Some(asked) = Match::parse(*kind, value)
            .map_err(|e| RestError::bad_request("Bad request", format!("{name}: {e}")))?
        {
            keys.push(Key {
                tag,
                kind: *kind,
                asked,
            });
        }
    }
    Ok(keys)
}

/// The UIDs a series or instance search names: one, or a list.
fn uids(
    query: &Map<String, Value>,
    name: &'static str,
    what: &str,
) -> Result<Vec<String>, RestError> {
    if let Some(other) = query.keys().find(|key| *key != name) {
        return Err(RestError::bad_request(
            "Not supported yet",
            format!("cannot search on {other} at this level yet"),
        ));
    }
    let needed =
        || RestError::bad_request("Not supported yet", format!("{what} search needs {name}"));
    let value = query.get(name).ok_or_else(needed)?;
    let value = value
        .as_str()
        .ok_or_else(|| RestError::bad_request("Bad request", format!("{name} is not a string")))?;
    match Match::parse(Kind::Uid, value)
        .map_err(|e| RestError::bad_request("Bad request", format!("{name}: {e}")))?
    {
        Some(Match::Is(uid)) => Ok(vec![uid]),
        Some(Match::AnyOf(uids)) => Ok(uids),
        _ => Err(needed()),
    }
}

/// Searches by DICOM attributes at one level.
///
/// A value matches as its attribute allows (PS3.4 C.2.2.2): itself; with `*` and `?` as
/// wild cards, for text and names; a range `from-to`, `from-` or `-to` for dates and
/// times; a list of UIDs separated by `\\`. Names match without regard to case, the rest
/// with. An empty value, or `*`, asks nothing.
///
/// - `Patient`: `PatientID`, `PatientName`, `PatientBirthDate`, `PatientSex`.
/// - `Study`: those, and `StudyInstanceUID`, `StudyDate`, `StudyTime`, `AccessionNumber`,
///   `StudyID`, `StudyDescription`, `ReferringPhysicianName`, `ModalitiesInStudy`.
/// - `Series`: `SeriesInstanceUID`, needed. `Instance`: `SOPInstanceUID`, needed.
///
/// Another attribute, or a value that is not a match for its attribute, is refused.
#[utoipa::path(
    post,
    path = "/tools/find",
    tag = "Search and archives",
    request_body(content = Find, description = "The level and what to match."),
    responses(
        (status = 200, description = "The identifiers found, or the resources.", body = Found),
        (status = 400, description = "The body is not a search, names an attribute its level cannot search on, or gives a value that is not a match for its attribute.", body = Error)
    )
)]
pub(crate) async fn find(Seen(state): Seen, body: Bytes) -> Result<Json<Value>, RestError> {
    let find: Find = serde_json::from_slice(&body)
        .map_err(|e| RestError::bad_request("Bad request", format!("not a search: {e}")))?;
    let index = state.archive.index();
    let q = &find.query;
    let found: Vec<Value> = match find.level.to_ascii_lowercase().as_str() {
        "patient" => {
            let keys = keys(q, PATIENT_KEYS)?;
            let ids = if keys.is_empty() {
                index.patient_public_ids(None).await?
            } else {
                // a patient is found by any of their studies
                let search = StudySearch {
                    keys,
                    ..StudySearch::default()
                };
                let mut ids: Vec<String> = Vec::new();
                for study in index.search_studies(&search).await? {
                    let Some(id) = study.public_id else { continue };
                    if let Some(study) = index.rest_study(&id).await?
                        && !ids.contains(&study.patient_public_id)
                    {
                        ids.push(study.patient_public_id);
                    }
                }
                ids.sort();
                ids
            };
            let ids = ids
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
            let search = StudySearch {
                keys: keys(q, STUDY_KEYS)?,
                // a range of dates and a range of times are read apart, as C-FIND reads
                // them without negotiation (PS3.4 C.2.2.2.5.4)
                combined_date_time: false,
                limit: find.limit.map(|limit| limit.max(0) as usize),
                offset: find.since.unwrap_or(0).max(0) as usize,
            };
            search
                .check()
                .map_err(|e| RestError::bad_request("Bad request", e))?;
            let mut found = Vec::new();
            for study in index.search_studies(&search).await? {
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
            let mut found = Vec::new();
            for uid in uids(q, "SeriesInstanceUID", "a series")? {
                let Some(id) = index.series_public_id(&uid).await? else {
                    continue;
                };
                if find.expand {
                    found.extend(index.rest_series(&id).await?.map(series_json));
                } else {
                    found.push(json!(id));
                }
            }
            found
        }
        "instance" => {
            let mut found = Vec::new();
            for uid in uids(q, "SOPInstanceUID", "an instance")? {
                let Some(instance) = index.rest_instance_by_uid(&uid).await? else {
                    continue;
                };
                found.push(if find.expand {
                    instance_json(instance)
                } else {
                    json!(instance.public_id)
                });
            }
            found
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
