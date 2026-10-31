//! `/ui/api/studies`: the web interface's study list, a page at a time, filtered and sorted by
//! the index rather than in the browser. Query parameters, all optional:
//!
//! - `q`: words, each to be found in the patient's name or ID, the accession number, the
//!   study description, the referring physician or the date;
//! - `from`, `to`: study dates, `YYYYMMDD`, both included;
//! - `modality`, `sex`: comma-separated choices (`sex=O` for anything but F and M);
//! - `patient`: one patient's studies, by public ID;
//! - `sort`: `patient`, `description`, `modality`, `date` (the default), `series`, `images`
//!   or `received`; `order`: `asc` or `desc` (the default);
//! - `offset`, `limit` (at most 1000, 50 by default);
//! - `summary=true`: also the totals of the whole list and its five last received studies.
//!
//! The answer: `{"Total", "Studies": [...], "Summary"?}`, each study with its ID, its
//! patient's, its and its patient's main tags, modalities, series and image counts and when it
//! last received an image. `/ui/api/studies/facets[?patient=]` gives what the filters can
//! choose, with how many studies each has: `{"Modalities", "Sexes", "Dates"}`.
//!
//! `/ui/api/patients` lists the patients the same way: `q` (words in the name or Patient ID),
//! `sort` (`name`, `last`: the latest study, the default, or `studies`), `order`, `offset`,
//! `limit`; it answers `{"Total", "Patients": [...]}`, each with its ID, main tags, study and
//! image counts, modalities and first and last study dates.
//! Part of the web interface: off with it (`PACS_UI=false`).

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use pacs_index::{Index, PatientQuery, PatientSort, WorklistQuery, WorklistSort, WorklistStudy};
use serde::Deserialize;
use serde_json::{Map, Value, json};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 1000;

fn error(status: StatusCode, message: &str) -> Response {
    (status, axum::Json(json!({ "Message": message }))).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Params {
    q: String,
    from: Option<String>,
    to: Option<String>,
    modality: String,
    sex: String,
    patient: Option<String>,
    sort: Option<String>,
    order: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
    summary: bool,
}

fn list(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn date(text: Option<String>) -> Result<Option<String>, &'static str> {
    match text.filter(|d| !d.is_empty()) {
        Some(d) if d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()) => Ok(Some(d)),
        Some(_) => Err("Dates are YYYYMMDD"),
        None => Ok(None),
    }
}

/// The index's query from the URL's, or why it can't be one.
fn query(p: Params) -> Result<(WorklistQuery, bool), &'static str> {
    let sort = match p.sort.as_deref().unwrap_or("date") {
        "patient" => WorklistSort::Patient,
        "description" => WorklistSort::Description,
        "modality" => WorklistSort::Modality,
        "date" => WorklistSort::Date,
        "series" => WorklistSort::Series,
        "images" => WorklistSort::Instances,
        "received" => WorklistSort::Received,
        _ => {
            return Err("sort is patient, description, modality, date, series, images or received");
        }
    };
    let descending = match p.order.as_deref().unwrap_or("desc") {
        "desc" => true,
        "asc" => false,
        _ => return Err("order is asc or desc"),
    };
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) || p.offset.is_some_and(|o| o < 0) {
        return Err("limit is 1 to 1000, offset 0 or more");
    }
    Ok((
        WorklistQuery {
            words: p.q.split_whitespace().map(str::to_owned).collect(),
            from: date(p.from)?,
            to: date(p.to)?,
            modalities: list(&p.modality),
            sexes: list(&p.sex),
            patient: p.patient.filter(|s| !s.is_empty()),
            sort,
            descending,
            offset: p.offset.unwrap_or(0),
            limit,
        },
        p.summary,
    ))
}

fn study(s: WorklistStudy) -> Value {
    json!({
        "ID": s.public_id,
        "ParentPatient": s.patient_public_id,
        "MainDicomTags": s.main_tags,
        "PatientMainDicomTags": s.patient_main_tags,
        "Modalities": s.modalities,
        "Series": s.series,
        "Instances": s.instances,
        "LastReceived": s.last_ms,
    })
}

fn counts(list: Vec<(String, i64)>) -> Value {
    Value::Object(
        list.into_iter()
            .map(|(k, n)| (k, json!(n)))
            .collect::<Map<_, _>>(),
    )
}

fn failed(what: &str, e: pacs_index::Error) -> Response {
    tracing::error!("the study list: {what}: {e}");
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "The study list could not be read",
    )
}

async fn studies(State(index): State<Index>, Query(params): Query<Params>) -> Response {
    let (q, with_summary) = match query(params) {
        Ok(q) => q,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let (page, total) = match index.worklist(&q).await {
        Ok(found) => found,
        Err(e) => return failed("a page", e),
    };
    let mut body =
        json!({ "Total": total, "Studies": page.into_iter().map(study).collect::<Vec<_>>() });
    if with_summary {
        let summary = match index.worklist_summary(&q).await {
            Ok(s) => s,
            Err(e) => return failed("its summary", e),
        };
        let recent = WorklistQuery {
            sort: WorklistSort::Received,
            descending: true,
            offset: 0,
            limit: 5,
            ..q
        };
        let recent = match index.worklist(&recent).await {
            Ok((r, _)) => r,
            Err(e) => return failed("its latest studies", e),
        };
        body["Summary"] = json!({
            "Patients": summary.patients,
            "Studies": summary.studies,
            "Series": summary.series,
            "Instances": summary.instances,
            "Modalities": summary.by_modality.into_iter().map(|(m, n)| json!([m, n])).collect::<Vec<_>>(),
            "Recent": recent.into_iter().map(study).collect::<Vec<_>>(),
        });
    }
    ([(header::CACHE_CONTROL, "no-store")], axum::Json(body)).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FacetParams {
    patient: Option<String>,
}

async fn facets(State(index): State<Index>, Query(p): Query<FacetParams>) -> Response {
    match index
        .worklist_facets(p.patient.as_deref().filter(|s| !s.is_empty()))
        .await
    {
        Ok(f) => (
            [(header::CACHE_CONTROL, "no-store")],
            axum::Json(json!({
                "Modalities": counts(f.modalities),
                "Sexes": counts(f.sexes),
                "Dates": counts(f.dates),
            })),
        )
            .into_response(),
        Err(e) => failed("its filters", e),
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PatientParams {
    q: String,
    sort: Option<String>,
    order: Option<String>,
    offset: Option<i64>,
    limit: Option<i64>,
}

fn patient_query(p: PatientParams) -> Result<PatientQuery, &'static str> {
    let sort = match p.sort.as_deref().unwrap_or("last") {
        "name" => PatientSort::Name,
        "last" => PatientSort::LastStudy,
        "studies" => PatientSort::Studies,
        _ => return Err("sort is name, last or studies"),
    };
    let descending = match p.order.as_deref().unwrap_or("desc") {
        "desc" => true,
        "asc" => false,
        _ => return Err("order is asc or desc"),
    };
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) || p.offset.is_some_and(|o| o < 0) {
        return Err("limit is 1 to 1000, offset 0 or more");
    }
    Ok(PatientQuery {
        words: p.q.split_whitespace().map(str::to_owned).collect(),
        sort,
        descending,
        offset: p.offset.unwrap_or(0),
        limit,
    })
}

async fn patients(State(index): State<Index>, Query(params): Query<PatientParams>) -> Response {
    let q = match patient_query(params) {
        Ok(q) => q,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    match index.worklist_patients(&q).await {
        Ok((list, total)) => {
            let list: Vec<Value> = list
                .into_iter()
                .map(|p| {
                    json!({
                        "ID": p.public_id,
                        "MainDicomTags": p.main_tags,
                        "Studies": p.studies,
                        "Instances": p.instances,
                        "Modalities": p.modalities,
                        "FirstStudy": p.first_study,
                        "LastStudy": p.last_study,
                    })
                })
                .collect();
            (
                [(header::CACHE_CONTROL, "no-store")],
                axum::Json(json!({ "Total": total, "Patients": list })),
            )
                .into_response()
        }
        Err(e) => failed("the patients", e),
    }
}

pub fn router(index: Index) -> Router {
    Router::new()
        .route("/ui/api/patients", get(patients))
        .route("/ui/api/studies", get(studies))
        .route("/ui/api/studies/facets", get(facets))
        .with_state(index)
}
