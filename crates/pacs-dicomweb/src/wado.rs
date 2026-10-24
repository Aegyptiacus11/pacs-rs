//! WADO-RS: retrieve studies, series and instances as stored, their metadata, and
//! frames (PS3.18 10.4). Nothing is transcoded: a request for a transfer syntax other
//! than the stored one answers 406.

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use pacs_archive::Archive;
use pacs_core::Instance;
use pacs_core::frames::{Frame, FrameError};
use pacs_index::InstanceEntry;
use serde_json::Value;

use crate::media::{Accept, Packaging, accepts_json, negotiate};
use crate::qido::json_response;
use crate::{ApiError, AppState, multipart};

const DICOM: &str = "application/dicom";

fn accept(headers: &HeaderMap) -> Accept {
    Accept::parse(headers.get(ACCEPT).and_then(|h| h.to_str().ok()))
}

fn multipart_type(part_type: &str, boundary: &str) -> String {
    format!(r#"multipart/related; type="{part_type}"; boundary={boundary}"#)
}

fn part_type(media_type: &str, ts: &str) -> String {
    format!("{media_type}; transfer-syntax={ts}")
}

/// Instances as `multipart/related; type="application/dicom"`, streamed from the store
/// one file at a time.
fn retrieve(
    archive: Archive,
    headers: &HeaderMap,
    entries: Vec<InstanceEntry>,
) -> Result<Response, ApiError> {
    if entries.is_empty() {
        return Err(ApiError::not_found("no such instances"));
    }
    let accept = accept(headers);
    let mut single = entries.len() == 1;
    for entry in &entries {
        match negotiate(&accept, DICOM, &entry.transfer_syntax_uid) {
            None => {
                return Err(ApiError::not_acceptable(format!(
                    "instance {} is stored in {} and is not transcoded",
                    entry.sop_instance_uid, entry.transfer_syntax_uid
                )));
            }
            Some(Packaging::Multipart) => single = false,
            Some(Packaging::Single) => {}
        }
    }
    if single {
        let entry = entries.into_iter().next().expect("one entry");
        let content_type = part_type(DICOM, &entry.transfer_syntax_uid);
        let body = Body::from_stream(futures::stream::once(async move {
            archive.file(&entry).await.map_err(std::io::Error::other)
        }));
        return Ok(([(CONTENT_TYPE, content_type)], body).into_response());
    }
    let boundary = multipart::new_boundary();
    let content_type = multipart_type(DICOM, &boundary);
    let closing = multipart::closing(&boundary);
    let parts = futures::stream::iter(entries).then(move |entry| {
        let archive = archive.clone();
        let boundary = boundary.clone();
        async move {
            let file = archive.file(&entry).await.map_err(std::io::Error::other)?;
            let header =
                multipart::part_header(&boundary, &part_type(DICOM, &entry.transfer_syntax_uid));
            let mut part = Vec::with_capacity(header.len() + file.len() + 2);
            part.extend_from_slice(&header);
            part.extend_from_slice(&file);
            part.extend_from_slice(b"\r\n");
            Ok::<_, std::io::Error>(Bytes::from(part))
        }
    });
    let body = parts.chain(futures::stream::once(async move { Ok(closing) }));
    Ok(([(CONTENT_TYPE, content_type)], Body::from_stream(body)).into_response())
}

async fn study_entries(archive: &Archive, study: &str) -> Result<Vec<InstanceEntry>, ApiError> {
    let mut entries = Vec::new();
    for series in archive.index().series(study, None).await? {
        entries.extend(
            archive
                .index()
                .instances(study, &series.series_instance_uid, None)
                .await?,
        );
    }
    Ok(entries)
}

pub(crate) async fn study(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study): Path<String>,
) -> Result<Response, ApiError> {
    let entries = study_entries(&state.archive, &study).await?;
    retrieve(state.archive, &headers, entries)
}

pub(crate) async fn series(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let entries = state
        .archive
        .index()
        .instances(&study, &series, None)
        .await?;
    retrieve(state.archive, &headers, entries)
}

pub(crate) async fn instance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series, instance)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let entries = state
        .archive
        .index()
        .instances(&study, &series, Some(&instance))
        .await?;
    retrieve(state.archive, &headers, entries)
}

fn metadata_response(headers: &HeaderMap, metadata: Vec<Value>) -> Result<Response, ApiError> {
    if !accepts_json(&accept(headers)) {
        return Err(ApiError::not_acceptable(
            "metadata comes as application/dicom+json",
        ));
    }
    if metadata.is_empty() {
        return Err(ApiError::not_found("no such instances"));
    }
    Ok(json_response(Value::Array(metadata)))
}

pub(crate) async fn study_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(study): Path<String>,
) -> Result<Response, ApiError> {
    let index = state.archive.index();
    let mut metadata = Vec::new();
    for series in index.series(&study, None).await? {
        metadata.extend(
            index
                .metadata(&study, &series.series_instance_uid, None)
                .await?,
        );
    }
    metadata_response(&headers, metadata)
}

pub(crate) async fn series_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let metadata = state
        .archive
        .index()
        .metadata(&study, &series, None)
        .await?;
    metadata_response(&headers, metadata)
}

pub(crate) async fn instance_metadata(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series, instance)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let metadata = state
        .archive
        .index()
        .metadata(&study, &series, Some(&instance))
        .await?;
    metadata_response(&headers, metadata)
}

/// `1`, `1,3,5`: frame numbers, from 1.
fn frame_numbers(list: &str) -> Result<Vec<u32>, ApiError> {
    list.split(',')
        .map(|n| {
            n.trim()
                .parse::<u32>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| ApiError::bad_request(format!("not a frame number: {n:?}")))
        })
        .collect()
}

fn frame_error(e: FrameError) -> ApiError {
    match e {
        FrameError::NoPixelData | FrameError::OutOfRange(..) => ApiError::not_found(e.to_string()),
        FrameError::Unsupported(_) => ApiError::new(StatusCode::NOT_IMPLEMENTED, e.to_string()),
    }
}

pub(crate) async fn frames(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((study, series, instance, list)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let numbers = frame_numbers(&list)?;
    let entry = state
        .archive
        .index()
        .instance(&study, &series, &instance)
        .await?
        .ok_or_else(|| ApiError::not_found("no such instance"))?;
    let file = state.archive.file(&entry).await?;
    let frames: Vec<Frame> = tokio::task::spawn_blocking(move || {
        let instance = Instance::from_bytes(&file).map_err(ApiError::internal)?;
        numbers
            .into_iter()
            .map(|n| instance.frame(n).map_err(frame_error))
            .collect::<Result<Vec<_>, ApiError>>()
    })
    .await
    .map_err(ApiError::internal)??;

    let (media_type, ts) = (frames[0].media_type, frames[0].transfer_syntax_uid.clone());
    let packaging = negotiate(&accept(&headers), media_type, &ts).ok_or_else(|| {
        ApiError::not_acceptable(format!(
            "frames are stored as {media_type} in {ts} and are not transcoded"
        ))
    })?;
    if packaging == Packaging::Single && frames.len() == 1 {
        let frame = frames.into_iter().next().expect("one frame");
        return Ok(([(CONTENT_TYPE, part_type(media_type, &ts))], frame.bytes).into_response());
    }
    let boundary = multipart::new_boundary();
    let body = multipart::body(
        &boundary,
        frames.into_iter().map(|f| {
            (
                part_type(f.media_type, &f.transfer_syntax_uid),
                Bytes::from(f.bytes),
            )
        }),
    );
    Ok((
        [(CONTENT_TYPE, multipart_type(media_type, &boundary))],
        body,
    )
        .into_response())
}
