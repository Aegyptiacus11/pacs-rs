//! `POST /instances`: one DICOM file as the body.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use bytes::Bytes;
use pacs_archive::{IngestError, Outcome};
use serde_json::{Value, json};

use crate::schema::{Error, Stored};
use crate::{AppState, RestError};

/// Stores one DICOM file.
///
/// The file is filed under the patient, study and series it names. An instance already stored
/// is kept as it was and answered `AlreadyStored`, unless the server runs with
/// `PACS_OVERWRITE_INSTANCES`.
#[utoipa::path(
    post,
    path = "/instances",
    tag = "Instances",
    request_body(content = crate::schema::Binary, content_type = "application/dicom", description = "One DICOM file, with its preamble and File Meta Information."),
    responses(
        (status = 200, description = "Stored, or already there.", body = Stored),
        (status = 400, description = "The body is not a DICOM file.", body = Error),
        (status = 409, description = "The file names a series or instance that is stored under another study or series, or a study that came in through a project: only that project's AE title adds to it.", body = Error),
        (status = 413, description = "The file is larger than `PACS_MAX_UPLOAD_BYTES`, or would take too much memory to read.", body = Error)
    )
)]
pub(crate) async fn instance(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    let size = body.len();
    let ingested = match state.archive.ingest(body).await {
        Ok(ingested) => ingested,
        Err(IngestError::Invalid(e)) => {
            return Err(RestError::bad_request(
                "Bad file format",
                format!("Cannot parse an invalid DICOM file (size: {size} bytes): {e}"),
            ));
        }
        Err(e @ IngestError::TooCostly { .. }) => {
            return Err(RestError::Status {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: "Too costly to read".into(),
                details: Some(e.to_string()),
            });
        }
        Err(IngestError::Conflict(e)) => {
            return Err(RestError::Status {
                status: StatusCode::CONFLICT,
                message: "Conflict with the stored hierarchy".into(),
                details: Some(e),
            });
        }
        Err(e) => return Err(RestError::internal(e)),
    };
    let instance = state
        .archive
        .index()
        .rest_instance_by_uid(&ingested.key.sop_instance_uid)
        .await?
        .ok_or_else(|| RestError::internal("the instance just stored is not in the index"))?;
    Ok(Json(json!({
        "ID": instance.public_id,
        "ParentPatient": instance.parent_patient,
        "ParentSeries": instance.parent_series,
        "ParentStudy": instance.parent_study,
        "Path": format!("/instances/{}", instance.public_id),
        "Status": match ingested.outcome {
            Outcome::Added | Outcome::Replaced => "Success",
            Outcome::AlreadyStored => "AlreadyStored",
        },
    })))
}
