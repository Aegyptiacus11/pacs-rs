//! `GET /studies/{id}/archive`: a study's files in a ZIP, one folder per patient,
//! study and series, as the reference server lays it out.

use std::collections::HashMap;
use std::io::{Cursor, Write};

use axum::extract::{Path, State};
use axum::http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
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
        .join(" ");
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
            let dir = name(&[&modality, tag(&series.main_tags, "SeriesDescription")]);
            series_dirs.insert(series.public_id, (dir, name(&[&modality])));
        }
    }

    let mut entries = Vec::new();
    let mut counters: HashMap<String, usize> = HashMap::new();
    for instance in index.rest_study_instances(&id).await? {
        let (dir, prefix) = series_dirs
            .get(&instance.parent_series)
            .cloned()
            .unwrap_or_else(|| ("Unknown".into(), "Unknown".into()));
        let n = counters.entry(instance.parent_series.clone()).or_default();
        *n += 1;
        let path = format!("{patient_dir}/{study_dir}/{dir}/{prefix}{n:06}.dcm");
        let file = state.archive.file_at(&instance.object_key).await?;
        entries.push((path, file));
    }

    let zip = tokio::task::spawn_blocking(move || -> zip::result::ZipResult<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .large_file(entries.iter().any(|(_, f)| f.len() >= u32::MAX as usize));
        for (path, file) in entries {
            writer.start_file(path, options)?;
            writer.write_all(&file)?;
        }
        Ok(writer.finish()?.into_inner())
    })
    .await
    .map_err(RestError::internal)?
    .map_err(RestError::internal)?;

    Ok((
        [
            (CONTENT_TYPE, "application/zip".to_owned()),
            (
                CONTENT_DISPOSITION,
                format!(r#"attachment; filename="{id}.zip""#),
            ),
        ],
        zip,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::name;

    #[test]
    fn names_are_safe() {
        assert_eq!(name(&["SYN1", "DOE^JOHN"]), "SYN1 DOE^JOHN");
        // no `/`, and never `..` as a whole path component
        assert_eq!(name(&["../..", "a/b"]), "_.. a_b");
        assert_eq!(name(&[".."]), "Unknown");
        assert_eq!(name(&["", ""]), "Unknown");
        assert_eq!(name(&["CT", ""]), "CT");
    }
}
