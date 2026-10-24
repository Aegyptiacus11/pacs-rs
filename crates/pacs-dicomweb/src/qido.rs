//! QIDO-RS: search for studies, a study's series, a series' instances (PS3.18 10.6).
//!
//! Matching is exact, on the keys the index has columns for; wildcards, ranges and
//! fuzzy matching answer 400 rather than quietly matching something else. Every answer
//! carries the computed attributes (counts, Modalities in Study, Retrieve URL), so
//! `includefield` for them is satisfied without asking.

use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use axum::http::header::ACCEPT;
use axum::response::{IntoResponse, Response};
use pacs_index::StudyFilter;
use serde_json::{Value, json};

use crate::media::{Accept, accepts_json};
use crate::{ApiError, AppState};

pub(crate) const DICOM_JSON: &str = "application/dicom+json";

type Params = Query<Vec<(String, String)>>;

pub(crate) fn study_url(base: &str, study: &str) -> String {
    format!("{base}/studies/{study}")
}

pub(crate) fn series_url(base: &str, study: &str, series: &str) -> String {
    format!("{base}/studies/{study}/series/{series}")
}

pub(crate) fn instance_url(base: &str, study: &str, series: &str, instance: &str) -> String {
    format!("{base}/studies/{study}/series/{series}/instances/{instance}")
}

/// The query keys one level understands, as (keyword, tag).
const STUDY_KEYS: &[(&str, &str)] = &[
    ("StudyInstanceUID", "0020000D"),
    ("PatientID", "00100020"),
    ("AccessionNumber", "00080050"),
    ("StudyDate", "00080020"),
];
const SERIES_KEYS: &[(&str, &str)] = &[("SeriesInstanceUID", "0020000E"), ("Modality", "00080060")];
const INSTANCE_KEYS: &[(&str, &str)] =
    &[("SOPInstanceUID", "00080018"), ("SOPClassUID", "00080016")];

/// A parsed search query: matching keys by tag, and paging.
#[derive(Debug, Default)]
struct Search {
    matches: Vec<(&'static str, String)>,
    limit: Option<usize>,
    offset: usize,
}

impl Search {
    fn parse(
        params: &[(String, String)],
        keys: &[(&'static str, &'static str)],
    ) -> Result<Self, ApiError> {
        let mut search = Search::default();
        for (name, value) in params {
            match name.as_str() {
                "limit" => {
                    search.limit = Some(value.parse().map_err(|_| {
                        ApiError::bad_request(format!("limit is not a number: {value:?}"))
                    })?)
                }
                "offset" => {
                    search.offset = value.parse().map_err(|_| {
                        ApiError::bad_request(format!("offset is not a number: {value:?}"))
                    })?
                }
                // the computed attributes are always returned; others are not yet
                "includefield" => {}
                "fuzzymatching" if value == "false" => {}
                _ => {
                    let tag = keys
                        .iter()
                        .find(|(keyword, tag)| name == keyword || name.eq_ignore_ascii_case(tag))
                        .map(|(_, tag)| *tag)
                        .ok_or_else(|| {
                            ApiError::bad_request(format!("cannot search on {name} here yet"))
                        })?;
                    if value.contains(['*', '?', '\\', ','])
                        || (tag == "00080020" && value.contains('-'))
                    {
                        return Err(ApiError::bad_request(format!(
                            "only exact matching is supported yet: {name}={value}"
                        )));
                    }
                    search.matches.push((tag, value.clone()));
                }
            }
        }
        Ok(search)
    }

    fn get(&self, tag: &str) -> Option<&str> {
        self.matches
            .iter()
            .find(|(t, _)| *t == tag)
            .map(|(_, v)| v.as_str())
    }

    fn page<T>(&self, items: Vec<T>) -> Vec<T> {
        let items = items.into_iter().skip(self.offset);
        match self.limit {
            Some(limit) => items.take(limit).collect(),
            None => items.collect(),
        }
    }
}

fn check_accept(headers: &HeaderMap) -> Result<(), ApiError> {
    let accept = Accept::parse(headers.get(ACCEPT).and_then(|h| h.to_str().ok()));
    if accepts_json(&accept) {
        Ok(())
    } else {
        Err(ApiError::not_acceptable(
            "search results come as application/dicom+json",
        ))
    }
}

pub(crate) fn json_response(body: Value) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, DICOM_JSON)],
        body.to_string(),
    )
        .into_response()
}

fn put(attributes: &mut Value, tag: &str, element: Value) {
    if let Some(map) = attributes.as_object_mut() {
        map.insert(tag.to_owned(), element);
    }
}

fn uid(value: &str) -> Value {
    json!({"vr": "UI", "Value": [value]})
}

fn count(n: i64) -> Value {
    json!({"vr": "IS", "Value": [n]})
}

fn url(value: String) -> Value {
    json!({"vr": "UR", "Value": [value]})
}

pub(crate) async fn studies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, STUDY_KEYS)?;
    let filter = StudyFilter {
        study_instance_uid: search.get("0020000D").map(str::to_owned),
        patient_id: search.get("00100020").map(str::to_owned),
        accession_number: search.get("00080050").map(str::to_owned),
        study_date: search.get("00080020").map(str::to_owned),
        limit: search.limit.map(|l| l as i64),
        offset: Some(search.offset as i64),
    };
    let base = state.base_url(&headers);
    let studies = state.archive.index().studies(&filter).await?;
    let body: Vec<Value> = studies
        .into_iter()
        .map(|study| {
            let mut attributes = study.attributes;
            let modalities = if study.modalities.is_empty() {
                json!({"vr": "CS"})
            } else {
                json!({"vr": "CS", "Value": study.modalities})
            };
            put(&mut attributes, "00080061", modalities);
            put(&mut attributes, "00201206", count(study.series_count));
            put(&mut attributes, "00201208", count(study.instance_count));
            put(
                &mut attributes,
                "00081190",
                url(study_url(&base, &study.study_instance_uid)),
            );
            attributes
        })
        .collect();
    Ok(json_response(Value::Array(body)))
}

pub(crate) async fn series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study): Path<String>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, SERIES_KEYS)?;
    let base = state.base_url(&headers);
    let mut series = state
        .archive
        .index()
        .series(&study, search.get("0020000E"))
        .await?;
    if let Some(modality) = search.get("00080060") {
        series.retain(|s| s.attributes["00080060"]["Value"][0] == modality);
    }
    let body: Vec<Value> = search
        .page(series)
        .into_iter()
        .map(|series| {
            let mut attributes = series.attributes;
            put(&mut attributes, "0020000D", uid(&series.study_instance_uid));
            put(&mut attributes, "00201209", count(series.instance_count));
            put(
                &mut attributes,
                "00081190",
                url(series_url(
                    &base,
                    &series.study_instance_uid,
                    &series.series_instance_uid,
                )),
            );
            attributes
        })
        .collect();
    Ok(json_response(Value::Array(body)))
}

pub(crate) async fn instances(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series)): Path<(String, String)>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, INSTANCE_KEYS)?;
    let base = state.base_url(&headers);
    let mut instances = state
        .archive
        .index()
        .instances(&study, &series, search.get("00080018"))
        .await?;
    if let Some(class) = search.get("00080016") {
        instances.retain(|i| i.sop_class_uid == class);
    }
    let body: Vec<Value> = search
        .page(instances)
        .into_iter()
        .map(|instance| {
            let mut attributes = instance.attributes;
            put(
                &mut attributes,
                "0020000D",
                uid(&instance.study_instance_uid),
            );
            put(
                &mut attributes,
                "0020000E",
                uid(&instance.series_instance_uid),
            );
            put(
                &mut attributes,
                "00081190",
                url(instance_url(
                    &base,
                    &instance.study_instance_uid,
                    &instance.series_instance_uid,
                    &instance.sop_instance_uid,
                )),
            );
            attributes
        })
        .collect();
    Ok(json_response(Value::Array(body)))
}
