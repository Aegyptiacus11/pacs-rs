//! `GET /studies/{id}/archive`, `/patients/{id}/archive` and `/tools/create-archive`: the files
//! in a ZIP, one folder per patient, study and series, as the reference server (26.9.1) lays it
//! out: `PatientID - PatientName/AccessionNumber - StudyDescription/
//! SeriesNumber - Modality - SeriesDescription/CT000001.dcm`. Streamed: see [`zip_stream`].

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_archive::Archive;
use serde::Deserialize;
use serde_json::Value;
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::{AppState, RestError};

fn tag<'a>(tags: &'a Value, name: &str) -> &'a str {
    tags[name].as_str().unwrap_or("")
}

/// A folder or file name from DICOM values: nothing that would make a path or upset a
/// file system, "Unknown" when nothing is left.
fn name(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter(|p| !p.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(" - ");
    let clean: String = joined
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_^.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let clean = clean.trim().trim_matches('.').to_owned();
    if clean.is_empty() {
        "Unknown".into()
    } else {
        clean
    }
}

pub(crate) async fn study(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, RestError> {
    let index = state.archive.index();
    let study = index.rest_study(&id).await?.ok_or(RestError::NotFound)?;
    let entries = study_entries(index, study).await?;
    Ok(zip_response(&state, &format!("{id}.zip"), files(entries)))
}

/// `GET /patients/{id}/archive`: each of the patient's studies, laid out as above.
pub(crate) async fn patient(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, RestError> {
    let index = state.archive.index();
    let patient = index.rest_patient(&id).await?.ok_or(RestError::NotFound)?;
    let mut entries = Vec::new();
    for study_id in &patient.studies {
        if let Some(study) = index.rest_study(study_id).await? {
            entries.extend(study_entries(index, study).await?);
        }
    }
    Ok(zip_response(&state, &format!("{id}.zip"), files(entries)))
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateArchive {
    #[serde(rename = "Resources")]
    resources: Vec<String>,
    #[serde(rename = "Synchronous", default = "yes")]
    synchronous: bool,
    #[serde(rename = "Transcode")]
    transcode: Option<Value>,
}

fn yes() -> bool {
    true
}

/// `POST /tools/create-archive` with `{"Resources": [...]}`: patients, studies, series and
/// instances in one ZIP, each file once however many of the resources hold it.
pub(crate) async fn create_archive(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Response, RestError> {
    let request: CreateArchive = serde_json::from_slice(&body).map_err(|e| {
        RestError::bad_request("Bad request", format!("not an archive request: {e}"))
    })?;
    if !request.synchronous {
        return Err(RestError::bad_request(
            "Not supported yet",
            "only synchronous archives: there are no jobs yet",
        ));
    }
    if request.transcode.is_some() {
        return Err(RestError::bad_request(
            "Not supported yet",
            "archives hold the files as stored: no Transcode",
        ));
    }
    resources_archive(&state, &request.resources).await
}

/// `GET /tools/create-archive?resources=a,b`: the same, so that a link can download it.
pub(crate) async fn create_archive_get(
    State(state): State<AppState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, RestError> {
    let ids: Vec<String> = params
        .get("resources")
        .map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    resources_archive(&state, &ids).await
}

/// The files of `ids` (any level) in the order they are named. A series or instance keeps
/// the folder and number it has in its study's archive. An unknown ID fails the whole
/// request, before anything is sent.
async fn resources_archive(state: &AppState, ids: &[String]) -> Result<Response, RestError> {
    let index = state.archive.index();
    let mut studies: HashMap<String, Vec<Entry>> = HashMap::new();
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        // (study, series to keep, instance to keep); None keeps all
        let mut wanted: Vec<(String, Option<String>, Option<String>)> = Vec::new();
        if let Some(patient) = index.rest_patient(id).await? {
            wanted.extend(patient.studies.into_iter().map(|s| (s, None, None)));
        } else if index.rest_study(id).await?.is_some() {
            wanted.push((id.clone(), None, None));
        } else if let Some(series) = index.rest_series(id).await? {
            wanted.push((series.parent_study, Some(id.clone()), None));
        } else if let Some(instance) = index.rest_instance(id).await? {
            wanted.push((instance.parent_study, None, Some(id.clone())));
        } else {
            return Err(RestError::NotFound);
        }
        for (study_id, series, instance) in wanted {
            if !studies.contains_key(&study_id) {
                let Some(study) = index.rest_study(&study_id).await? else {
                    continue;
                };
                studies.insert(study_id.clone(), study_entries(index, study).await?);
            }
            for entry in &studies[&study_id] {
                let keep = series.as_ref().is_none_or(|s| *s == entry.series)
                    && instance.as_ref().is_none_or(|i| *i == entry.instance);
                if keep && seen.insert(entry.instance.clone()) {
                    entries.push(entry.clone());
                }
            }
        }
    }
    Ok(zip_response(state, "Archive.zip", files(entries)))
}

/// One file of an archive.
#[derive(Debug, Clone)]
struct Entry {
    instance: String,
    series: String,
    path: String,
    object_key: String,
}

fn files(entries: Vec<Entry>) -> Vec<(String, String)> {
    entries
        .into_iter()
        .map(|e| (e.path, e.object_key))
        .collect()
}

/// A study's files, in series then instance order.
async fn study_entries(
    index: &pacs_index::Index,
    study: pacs_index::RestStudy,
) -> Result<Vec<Entry>, RestError> {
    let patient_dir = name(&[
        tag(&study.patient_main_tags, "PatientID"),
        tag(&study.patient_main_tags, "PatientName"),
    ]);
    let study_dir = name(&[
        tag(&study.main_tags, "AccessionNumber"),
        tag(&study.main_tags, "StudyDescription"),
    ]);
    let mut series_dirs = HashMap::new();
    for series_id in &study.series {
        if let Some(series) = index.rest_series(series_id).await? {
            let modality = tag(&series.main_tags, "Modality").to_owned();
            let dir = name(&[
                tag(&series.main_tags, "SeriesNumber"),
                &modality,
                tag(&series.main_tags, "SeriesDescription"),
            ]);
            series_dirs.insert(series.public_id, (dir, name(&[&modality])));
        }
    }

    let mut entries = Vec::new();
    let mut counters: HashMap<String, usize> = HashMap::new();
    for instance in index.rest_study_instances(&study.public_id).await? {
        let (dir, prefix) = series_dirs
            .get(&instance.parent_series)
            .cloned()
            .unwrap_or_else(|| ("Unknown".into(), "Unknown".into()));
        let n = counters.entry(instance.parent_series.clone()).or_default();
        *n += 1;
        let path = format!("{patient_dir}/{study_dir}/{dir}/{prefix}{n:06}.dcm");
        entries.push(Entry {
            instance: instance.public_id,
            series: instance.parent_series,
            path,
            object_key: instance.object_key,
        });
    }
    Ok(entries)
}

fn zip_response(state: &AppState, filename: &str, entries: Vec<(String, String)>) -> Response {
    let body = Body::from_stream(zip_stream(state.archive.clone(), entries));
    (
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (
                CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{filename}""#),
            ),
        ],
        body,
    )
        .into_response()
}

/// Where the ZIP writer puts its bytes, for the stream to take them from as they come.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("not poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Sink {
    fn take(&self) -> Bytes {
        Bytes::from(std::mem::take(&mut *self.0.lock().expect("not poisoned")))
    }
}

type Writer = zip::ZipWriter<zip::write::StreamWriter<Sink>>;

/// The ZIP of `entries`, made as it is sent: each step reads one file from the store and
/// compresses it, so memory holds one file, not the study, and a slow client slows the
/// reading. Entries carry their sizes after their data (streaming ZIP), as any unzip
/// reads them. An error midway ends the body early: the client sees a cut-off ZIP.
fn zip_stream(
    archive: Archive,
    entries: Vec<(String, String)>,
) -> impl futures::Stream<Item = std::io::Result<Bytes>> {
    let sink = Sink::default();
    let writer: Writer = zip::ZipWriter::new_stream(sink.clone());
    futures::stream::unfold(Some((writer, entries.into_iter())), move |state| {
        let (archive, sink) = (archive.clone(), sink.clone());
        async move {
            let (mut writer, mut entries) = state?;
            let step = match entries.next() {
                Some((path, object_key)) => {
                    let file = match archive.file_at(&object_key).await {
                        Ok(file) => file,
                        Err(e) => return Some((Err(std::io::Error::other(e)), None)),
                    };
                    tokio::task::spawn_blocking(move || {
                        let options = SimpleFileOptions::default()
                            .compression_method(CompressionMethod::Deflated)
                            .large_file(file.len() >= u32::MAX as usize);
                        writer.start_file(path, options)?;
                        writer.write_all(&file)?;
                        Ok::<_, zip::result::ZipError>(Some(writer))
                    })
                    .await
                }
                None => {
                    tokio::task::spawn_blocking(move || {
                        writer.finish()?;
                        Ok(None)
                    })
                    .await
                }
            };
            match step {
                Ok(Ok(Some(writer))) => Some((Ok(sink.take()), Some((writer, entries)))),
                Ok(Ok(None)) => Some((Ok(sink.take()), None)),
                Ok(Err(e)) => Some((Err(std::io::Error::other(e)), None)),
                Err(e) => Some((Err(std::io::Error::other(e)), None)),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn names_are_safe() {
        assert_eq!(name(&["SYN1", "DOE^JOHN"]), "SYN1 - DOE^JOHN");
        // no `/`, and never `..` as a whole path component
        assert_eq!(name(&["../..", "a/b"]), "_.. - a_b");
        assert_eq!(name(&[".."]), "Unknown");
        assert_eq!(name(&["", ""]), "Unknown");
        assert_eq!(name(&["CT", ""]), "CT");
    }
}
