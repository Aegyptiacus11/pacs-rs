//! `POST /tools/find`: search by DICOM attributes at one level.
//!
//! Matching is that of PS3.4 C.2.2.2 (`pacs_index::matching`): single value, wild card
//! (`*`, `?`), list of UIDs (separated by `\\` or `,`) and range of dates or times, on
//! the attributes the index keeps with each study. What a level cannot search on answers
//! 400, rather than matching something else.

use axum::Json;
use bytes::Bytes;
use pacs_index::{Key, Kind, Level, Match, StudySearch};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use utoipa::ToSchema;

use crate::RestError;
use crate::resources::{instance_json, labelled, patient_json, series_json, study_json};
use crate::schema::{Error, Found};
use crate::scope::Seen;

/// A search.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct Find {
    /// `Patient`, `Study`, `Series` or `Instance`, in any case.
    #[serde(rename = "Level")]
    level: String,
    /// Attributes by keyword and the value each must equal: `{"PatientID": "LITS-0"}`.
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
    /// Labels the resources must have, as `LabelsConstraint` says.
    #[serde(rename = "Labels", default)]
    labels: Vec<String>,
    /// `All` (the default): every one of `Labels`; `Any`: at least one; `None`: none of them.
    #[serde(rename = "LabelsConstraint")]
    labels_constraint: Option<String>,
}

/// How a resource's labels must meet those a search names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Constraint {
    All,
    Any,
    None,
}

impl Constraint {
    fn parse(text: Option<&str>) -> Result<Self, RestError> {
        match text {
            None | Some("All") => Ok(Constraint::All),
            Some("Any") => Ok(Constraint::Any),
            Some("None") => Ok(Constraint::None),
            Some(other) => Err(RestError::bad_request(
                "Bad request",
                format!("LabelsConstraint is All, Any or None, not {other:?}"),
            )),
        }
    }

    fn met_by(self, has: &[String], asked: &[String]) -> bool {
        match self {
            Constraint::All => asked.iter().all(|l| has.contains(l)),
            Constraint::Any => asked.iter().any(|l| has.contains(l)),
            Constraint::None => !asked.iter().any(|l| has.contains(l)),
        }
    }
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
///
/// `Labels` keeps the resources with all of them (`LabelsConstraint` `All`, the default),
/// any of them (`Any`) or none of them (`None`). With labels, a series or instance search
/// needs no UID, unless the constraint is `None`.
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
    let constraint = Constraint::parse(find.labels_constraint.as_deref())?;
    if let Some(bad) = find
        .labels
        .iter()
        .find(|l| !pacs_index::labels::is_label(l))
    {
        return Err(RestError::bad_request(
            "Bad request",
            format!("{bad:?} is not a label"),
        ));
    }
    let labelled = !find.labels.is_empty();
    let index = state.archive.index();
    let q = &find.query;
    let since = find.since.unwrap_or(0).max(0) as usize;
    let limit = find.limit.map_or(usize::MAX, |l| l.max(0) as usize);
    // with labels, every match is found first and the page taken after they are read
    let (level, ids, paged) = match find.level.to_ascii_lowercase().as_str() {
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
            (Level::Patient, ids, false)
        }
        "study" => {
            let search = StudySearch {
                keys: keys(q, STUDY_KEYS)?,
                // a range of dates and a range of times are read apart, as C-FIND reads
                // them without negotiation (PS3.4 C.2.2.2.5.4)
                combined_date_time: false,
                limit: (!labelled).then_some(limit).filter(|l| *l != usize::MAX),
                offset: if labelled { 0 } else { since },
            };
            search
                .check()
                .map_err(|e| RestError::bad_request("Bad request", e))?;
            let ids = index
                .search_studies(&search)
                .await?
                .into_iter()
                .filter_map(|study| study.public_id)
                .collect();
            (Level::Study, ids, !labelled)
        }
        "series" => {
            let ids = if q.is_empty() && labelled && constraint != Constraint::None {
                let mut seen = Vec::new();
                for id in index
                    .labelled(Level::Series, &find.labels, constraint == Constraint::All)
                    .await?
                {
                    if index.rest_series(&id).await?.is_some() {
                        seen.push(id);
                    }
                }
                seen
            } else {
                let mut ids = Vec::new();
                for uid in uids(q, "SeriesInstanceUID", "a series")? {
                    ids.extend(index.series_public_id(&uid).await?);
                }
                ids
            };
            (Level::Series, ids, !labelled)
        }
        "instance" => {
            let ids = if q.is_empty() && labelled && constraint != Constraint::None {
                let mut seen = Vec::new();
                for id in index
                    .labelled(Level::Instance, &find.labels, constraint == Constraint::All)
                    .await?
                {
                    if index.rest_instance(&id).await?.is_some() {
                        seen.push(id);
                    }
                }
                seen
            } else {
                let mut ids = Vec::new();
                for uid in uids(q, "SOPInstanceUID", "an instance")? {
                    if let Some(instance) = index.rest_instance_by_uid(&uid).await? {
                        ids.push(instance.public_id);
                    }
                }
                ids
            };
            (Level::Instance, ids, !labelled)
        }
        other => {
            return Err(RestError::bad_request(
                "Bad request",
                format!("unknown or unsupported level {other:?}"),
            ));
        }
    };
    let ids: Vec<String> = if labelled {
        let labels = index.labels_of(&ids).await?;
        ids.into_iter()
            .filter(|id| {
                let has = labels.get(id).map(Vec::as_slice).unwrap_or_default();
                constraint.met_by(has, &find.labels)
            })
            .collect()
    } else {
        ids
    };
    // patients and labelled searches are paged here; studies by the index; series and
    // instances, named by their UIDs, are not paged
    let ids: Vec<String> = match (level, paged) {
        (Level::Patient, _) | (_, false) => ids.into_iter().skip(since).take(limit).collect(),
        _ => ids,
    };
    let mut found = Vec::new();
    for id in ids {
        if !find.expand {
            found.push(json!(id));
            continue;
        }
        match level {
            Level::Patient => found.extend(index.rest_patient(&id).await?.map(patient_json)),
            Level::Study => found.extend(index.rest_study(&id).await?.map(study_json)),
            Level::Series => found.extend(index.rest_series(&id).await?.map(series_json)),
            Level::Instance => found.extend(index.rest_instance(&id).await?.map(instance_json)),
        }
    }
    labelled_answer(index, found).await
}

async fn labelled_answer(
    index: &pacs_index::Index,
    found: Vec<Value>,
) -> Result<Json<Value>, RestError> {
    labelled(index, Value::Array(found)).await
}
