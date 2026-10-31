//! `GET /modalities/{id}/studies`: a remote AE's studies, by C-FIND.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use pacs_dimse::Config;
use pacs_rest::RestError;
use serde_json::Value;

/// Matches returned when no `limit` is given, and the most one may ask for.
const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1000;

/// The remote AE's studies, found by C-FIND at study level (Study Root), as a DICOM JSON
/// array like a QIDO-RS search: `PatientID`, `PatientName`, `AccessionNumber`, `StudyDate`
/// (a date or a range) and `ModalitiesInStudy`, with `*` and `?` where PS3.4 C.2.2.2.4 allows
/// them, and `limit`. With more matches than `limit` the search is cancelled (C-CANCEL) and
/// a `Warning` header says so; so it does when the archive did not match on every key or
/// ended early. 400 for a query that breaks the matching rules, 502 when the remote AE
/// cannot be reached or answers with a failure, 404 for an unknown id.
pub(super) async fn studies(
    State(config): State<Arc<Config>>,
    Path(id): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let Some(remote) = config.remote(&id) else {
        return RestError::NotFound.into_response();
    };
    let (query, limit) = match study_query(params) {
        Ok(parsed) => parsed,
        Err(e) => return RestError::bad_request("Bad query", e).into_response(),
    };
    let mut found = Vec::new();
    let mut more = false;
    let result = pacs_dimse::find_studies(remote, &config.ae_title, &query, config.timeout, |m| {
        if found.len() == limit {
            more = true;
            return std::ops::ControlFlow::Break(());
        }
        found.push(pacs_dimse::query::to_json(&m));
        std::ops::ControlFlow::Continue(())
    })
    .await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(pacs_dimse::FindError::Query(e)) => {
            return RestError::bad_request("Bad query", e.to_string()).into_response();
        }
        Err(e) => {
            tracing::info!(remote = %id, error = %e, "C-FIND failed");
            return RestError::Status {
                status: StatusCode::BAD_GATEWAY,
                message: format!("C-FIND to {id} failed"),
                details: Some(e.to_string()),
            }
            .into_response();
        }
    };
    let mut warnings = Vec::new();
    if more {
        warnings.push(format!("more studies match than the limit of {limit}"));
    }
    if outcome.optional_keys_unsupported {
        warnings
            .push("the archive did not match on every key; pacs-rs filtered what it could".into());
    }
    if !more && outcome.status != pacs_dimse::Status::SUCCESS {
        warnings.push(format!(
            "the archive ended the search early ({})",
            outcome.status
        ));
    }
    let mut response = (
        [(header::CONTENT_TYPE, "application/dicom+json")],
        Json(Value::Array(found)),
    )
        .into_response();
    for warning in warnings {
        if let Ok(value) = HeaderValue::from_str(&format!("299 pacs-rs \"{warning}\"")) {
            response.headers_mut().append(header::WARNING, value);
        }
    }
    response
}

/// The query parameters as a study query and a limit; any other parameter is an error, not
/// silently ignored.
fn study_query(params: HashMap<String, String>) -> Result<(pacs_dimse::StudyQuery, usize), String> {
    let mut query = pacs_dimse::StudyQuery::default();
    let mut limit = DEFAULT_LIMIT;
    for (key, value) in params {
        match key.as_str() {
            "PatientID" => query.patient_id = Some(value),
            "PatientName" => query.patient_name = Some(value),
            "AccessionNumber" => query.accession_number = Some(value),
            "StudyDate" => {
                query.study_date = Some(
                    value
                        .parse()
                        .map_err(|e: pacs_dimse::QueryError| e.to_string())?,
                )
            }
            "ModalitiesInStudy" => query.modality = Some(value),
            "limit" => {
                limit = value
                    .parse()
                    .ok()
                    .filter(|l| (1..=MAX_LIMIT).contains(l))
                    .ok_or_else(|| format!("limit is a number from 1 to {MAX_LIMIT}"))?;
            }
            other => {
                return Err(format!(
                    "{other} is not a search key here; use PatientID, PatientName, AccessionNumber, StudyDate, ModalitiesInStudy or limit"
                ));
            }
        }
    }
    let query = query.checked().map_err(|e| e.to_string())?;
    Ok((query, limit))
}
