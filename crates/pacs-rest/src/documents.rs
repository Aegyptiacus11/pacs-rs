//! PDF documents in studies: `POST /studies/{id}/documents` makes a PDF (the body) into an
//! Encapsulated PDF instance of the study, in a new series of modality DOC, as a report system
//! would; `GET /instances/{id}/pdf` gives an Encapsulated PDF instance's document back.

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_core::document::{NewDocument, encapsulated_pdf, not_a_pdf, pdf_of, utc};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::RestError;
use crate::schema::{Error, Stored};
use crate::scope::Seen;
use crate::upload::stored;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct DocumentParams {
    title: String,
}

/// A new UID under the `2.25` root: a random 122-bit UUID as one decimal number (PS3.5 B.2).
pub(crate) fn new_uid() -> Result<String, RestError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| RestError::internal(format!("no random bytes: {e}")))?;
    // version 4, variant 1 (RFC 9562), as B.2 asks of the UUID
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(format!("2.25.{}", u128::from_be_bytes(bytes)))
}

/// The highest Series Number of a study's series, 0 for none: a new series comes after it.
pub(crate) async fn last_series_number(
    state: &crate::AppState,
    study: &pacs_index::RestStudy,
) -> Result<u32, RestError> {
    let mut last = 0u32;
    for series in &study.series {
        if let Some(s) = state.archive.index().rest_series(series).await? {
            let number = s.main_tags.get("SeriesNumber").and_then(Value::as_str);
            last = last.max(number.and_then(|n| n.trim().parse().ok()).unwrap_or(0));
        }
    }
    Ok(last)
}

/// Adds a PDF document to a study.
///
/// The body is the PDF itself. It becomes an Encapsulated PDF instance (PS3.3 A.45.1) of the
/// study, in a series of its own (modality DOC, described by the title), with the study's
/// patient and study attributes; Content Date and Time are when it was added, in UTC. The
/// answer is that of `POST /instances`, and so are the refusals: an account limited to projects
/// adds nothing, nor does anyone to a study that came in through a project.
#[utoipa::path(
    post,
    path = "/studies/{id}/documents",
    tag = "Studies",
    params(
        ("id" = String, Path, description = "The study's identifier."),
        ("title" = Option<String>, Query, description = "The document's title, and its series' description (64 characters of it); \"PDF document\" if none.")
    ),
    request_body(content = crate::schema::Binary, content_type = "application/pdf", description = "The PDF."),
    responses(
        (status = 200, description = "Stored: the new instance.", body = Stored),
        (status = 400, description = "The body is not a PDF.", body = Error),
        (status = 403, description = "The account is limited to some projects.", body = Error),
        (status = 404, description = "No study has this identifier; the body is empty."),
        (status = 409, description = "The study came in through a project.", body = Error),
        (status = 413, description = "The PDF is larger than `PACS_MAX_UPLOAD_BYTES`.", body = Error)
    )
)]
pub(crate) async fn add(
    Seen(state): Seen,
    Path(id): Path<String>,
    Query(params): Query<DocumentParams>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    if let Some(why) = not_a_pdf(&body) {
        return Err(RestError::bad_request("Not a PDF", why));
    }
    let study = state
        .archive
        .index()
        .rest_study(&id)
        .await?
        .ok_or(RestError::NotFound)?;
    let last = last_series_number(&state, &study).await?;
    let empty = Map::new();
    let title = match params.title.trim() {
        "" => "PDF document",
        t => t,
    };
    let (date, time) = utc(std::time::SystemTime::now());
    let (series_uid, instance_uid) = (new_uid()?, new_uid()?);
    let file = encapsulated_pdf(
        &NewDocument {
            study: study.main_tags.as_object().unwrap_or(&empty),
            patient: study.patient_main_tags.as_object().unwrap_or(&empty),
            title,
            series_uid: &series_uid,
            instance_uid: &instance_uid,
            series_number: last + 1,
            date: &date,
            time: &time,
        },
        &body,
    )
    .map_err(|e| RestError::bad_request("Not a document", e.to_string()))?;
    let ingested = state
        .archive
        .ingest_in_study(Bytes::from(file), &study.study_instance_uid)
        .await;
    stored(&state, ingested, body.len()).await
}

/// An Encapsulated PDF instance's document.
///
/// The PDF as it was put in, without the padding DICOM adds to an odd length.
#[utoipa::path(
    get,
    path = "/instances/{id}/pdf",
    tag = "Instances",
    params(("id" = String, Path, description = "The instance's identifier.")),
    responses(
        (status = 200, description = "The PDF.", body = crate::schema::Binary, content_type = "application/pdf"),
        (status = 404, description = "No instance has this identifier, or it holds no PDF; the body is empty.")
    )
)]
pub(crate) async fn pdf(Seen(state): Seen, Path(id): Path<String>) -> Result<Response, RestError> {
    let instance = state
        .archive
        .index()
        .rest_instance(&id)
        .await?
        .ok_or(RestError::NotFound)?;
    let file = state.archive.file_at(&instance.object_key).await?;
    let object = pacs_core::Instance::from_bytes(&file).map_err(RestError::internal)?;
    let pdf = pdf_of(&object.object).ok_or(RestError::NotFound)?;
    Ok((
        [
            (CONTENT_TYPE, "application/pdf".to_owned()),
            // shown in the browser, saved under its instance's name
            (
                CONTENT_DISPOSITION,
                format!("inline; filename=\"{id}.pdf\""),
            ),
        ],
        pdf,
    )
        .into_response())
}
