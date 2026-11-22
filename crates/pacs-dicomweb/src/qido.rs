//! QIDO-RS: search for studies, a study's series, a series' instances (PS3.18 10.6).
//!
//! Matching is that of PS3.4 C.2.2.2 (`pacs_index::matching`): single value, wild card,
//! list of UIDs and range, on the required keys of PS3.18 Table 10.6.1-5 that the index
//! keeps. A key it does not know answers 400 rather than quietly matching something else. Every answer
//! carries the computed attributes (counts, Modalities in Study, Retrieve URL), so
//! `includefield` for them is satisfied without asking.

use axum::extract::{Path, Query};
use axum::http::HeaderMap;
use axum::http::header::ACCEPT;
use axum::response::{IntoResponse, Response};
use pacs_index::matching::ItemKey;
use pacs_index::{Key, Kind, Match, StudySearch};
use serde_json::{Value, json};

use crate::media::{Accept, accepts_json};
use crate::schema::{Attributes, Problem};
use crate::scope::Seen;
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

/// The matching keys one level understands, as (keyword, tag, how its values match): the
/// required ones of PS3.18 Table 10.6.1-5 that the index keeps, and a few more it keeps.
const STUDY_KEYS: &[(&str, &str, Kind)] = &[
    ("StudyDate", "00080020", Kind::Date),
    ("StudyTime", "00080030", Kind::Time),
    ("AccessionNumber", "00080050", Kind::Text),
    ("ModalitiesInStudy", "00080061", Kind::Text),
    ("ReferringPhysicianName", "00080090", Kind::Name),
    ("PatientName", "00100010", Kind::Name),
    ("PatientID", "00100020", Kind::Text),
    ("StudyInstanceUID", "0020000D", Kind::Uid),
    ("StudyID", "00200010", Kind::Text),
    ("StudyDescription", "00081030", Kind::Text),
    ("PatientBirthDate", "00100030", Kind::Date),
    ("PatientSex", "00100040", Kind::Text),
];
const SERIES_KEYS: &[(&str, &str, Kind)] = &[
    ("Modality", "00080060", Kind::Text),
    ("SeriesInstanceUID", "0020000E", Kind::Uid),
    ("SeriesNumber", "00200011", Kind::Number),
    ("PerformedProcedureStepStartDate", "00400244", Kind::Date),
    ("PerformedProcedureStepStartTime", "00400245", Kind::Time),
    ("SeriesDescription", "0008103E", Kind::Text),
];
/// The sequences one level can be searched by, each with the keys of its item it
/// understands: a query parameter names one as `Sequence.Attribute` (PS3.18 8.3.4.1), by
/// keyword or tag. Request Attributes Sequence is a required key of the series
/// (Table 10.6.1-5).
type Sequences = &'static [(
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str, Kind)],
)];
const SERIES_SEQUENCES: Sequences = &[(
    "RequestAttributesSequence",
    "00400275",
    &[
        ("ScheduledProcedureStepID", "00400009", Kind::Text),
        ("RequestedProcedureID", "00401001", Kind::Text),
    ],
)];
const INSTANCE_KEYS: &[(&str, &str, Kind)] = &[
    ("SOPClassUID", "00080016", Kind::Uid),
    ("SOPInstanceUID", "00080018", Kind::Uid),
    ("InstanceNumber", "00200013", Kind::Number),
];

/// The kinds of matching a search may ask for by a parameter of their own, none of which
/// is done here: asked for, the search is made without, and the answer says so
/// (PS3.18 8.3.4.2, 8.3.4.5, 8.3.4.6).
const NOT_DONE: &[(&str, &str)] = &[
    (
        "fuzzymatching",
        "The fuzzymatching parameter is not supported. Only literal matching has been performed.",
    ),
    (
        "emptyvaluematching",
        "The emptyvaluematching parameter is not supported. Empty Value Matching has not been performed.",
    ),
    (
        "multiplevaluematching",
        "The multiplevaluematching parameter is not supported. Multiple Value Matching has not been performed.",
    ),
];

/// A parsed search query: its matching keys, its page, and what it asked for that is not
/// done.
#[derive(Debug, Default)]
struct Search {
    keys: Vec<Key>,
    limit: Option<usize>,
    offset: usize,
    not_done: Vec<&'static str>,
}

impl Search {
    fn parse(
        params: &[(String, String)],
        keys: &[(&'static str, &'static str, Kind)],
        sequences: Sequences,
    ) -> Result<Self, ApiError> {
        let mut search = Search::default();
        // every key named, as tags joined by `.`, to refuse one given twice
        let mut named: Vec<String> = Vec::new();
        let mut once = |path: String, name: &str| {
            if named.contains(&path) {
                return Err(ApiError::bad_request(format!("{name} is given twice")));
            }
            named.push(path);
            Ok(())
        };
        let cannot =
            |name: &str| ApiError::bad_request(format!("cannot search on {name} here yet"));
        let named_by = |name: &str, keyword: &str, tag: &str| {
            name == keyword || name.eq_ignore_ascii_case(tag)
        };
        for (name, value) in params {
            if let Some((_, said)) = NOT_DONE.iter().find(|(parameter, _)| name == parameter) {
                match value.as_str() {
                    "false" => {}
                    "true" => search.not_done.push(said),
                    _ => {
                        return Err(ApiError::bad_request(format!(
                            "{name} is true or false, not {value:?}"
                        )));
                    }
                }
                continue;
            }
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
                // a key of a sequence's item (C.2.2.2.6): every key of one sequence is
                // asked of the same item
                _ if name.contains('.') => {
                    let (outer, inner) = name.split_once('.').unwrap_or_default();
                    let (_, sequence, items) = sequences
                        .iter()
                        .find(|(keyword, tag, _)| named_by(outer, keyword, tag))
                        .ok_or_else(|| cannot(name))?;
                    let (_, tag, kind) = items
                        .iter()
                        .find(|(keyword, tag, _)| named_by(inner, keyword, tag))
                        .ok_or_else(|| cannot(name))?;
                    once(format!("{sequence}.{tag}"), name)?;
                    let Some(asked) = Match::parse(*kind, value)
                        .map_err(|e| ApiError::bad_request(format!("{name}: {e}")))?
                    else {
                        continue;
                    };
                    let key = ItemKey {
                        tag: (*tag).to_owned(),
                        kind: *kind,
                        asked,
                    };
                    match search.keys.iter_mut().find(|k| k.tag == *sequence) {
                        Some(Key {
                            asked: Match::Item(item),
                            ..
                        }) => item.push(key),
                        _ => search.keys.push(Key {
                            tag: sequence,
                            kind: Kind::Sequence,
                            asked: Match::Item(vec![key]),
                        }),
                    }
                }
                _ if sequences
                    .iter()
                    .any(|(keyword, tag, _)| named_by(name, keyword, tag)) =>
                {
                    // the sequence alone, empty, asks for nothing
                    if !value.trim().is_empty() {
                        return Err(ApiError::bad_request(format!(
                            "{name} is matched by the keys of its item, as {name}.<attribute>=<value>, not by a value"
                        )));
                    }
                }
                _ => {
                    let (_, tag, kind) = keys
                        .iter()
                        .find(|(keyword, tag, _)| named_by(name, keyword, tag))
                        .ok_or_else(|| cannot(name))?;
                    // "Each attribute in the Query Parameter shall be not be repeated"
                    once((*tag).to_owned(), name)?;
                    // an empty value matches everything: nothing to ask
                    if let Some(asked) = Match::parse(*kind, value)
                        .map_err(|e| ApiError::bad_request(format!("{name}: {e}")))?
                    {
                        search.keys.push(Key {
                            tag,
                            kind: *kind,
                            asked,
                        });
                    }
                }
            }
        }
        Ok(search)
    }

    /// A range of times that ends before it starts matches nothing at these levels,
    /// where no date comes with it: refused, so that it is not taken for an empty result.
    fn no_reversed_range(&self) -> Result<(), ApiError> {
        match self.keys.iter().find(|key| key.asked.reversed()) {
            Some(_) => Err(ApiError::bad_request(
                "the range of times ends before it starts: a range does not cross midnight",
            )),
            None => Ok(()),
        }
    }

    /// Whether a series' or an instance's attributes answer every key.
    fn answered_by(&self, attributes: &Value) -> bool {
        self.keys
            .iter()
            .all(|key| key.asked.answered_by(key.kind, attributes, key.tag))
    }

    fn page<T>(&self, items: Vec<T>) -> Vec<T> {
        let items = items.into_iter().skip(self.offset);
        match self.limit {
            Some(limit) => items.take(limit).collect(),
            None => items.collect(),
        }
    }

    /// The answer, with a warning for each kind of matching asked for and not done.
    fn response(&self, base: &str, body: Value) -> Response {
        let mut response = json_response(body);
        for said in &self.not_done {
            if let Ok(warning) = format!("299 {base}: {said}").parse() {
                response
                    .headers_mut()
                    .append(axum::http::header::WARNING, warning);
            }
        }
        response
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

/// `lower` laid over `upper`: a series result is its study's attributes plus its own,
/// an instance result its study's, its series' and its own, as the reference server
/// answers. The lower level wins where both have a tag.
fn inherit(upper: &Value, lower: Value) -> Value {
    let mut merged = upper.clone();
    if let (Some(m), Value::Object(l)) = (merged.as_object_mut(), lower) {
        m.extend(l);
    }
    merged
}

/// Search results are JSON, so their strings are UTF-8 whatever the files say.
fn utf8(attributes: &mut Value) {
    put(
        attributes,
        "00080005",
        json!({"vr": "CS", "Value": ["ISO_IR 192"]}),
    );
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

/// A study's stored attributes, `{}` for a study not indexed.
async fn study_attributes(state: &AppState, study: &str) -> Result<Value, ApiError> {
    Ok(state
        .archive
        .index()
        .study(study)
        .await?
        .map_or_else(|| json!({}), |s| s.attributes))
}

/// Searches for studies (QIDO-RS).
///
/// Each study with its attributes, its counts of series and instances, the modalities of its series and the address to retrieve it from.
#[utoipa::path(
    get,
    path = "/studies",
    tag = "DICOMweb",
    params(("{attribute}" = Option<String>, Query, description = "A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches."), ("limit" = Option<u64>, Query, description = "At most this many."), ("offset" = Option<u64>, Query, description = "Skip this many first.")),
    responses(
        (status = 200, description = "The studies, newest first.", body = Vec<Attributes>, content_type = "application/dicom+json"),
        (status = 400, description = "A key this level does not have, one given twice, or a value that is not a match for its key.", body = Problem),
        (status = 406, description = "`Accept` asks for something else than this answers in.", body = Problem)
    )
)]
pub(crate) async fn studies(
    Seen(state): Seen,
    headers: HeaderMap,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, STUDY_KEYS, &[])?;
    let base = state.base_url(&headers);
    let asked = StudySearch {
        keys: search.keys.clone(),
        // a range of dates with a range of times is one range of moments here
        // (PS3.18 8.3.4.1.1)
        combined_date_time: true,
        limit: search.limit,
        offset: search.offset,
    };
    asked.check().map_err(ApiError::bad_request)?;
    let studies = state.archive.index().search_studies(&asked).await?;
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
            utf8(&mut attributes);
            attributes
        })
        .collect();
    Ok(search.response(&base, Value::Array(body)))
}

/// Lists and searches a study's series (QIDO-RS).
#[utoipa::path(
    get,
    path = "/studies/{study}/series",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID."), ("{attribute}" = Option<String>, Query, description = "A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches."), ("limit" = Option<u64>, Query, description = "At most this many."), ("offset" = Option<u64>, Query, description = "Skip this many first.")),
    responses(
        (status = 200, description = "The series, by Series Number; none for a study that is not there.", body = Vec<Attributes>, content_type = "application/dicom+json"),
        (status = 400, description = "A key this level does not have, one given twice, or a value that is not a match for its key.", body = Problem),
        (status = 406, description = "`Accept` asks for something else than this answers in.", body = Problem)
    )
)]
pub(crate) async fn series(
    Seen(state): Seen,
    headers: HeaderMap,
    Path(study): Path<String>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, SERIES_KEYS, SERIES_SEQUENCES)?;
    search.no_reversed_range()?;
    let base = state.base_url(&headers);
    let mut series = state.archive.index().series(&study, None).await?;
    series.retain(|s| search.answered_by(&s.attributes));
    let study_attributes = study_attributes(&state, &study).await?;
    let body: Vec<Value> = search
        .page(series)
        .into_iter()
        .map(|series| {
            let mut attributes = inherit(&study_attributes, series.attributes);
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
            utf8(&mut attributes);
            attributes
        })
        .collect();
    Ok(search.response(&base, Value::Array(body)))
}

/// Lists and searches a series' instances (QIDO-RS).
#[utoipa::path(
    get,
    path = "/studies/{study}/series/{series}/instances",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID."), ("series" = String, Path, description = "Series Instance UID."), ("{attribute}" = Option<String>, Query, description = "A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches."), ("limit" = Option<u64>, Query, description = "At most this many."), ("offset" = Option<u64>, Query, description = "Skip this many first.")),
    responses(
        (status = 200, description = "The instances, by Instance Number; none for a series that is not there.", body = Vec<Attributes>, content_type = "application/dicom+json"),
        (status = 400, description = "A key this level does not have, one given twice, or a value that is not a match for its key.", body = Problem),
        (status = 406, description = "`Accept` asks for something else than this answers in.", body = Problem)
    )
)]
pub(crate) async fn instances(
    Seen(state): Seen,
    headers: HeaderMap,
    Path((study, series)): Path<(String, String)>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    check_accept(&headers)?;
    let search = Search::parse(&params, INSTANCE_KEYS, &[])?;
    search.no_reversed_range()?;
    let base = state.base_url(&headers);
    let mut instances = state
        .archive
        .index()
        .instances(&study, &series, None)
        .await?;
    instances.retain(|i| search.answered_by(&i.attributes));
    let upper = match state
        .archive
        .index()
        .series(&study, Some(&series))
        .await?
        .into_iter()
        .next()
    {
        Some(s) => inherit(&study_attributes(&state, &study).await?, s.attributes),
        None => json!({}),
    };
    let body: Vec<Value> = search
        .page(instances)
        .into_iter()
        .map(|instance| {
            let mut attributes = inherit(&upper, instance.attributes);
            // every indexed instance is in the store, ready to retrieve
            put(
                &mut attributes,
                "00080056",
                json!({"vr": "CS", "Value": ["ONLINE"]}),
            );
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
            utf8(&mut attributes);
            attributes
        })
        .collect();
    Ok(search.response(&base, Value::Array(body)))
}
