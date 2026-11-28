//! `POST /studies/{id}/key-images`: images of a study marked as key images, made into a Key
//! Object Selection Document (PS3.3 A.35.4) of the study, in a series of its own (modality KO),
//! which other PACS and viewers read as the study's key images.

use axum::Json;
use axum::extract::Path;
use bytes::Bytes;
use pacs_core::document::utc;
use pacs_core::key_images::{KeyImage, NewKeyImages, key_object_selection};
use serde::Deserialize;
use serde_json::{Map, Value};
use utoipa::ToSchema;

use crate::RestError;
use crate::documents::{last_series_number, new_uid};
use crate::schema::{Error, Stored};
use crate::scope::Seen;
use crate::upload::stored;

/// The images, and what they are about.
#[derive(Debug, Deserialize, ToSchema)]
pub(crate) struct KeyImages {
    /// The images, by identifier, in the order they are to be read; a frame of a multi-frame
    /// instance as `{"ID": ..., "Frame": n}`.
    #[serde(rename = "Images")]
    images: Vec<KeyImageRef>,
    /// The Key Object Description, if any.
    #[serde(rename = "Description", default)]
    description: String,
}

/// A key image: an instance's identifier, or one frame of a multi-frame instance.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub(crate) enum KeyImageRef {
    Instance(String),
    Frame {
        #[serde(rename = "ID")]
        id: String,
        #[serde(rename = "Frame")]
        frame: u32,
    },
}

/// Marks images of a study as its key images.
///
/// They become a Key Object Selection Document of the study, titled Of Interest (DCM 113000),
/// in a new series of modality KO after the study's last; its answer is that of `POST
/// /instances`, and so are its refusals. Every image must be an instance of this study.
#[utoipa::path(
    post,
    path = "/studies/{id}/key-images",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    request_body(content = KeyImages, description = "The images."),
    responses(
        (status = 200, description = "Stored: the new Key Object Selection.", body = Stored),
        (status = 400, description = "No image, an image that is not of this study, or a frame number of 0.", body = Error),
        (status = 403, description = "The account is limited to some projects.", body = Error),
        (status = 404, description = "No study has this identifier; the body is empty."),
        (status = 409, description = "The study came in through a project.", body = Error)
    )
)]
pub(crate) async fn add(
    Seen(state): Seen,
    Path(id): Path<String>,
    body: Bytes,
) -> Result<Json<Value>, RestError> {
    let asked: KeyImages = serde_json::from_slice(&body).map_err(|e| {
        RestError::bad_request("Bad request", format!("not a list of key images: {e}"))
    })?;
    if asked.images.is_empty() {
        return Err(RestError::bad_request("Bad request", "no image"));
    }
    let index = state.archive.index();
    let study = index.rest_study(&id).await?.ok_or(RestError::NotFound)?;
    let mut images = Vec::new();
    for image in &asked.images {
        let (instance, frame) = match image {
            KeyImageRef::Instance(i) => (i, None),
            KeyImageRef::Frame { frame: 0, id } => {
                return Err(RestError::bad_request(
                    "Bad request",
                    format!("frame 0 of {id}: frames count from 1"),
                ));
            }
            KeyImageRef::Frame { id, frame } => (id, Some(*frame)),
        };
        let found = index
            .rest_instance(instance)
            .await?
            .filter(|i| i.parent_study == study.public_id);
        let Some(found) = found else {
            return Err(RestError::bad_request(
                "Bad request",
                format!("{instance} is not an instance of this study"),
            ));
        };
        images.push(KeyImage {
            sop_class: found.sop_class_uid,
            sop_instance: found.sop_instance_uid,
            series: found.series_instance_uid,
            frame,
        });
    }
    let last = last_series_number(&state, &study).await?;
    let (date, time) = utc(std::time::SystemTime::now());
    let (series_uid, instance_uid) = (new_uid()?, new_uid()?);
    let empty = Map::new();
    let file = key_object_selection(
        &NewKeyImages {
            study: study.main_tags.as_object().unwrap_or(&empty),
            patient: study.patient_main_tags.as_object().unwrap_or(&empty),
            description: &asked.description,
            series_uid: &series_uid,
            instance_uid: &instance_uid,
            series_number: last + 1,
            date: &date,
            time: &time,
        },
        &images,
    )
    .map_err(|e| RestError::bad_request("Bad request", e.to_string()))?;
    let size = file.len();
    let ingested = state
        .archive
        .ingest_in_study(Bytes::from(file), &study.study_instance_uid)
        .await;
    stored(&state, ingested, size).await
}
