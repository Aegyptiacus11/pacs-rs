//! WADO-RS: retrieve studies, series and instances, their metadata, and frames (PS3.18
//! 10.4). The client's preference decides what goes out (`media::choose`): as stored, or
//! transcoded to Explicit VR Little Endian, JPEG-LS lossless or JPEG XL lossless, and
//! nothing else (406). Responses carry validators for the browser's cache (`cache`).

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures::StreamExt;
use pacs_archive::Archive;
use pacs_core::Instance;
use pacs_core::codec::{self, EXPLICIT_VR_LE, JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS, Shape};
use pacs_core::frames::{Frame, FrameError, NATIVE_MEDIA_TYPE, compressed_media_type};
use pacs_index::InstanceEntry;

use crate::cache::Validator;
use crate::media::{Accept, Choice, Packaging, accepts_json, choose};
use crate::qido::DICOM_JSON;
use crate::{ApiError, AppState, multipart};

const DICOM: &str = "application/dicom";

/// What instances and frames can be transcoded to, in this order when a client takes
/// several equally.
const TARGETS: [&str; 3] = [EXPLICIT_VR_LE, JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS];

/// The bulk data media type of frames in `ts`.
fn frame_media_type(ts: &str) -> &'static str {
    if codec::is_native(ts) {
        NATIVE_MEDIA_TYPE
    } else {
        compressed_media_type(ts).unwrap_or(NATIVE_MEDIA_TYPE)
    }
}

/// The candidates for content stored in `stored`, other than `stored` itself.
fn candidates(
    stored: &str,
    part_type: impl Fn(&str) -> &'static str,
) -> Vec<(&'static str, &'static str)> {
    if !codec::can_decode(stored) {
        return Vec::new();
    }
    TARGETS
        .into_iter()
        .filter(|t| *t != stored)
        .map(|t| (part_type(t), t))
        .collect()
}

fn accept(headers: &HeaderMap) -> Accept {
    Accept::parse(headers.get(ACCEPT).and_then(|h| h.to_str().ok()))
}

fn multipart_type(part_type: &str, boundary: &str) -> String {
    format!(r#"multipart/related; type="{part_type}"; boundary={boundary}"#)
}

fn part_type(media_type: &str, ts: &str) -> String {
    format!("{media_type}; transfer-syntax={ts}")
}

/// An instance's file, transcoded to `target` when there is one.
async fn file_as(
    archive: &Archive,
    entry: &InstanceEntry,
    target: Option<&'static str>,
) -> std::io::Result<Bytes> {
    let file = archive.file(entry).await.map_err(std::io::Error::other)?;
    let Some(target) = target else {
        return Ok(file);
    };
    tokio::task::spawn_blocking(move || {
        let instance = Instance::from_bytes(&file).map_err(std::io::Error::other)?;
        instance
            .transcoded(target, false)
            .map(Bytes::from)
            .map_err(std::io::Error::other)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Instances as `multipart/related; type="application/dicom"`, streamed from the store
/// one file at a time, each transcoded on the way if the client prefers another transfer
/// syntax. `fixed` when the response cannot change (see `cache`).
fn retrieve(
    archive: Archive,
    headers: &HeaderMap,
    entries: Vec<InstanceEntry>,
    fixed: bool,
) -> Result<Response, ApiError> {
    if entries.is_empty() {
        return Err(ApiError::not_found("no such instances"));
    }
    let validator = Validator::for_entries(headers, &entries, "", fixed);
    let accept = accept(headers);
    let mut single = entries.len() == 1;
    let mut plan = Vec::with_capacity(entries.len());
    for entry in entries {
        let stored = entry.transfer_syntax_uid.as_str();
        let candidates = candidates(stored, |_| DICOM);
        let (choice, packaging) = choose(&accept, (DICOM, stored), &candidates).ok_or_else(|| {
            ApiError::not_acceptable(format!(
                "instance {} is stored in {stored} and cannot be transcoded to what was asked for",
                entry.sop_instance_uid
            ))
        })?;
        let target = match choice {
            Choice::Stored => None,
            Choice::Candidate(i) => Some(candidates[i].1),
        };
        if packaging == Packaging::Multipart {
            single = false;
        }
        plan.push((entry, target));
    }
    if validator.matches(headers) {
        return Ok(validator.not_modified());
    }
    if single {
        let (entry, target) = plan.into_iter().next().expect("one entry");
        let content_type = part_type(DICOM, target.unwrap_or(&entry.transfer_syntax_uid));
        let body = Body::from_stream(futures::stream::once(async move {
            file_as(&archive, &entry, target).await
        }));
        let mut response = ([(CONTENT_TYPE, content_type)], body).into_response();
        validator.apply(&mut response);
        return Ok(response);
    }
    let boundary = multipart::new_boundary();
    let content_type = multipart_type(DICOM, &boundary);
    let closing = multipart::closing(&boundary);
    let parts = futures::stream::iter(plan).then(move |(entry, target)| {
        let archive = archive.clone();
        let boundary = boundary.clone();
        async move {
            let file = file_as(&archive, &entry, target).await?;
            let ts = target.unwrap_or(&entry.transfer_syntax_uid);
            let header = multipart::part_header(&boundary, &part_type(DICOM, ts));
            let mut part = Vec::with_capacity(header.len() + file.len() + 2);
            part.extend_from_slice(&header);
            part.extend_from_slice(&file);
            part.extend_from_slice(b"\r\n");
            Ok::<_, std::io::Error>(Bytes::from(part))
        }
    });
    let body = parts.chain(futures::stream::once(async move { Ok(closing) }));
    let mut response = ([(CONTENT_TYPE, content_type)], Body::from_stream(body)).into_response();
    validator.apply(&mut response);
    Ok(response)
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
    retrieve(state.archive, &headers, entries, false)
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
    retrieve(state.archive, &headers, entries, false)
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
    let fixed = !state.archive.overwrites();
    retrieve(state.archive, &headers, entries, fixed)
}

/// The stored JSON of each instance, joined into one array as it is: no parsing.
fn metadata_response(headers: &HeaderMap, metadata: Vec<String>) -> Result<Response, ApiError> {
    if !accepts_json(&accept(headers)) {
        return Err(ApiError::not_acceptable(
            "metadata comes as application/dicom+json",
        ));
    }
    if metadata.is_empty() {
        return Err(ApiError::not_found("no such instances"));
    }
    let mut body = String::with_capacity(metadata.iter().map(|m| m.len() + 1).sum::<usize>() + 2);
    body.push('[');
    body.push_str(&metadata.join(","));
    body.push(']');
    let validator = Validator::for_body(&body);
    if validator.matches(headers) {
        return Ok(validator.not_modified());
    }
    let mut response = ([(CONTENT_TYPE, DICOM_JSON)], body).into_response();
    validator.apply(&mut response);
    Ok(response)
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
                .metadata_text(&study, &series.series_instance_uid, None)
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
        .metadata_text(&study, &series, None)
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
        .metadata_text(&study, &series, Some(&instance))
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
    let validator = Validator::for_entries(
        &headers,
        std::slice::from_ref(&entry),
        &list,
        !state.archive.overwrites(),
    );
    if validator.matches(&headers) {
        return Ok(validator.not_modified());
    }
    let file = state.archive.file(&entry).await?;
    let (frames, shape) = tokio::task::spawn_blocking(move || {
        let instance = Instance::from_bytes(&file).map_err(ApiError::internal)?;
        let frames = numbers
            .into_iter()
            .map(|n| instance.frame(n).map_err(frame_error))
            .collect::<Result<Vec<_>, ApiError>>()?;
        Ok::<_, ApiError>((frames, instance.shape()))
    })
    .await
    .map_err(ApiError::internal)??;

    let accept = accept(&headers);
    let (media_type, ts) = (frames[0].media_type, frames[0].transfer_syntax_uid.clone());
    let candidates = candidates(&ts, frame_media_type);
    let refused = |why: String| {
        ApiError::not_acceptable(format!(
            "frames are stored as {media_type} in {ts} and {why}"
        ))
    };
    let (choice, packaging) = choose(&accept, (media_type, &ts), &candidates)
        .ok_or_else(|| refused("cannot be transcoded to what was asked for".into()))?;
    let frames = match choice {
        Choice::Stored => frames,
        Choice::Candidate(i) => {
            let shape = shape.map_err(|e| refused(e.to_string()))?;
            let target = candidates[i].1;
            tokio::task::spawn_blocking(move || transcode_frames(frames, shape, target))
                .await
                .map_err(ApiError::internal)?
                .map_err(|e| refused(format!("could not be transcoded: {e}")))?
        }
    };
    let (media_type, ts) = (frames[0].media_type, frames[0].transfer_syntax_uid.clone());
    if packaging == Packaging::Single && frames.len() == 1 {
        let frame = frames.into_iter().next().expect("one frame");
        let mut response =
            ([(CONTENT_TYPE, part_type(media_type, &ts))], frame.bytes).into_response();
        validator.apply(&mut response);
        return Ok(response);
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
    let mut response = (
        [(CONTENT_TYPE, multipart_type(media_type, &boundary))],
        body,
    )
        .into_response();
    validator.apply(&mut response);
    Ok(response)
}

/// Each frame decoded and encoded again in `target`.
fn transcode_frames(
    frames: Vec<Frame>,
    shape: Shape,
    target: &'static str,
) -> Result<Vec<Frame>, codec::CodecError> {
    frames
        .into_iter()
        .map(|frame| {
            let native = codec::decode(&frame.transfer_syntax_uid, shape, &frame.bytes)?;
            Ok(Frame {
                bytes: codec::encode(target, shape, &native)?,
                transfer_syntax_uid: target.to_owned(),
                media_type: frame_media_type(target),
            })
        })
        .collect()
}
