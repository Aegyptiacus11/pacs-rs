//! STOW-RS (PS3.18 10.5): `multipart/related; type="application/dicom"` bodies of DICOM
//! files, and `type="application/dicom+json"` bodies of metadata with their bulk data
//! (10.5.1.2.2): a first part of DICOM JSON, an array of one data set an instance, and parts
//! whose Content-Location the BulkDataURIs in it name.

use axum::extract::Path;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_archive::{IngestError, Ingested};
use serde_json::{Value, json};

use crate::media::MediaType;
use crate::qido::{DICOM_JSON, instance_url, study_url};
use crate::schema::{Attributes, Binary, Problem};
use crate::scope::Seen;
use crate::{ApiError, AppState, multipart};

/// Failure Reason (0008,1197) values, from the C-STORE statuses of PS3.4 B.2.3 and the
/// general Processing Failure of PS3.7 C.4.
mod reason {
    /// 0110H Processing failure
    pub const PROCESSING_FAILURE: u16 = 0x0110;
    /// A900H Error: Data Set does not match SOP Class (here: not in the target study)
    pub const OUT_OF_RESOURCES: u16 = 0xA700;
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
    // the UIDs, when the file can be read at all and at a cost the archive accepts
    let key = match e {
        IngestError::TooCostly { .. } => None,
        _ => pacs_core::Instance::from_bytes(bytes).ok().map(|i| i.key),
    };
    let reason = match e {
        IngestError::Invalid(_) => reason::CANNOT_UNDERSTAND,
        IngestError::WrongStudy { .. } => reason::DOES_NOT_MATCH,
        IngestError::TooCostly { .. } => reason::OUT_OF_RESOURCES,
        IngestError::Conflict(_)
        | IngestError::Limited
        | IngestError::Store(_)
        | IngestError::Index(_) => reason::PROCESSING_FAILURE,
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
    // nothing is stored for an account limited to projects: it would be in no project
    if state.archive.index().account().is_some() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            pacs_archive::IngestError::Limited.to_string(),
        ));
    }
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
    let metadata = match content_type
        .param("type")
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("application/dicom") => false,
        Some("application/dicom+json") => true,
        Some("application/dicom+xml") => {
            return Err(unsupported(
                "metadata in DICOM XML is not taken: send it as application/dicom+json, or the files as application/dicom",
            ));
        }
        Some(t) => {
            return Err(unsupported(format!(
                "the parts are application/dicom, or application/dicom+json with bulk data, not {t}"
            )));
        }
    };
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
    // with metadata, each instance is made a file first, then stored as one sent whole
    let parts = if metadata {
        let mut files = Vec::new();
        for made in from_metadata(&parts)? {
            match made {
                Ok(file) => files.push(multipart::Part {
                    content_type: None,
                    content_location: None,
                    content: file,
                }),
                Err(failure) => failed.push(failure),
            }
        }
        files
    } else {
        parts
    };
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

/// Stores instances (STOW-RS).
///
/// Several files in one request. An instance already stored is kept as it was and counted as stored.
#[utoipa::path(
    post,
    path = "/studies",
    tag = "DICOMweb",
    request_body(content = Binary, content_type = "multipart/related", description = "One part an instance, each `application/dicom`: `multipart/related; type=\"application/dicom\"`."),
    responses(
        (status = 200, description = "Every instance was stored. The answer names each one (PS3.18 Table 10.5.3-1).", body = Attributes, content_type = "application/dicom+json"),
        (status = 202, description = "Some were stored and some were not; the answer names both, each failure with its reason.", body = Attributes, content_type = "application/dicom+json"),
        (status = 400, description = "The body is not `multipart/related` with parts.", body = Problem),
        (status = 403, description = "The account is limited to some projects: it stores nothing over HTTP.", body = Problem),
        (status = 409, description = "None was stored; the answer names each failure with its reason.", body = Attributes, content_type = "application/dicom+json"),
        (status = 415, description = "The parts are not `application/dicom`.", body = Problem)
    )
)]
pub(crate) async fn store(
    Seen(state): Seen,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    store_parts(state, headers, body, None).await
}

/// Stores instances into one study (STOW-RS).
///
/// As above, and an instance of another study is refused, the others being stored.
#[utoipa::path(
    post,
    path = "/studies/{study}",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID.")),
    request_body(content = Binary, content_type = "multipart/related", description = "One part an instance, each `application/dicom`: `multipart/related; type=\"application/dicom\"`."),
    responses(
        (status = 200, description = "Every instance was stored. The answer names each one (PS3.18 Table 10.5.3-1).", body = Attributes, content_type = "application/dicom+json"),
        (status = 202, description = "Some were stored and some were not; the answer names both, each failure with its reason.", body = Attributes, content_type = "application/dicom+json"),
        (status = 400, description = "The body is not `multipart/related` with parts.", body = Problem),
        (status = 403, description = "The account is limited to some projects: it stores nothing over HTTP.", body = Problem),
        (status = 409, description = "None was stored; the answer names each failure with its reason.", body = Attributes, content_type = "application/dicom+json"),
        (status = 415, description = "The parts are not `application/dicom`.", body = Problem)
    )
)]
pub(crate) async fn store_in_study(
    Seen(state): Seen,
    headers: HeaderMap,
    Path(study): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    store_parts(state, headers, body, Some(study)).await
}

/// The transfer syntax pixel data of `media_type` is in: its `transfer-syntax` parameter,
/// else the one PS3.18 Table 8.7.3-5 gives the type by default.
fn pixel_transfer_syntax(media_type: &MediaType) -> Option<String> {
    if let Some(ts) = media_type.param("transfer-syntax") {
        return Some(ts.to_owned());
    }
    Some(
        match media_type.essence.as_str() {
            "application/octet-stream" => pacs_core::codec::EXPLICIT_VR_LE,
            "image/jls" => "1.2.840.10008.1.2.4.80",
            "image/jpeg" => "1.2.840.10008.1.2.4.50",
            "image/jp2" => "1.2.840.10008.1.2.4.91",
            "image/jxl" => "1.2.840.10008.1.2.4.112",
            _ => return None,
        }
        .to_owned(),
    )
}

/// A part a BulkDataURI names: by its Content-Location, the same or the one ending the
/// other (a client may name parts by a path where it wrote an absolute URI, or the other
/// way around).
fn part_named<'a>(parts: &'a [multipart::Part], uri: &str) -> Option<&'a multipart::Part> {
    parts.iter().find(|part| {
        part.content_location
            .as_deref()
            .is_some_and(|at| at == uri || at.ends_with(uri) || uri.ends_with(at))
    })
}

/// Each instance of a metadata request made a Part 10 file, or why it could not be.
fn from_metadata(parts: &[multipart::Part]) -> Result<Vec<Result<Bytes, Failure>>, ApiError> {
    let first = &parts[0];
    let is_json = first
        .content_type
        .as_deref()
        .and_then(MediaType::parse)
        .is_none_or(|m| {
            matches!(
                m.essence.as_str(),
                "application/dicom+json" | "application/json"
            )
        });
    if !is_json {
        return Err(unsupported(
            "the first part of a metadata request is its DICOM JSON",
        ));
    }
    let data_sets: Vec<Value> = serde_json::from_slice(&first.content)
        .map_err(|e| ApiError::bad_request(format!("the metadata is not DICOM JSON: {e}")))?;
    Ok(data_sets
        .into_iter()
        .map(|data_set| instance_file(data_set, &parts[1..]))
        .collect())
}

/// One data set of DICOM JSON and the parts its BulkDataURIs name, as a Part 10 file: bulk
/// values put in place, the pixel data native or, from a compressed part, encapsulated in
/// one fragment, the transfer syntax the pixel data's.
fn instance_file(mut data_set: Value, parts: &[multipart::Part]) -> Result<Bytes, Failure> {
    use base64::Engine;
    let uid = |tag: &str, data_set: &Value| {
        pacs_index::matching::values(data_set, tag)
            .into_iter()
            .next()
    };
    // why is for the reader of this code: the answer has only a Failure Reason
    let failure = |data_set: &Value, _why: &str| Failure {
        sop_class_uid: uid("00080016", data_set),
        sop_instance_uid: uid("00080018", data_set),
        reason: reason::CANNOT_UNDERSTAND,
    };
    let Some(object) = data_set.as_object_mut() else {
        return Err(failure(
            &Value::Null,
            "a data set that is not a JSON object",
        ));
    };
    // the pixel data, from its part
    let mut encapsulated = None;
    let mut transfer_syntax = pacs_core::codec::EXPLICIT_VR_LE.to_owned();
    if let Some(uri) = object
        .get("7FE00010")
        .and_then(|e| e.get("BulkDataURI"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        let snapshot = Value::Object(object.clone());
        let part = part_named(parts, &uri)
            .ok_or_else(|| failure(&snapshot, "no part for the pixel data's BulkDataURI"))?;
        let media_type = part
            .content_type
            .as_deref()
            .and_then(MediaType::parse)
            .unwrap_or(MediaType {
                essence: "application/octet-stream".into(),
                params: Vec::new(),
            });
        transfer_syntax = pixel_transfer_syntax(&media_type)
            .ok_or_else(|| failure(&snapshot, "pixel data of a media type not taken"))?;
        if pacs_core::codec::is_native(&transfer_syntax) {
            let vr = object["7FE00010"]["vr"].as_str().unwrap_or("OW").to_owned();
            object.insert(
                "7FE00010".into(),
                json!({"vr": vr, "InlineBinary": base64::engine::general_purpose::STANDARD.encode(&part.content)}),
            );
        } else {
            let frames = uid("00280008", &snapshot)
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(1);
            if frames > 1 {
                return Err(failure(
                    &snapshot,
                    "compressed pixel data of several frames in one part",
                ));
            }
            object.remove("7FE00010");
            encapsulated = Some(part.content.to_vec());
        }
    }
    // every other bulk value, inline
    let mut missing = None;
    inline_bulk(object, parts, &mut missing);
    if let Some(uri) = missing {
        return Err(failure(
            &Value::Object(object.clone()),
            &format!("no part for the BulkDataURI {uri}"),
        ));
    }
    let snapshot = Value::Object(object.clone());
    let mut dicom: dicom_object::InMemDicomObject =
        dicom_json::from_value(Value::Object(object.clone()))
            .map_err(|_| failure(&snapshot, "the data set is not DICOM JSON"))?;
    if let Some(fragment) = encapsulated {
        let mut fragment = fragment;
        if fragment.len() % 2 == 1 {
            fragment.push(0);
        }
        dicom.put(dicom_core::DataElement::new(
            dicom_dictionary_std::tags::PIXEL_DATA,
            dicom_core::VR::OB,
            dicom_core::value::PixelFragmentSequence::new(Vec::<u32>::new(), vec![fragment]),
        ));
    }
    let (Some(class), Some(instance)) = (uid("00080016", &snapshot), uid("00080018", &snapshot))
    else {
        return Err(failure(&snapshot, "no SOP Class UID or SOP Instance UID"));
    };
    let meta = dicom_object::FileMetaTableBuilder::new()
        .transfer_syntax(transfer_syntax)
        .media_storage_sop_class_uid(class)
        .media_storage_sop_instance_uid(instance);
    let file = dicom
        .with_meta(meta)
        .map_err(|e| failure(&snapshot, &format!("no file meta: {e}")))?;
    let mut bytes = Vec::new();
    file.write_all(&mut bytes)
        .map_err(|e| failure(&snapshot, &format!("not written: {e}")))?;
    Ok(Bytes::from(bytes))
}

/// Every BulkDataURI of `object` but the pixel data's, and those of its items, replaced by
/// the named part's content inline; the first URI no part has is noted in `missing`.
fn inline_bulk(
    object: &mut serde_json::Map<String, Value>,
    parts: &[multipart::Part],
    missing: &mut Option<String>,
) {
    use base64::Engine;
    for element in object.values_mut() {
        let Some(element) = element.as_object_mut() else {
            continue;
        };
        if let Some(uri) = element
            .get("BulkDataURI")
            .and_then(Value::as_str)
            .map(str::to_owned)
        {
            match part_named(parts, &uri) {
                Some(part) => {
                    element.remove("BulkDataURI");
                    element.insert(
                        "InlineBinary".into(),
                        Value::String(
                            base64::engine::general_purpose::STANDARD.encode(&part.content),
                        ),
                    );
                }
                None => {
                    missing.get_or_insert(uri);
                }
            }
            continue;
        }
        if let Some(Value::Array(items)) = element.get_mut("Value") {
            for item in items {
                if let Some(item) = item.as_object_mut() {
                    inline_bulk(item, parts, missing);
                }
            }
        }
    }
}
