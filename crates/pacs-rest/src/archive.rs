//! `GET /studies/{id}/archive`, `/patients/{id}/archive`: the files in a ZIP, one folder per patient,
//! study and series, as the reference server (26.9.1) lays it out:
//! `PatientID - PatientName/AccessionNumber - StudyDescription/
//! SeriesNumber - Modality - SeriesDescription/CT000001.dcm`. Streamed: see [`zip_stream`].

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use pacs_archive::Archive;
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
    Ok(zip_response(&state, &id, entries))
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
    Ok(zip_response(&state, &id, entries))
}

/// (path in the ZIP, storage key) of a study's files, in series then instance order.
async fn study_entries(
    index: &pacs_index::Index,
    study: pacs_index::RestStudy,
) -> Result<Vec<(String, String)>, RestError> {
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
        entries.push((path, instance.object_key));
    }
    Ok(entries)
}

fn zip_response(state: &AppState, id: &str, entries: Vec<(String, String)>) -> Response {
    let body = Body::from_stream(zip_stream(state.archive.clone(), entries));
    (
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (
                CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{id}.zip""#),
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
