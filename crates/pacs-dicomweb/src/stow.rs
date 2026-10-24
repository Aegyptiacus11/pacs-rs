//! STOW-RS (PS3.18 10.5): `multipart/related; type="application/dicom"` bodies of DICOM
//! files.

use axum::extract::{Path, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_archive::{IngestError, Ingested};
use serde_json::{Value, json};

use crate::media::MediaType;
use crate::qido::{DICOM_JSON, instance_url, study_url};
use crate::{ApiError, AppState, multipart};

/// Failure Reason (0008,1197) values, from the C-STORE statuses of PS3.4 B.2.3 and the
/// general Processing Failure of PS3.7 C.4.
mod reason {
    /// 0110H Processing failure
    pub const PROCESSING_FAILURE: u16 = 0x0110;
    /// A900H Error: Data Set does not match SOP Class (here: not in the target study)
    pub const DOES_NOT_MATCH: u16 = 0xA900;
    /// C000H Error: Cannot understand
    pub const CANNOT_UNDERSTAND: u16 = 0xC000;
}

/// Tags of the Store Instances Response Module, as DICOM JSON keys.
mod tags {
    pub const RETRIEVE_URL: &str = "00081190";
    pub const FAILURE_REASON: &str = "00081197";
    pub const FAILED_SOP_SEQUENCE: &str = "00081198";
    pub const REFERENCED_SOP_SEQUENCE: &str = "00081199";
    pub const REFERENCED_SOP_CLASS_UID: &str = "00081150";
    pub const REFERENCED_SOP_INSTANCE_UID: &str = "00081155";
}

struct Failure {
    sop_class_uid: Option<String>,
    sop_instance_uid: Option<String>,
    reason: u16,
}

fn failure_of(e: &IngestError, bytes: &Bytes) -> Failure {
    // the UIDs, when the file can be read at all
    let key = pacs_core::Instance::from_bytes(bytes).ok().map(|i| i.key);
    let reason = match e {
        IngestError::Invalid(_) => reason::CANNOT_UNDERSTAND,
        IngestError::WrongStudy { .. } => reason::DOES_NOT_MATCH,
        IngestError::Conflict(_) | IngestError::Store(_) | IngestError::Index(_) => {
            reason::PROCESSING_FAILURE
        }
    };
    Failure {
        sop_class_uid: key.as_ref().map(|k| k.sop_class_uid.clone()),
        sop_instance_uid: key.map(|k| k.sop_instance_uid),
        reason,
    }
}

fn ui(value: &str) -> Value {
    json!({"vr": "UI", "Value": [value]})
}

async fn store_parts(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
    study: Option<String>,
) -> Result<Response, ApiError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .and_then(MediaType::parse)
        .ok_or_else(|| unsupported("a STOW-RS request needs a Content-Type"))?;
    if content_type.essence != "multipart/related" {
        return Err(unsupported(format!(
            "expected multipart/related, got {}",
            content_type.essence
        )));
    }
    if let Some(t) = content_type.param("type")
        && !t.eq_ignore_ascii_case("application/dicom")
    {
        return Err(unsupported(format!(
            "only application/dicom parts are stored yet, not {t}"
        )));
    }
    let boundary = content_type
        .param("boundary")
        .ok_or_else(|| ApiError::bad_request("multipart/related without a boundary"))?;
    let parts = multipart::parse(boundary, &body).map_err(|e| ApiError::bad_request(e.0))?;
    if parts.is_empty() {
        return Err(ApiError::bad_request("no parts"));
    }

    let base = state.base_url(&headers);
    let mut stored: Vec<Ingested> = Vec::new();
    let mut failed: Vec<Failure> = Vec::new();
    for part in parts {
        let is_dicom = part
            .content_type
            .as_deref()
            .is_none_or(|t| MediaType::parse(t).is_some_and(|m| m.essence == "application/dicom"));
        if !is_dicom {
            failed.push(Failure {
                sop_class_uid: None,
                sop_instance_uid: None,
                reason: reason::CANNOT_UNDERSTAND,
            });
            continue;
        }
        let result = match &study {
            Some(study) => {
                state
                    .archive
                    .ingest_in_study(part.content.clone(), study)
                    .await
            }
            None => state.archive.ingest(part.content.clone()).await,
        };
        match result {
            Ok(ingested) => stored.push(ingested),
            Err(e) => failed.push(failure_of(&e, &part.content)),
        }
    }

    let mut response = serde_json::Map::new();
    let study_for_url = study.clone().or_else(|| {
        let first = stored.first()?.key.study_instance_uid.clone();
        stored
            .iter()
            .all(|s| s.key.study_instance_uid == first)
            .then_some(first)
    });
    if let Some(study) = study_for_url {
        response.insert(
            tags::RETRIEVE_URL.into(),
            json!({"vr": "UR", "Value": [study_url(&base, &study)]}),
        );
    }
    if !failed.is_empty() {
        let items: Vec<Value> = failed
            .iter()
            .map(|f| {
                let mut item = serde_json::Map::new();
                if let Some(class) = &f.sop_class_uid {
                    item.insert(tags::REFERENCED_SOP_CLASS_UID.into(), ui(class));
                }
                if let Some(sop) = &f.sop_instance_uid {
                    item.insert(tags::REFERENCED_SOP_INSTANCE_UID.into(), ui(sop));
                }
                item.insert(
                    tags::FAILURE_REASON.into(),
                    json!({"vr": "US", "Value": [f.reason]}),
                );
                Value::Object(item)
            })
            .collect();
        response.insert(
            tags::FAILED_SOP_SEQUENCE.into(),
            json!({"vr": "SQ", "Value": items}),
        );
    }
    if !stored.is_empty() {
        let items: Vec<Value> = stored
            .iter()
            .map(|s| {
                let k = &s.key;
                json!({
                    tags::REFERENCED_SOP_CLASS_UID: ui(&k.sop_class_uid),
                    tags::REFERENCED_SOP_INSTANCE_UID: ui(&k.sop_instance_uid),
                    tags::RETRIEVE_URL: {"vr": "UR", "Value": [instance_url(
                        &base,
                        &k.study_instance_uid,
                        &k.series_instance_uid,
                        &k.sop_instance_uid,
                    )]},
                })
            })
            .collect();
        response.insert(
            tags::REFERENCED_SOP_SEQUENCE.into(),
            json!({"vr": "SQ", "Value": items}),
        );
    }

    // PS3.18 Table 10.5.3-1
    let status = match (stored.is_empty(), failed.is_empty()) {
        (false, true) => StatusCode::OK,
        (false, false) => StatusCode::ACCEPTED,
        (true, _) => StatusCode::CONFLICT,
    };
    Ok((
        status,
        [(CONTENT_TYPE, DICOM_JSON)],
        Value::Object(response).to_string(),
    )
        .into_response())
}

fn unsupported(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, message)
}

pub(crate) async fn store(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    store_parts(state, headers, body, None).await
}

pub(crate) async fn store_in_study(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    store_parts(state, headers, body, Some(study)).await
}
