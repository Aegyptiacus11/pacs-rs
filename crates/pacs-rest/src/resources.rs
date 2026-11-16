//! Patients, studies, series and instances as JSON, by public ID.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query};
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use pacs_index::{RestInstance, RestPatient, RestSeries, RestStudy, Statistics};
use serde_json::{Value, json};

use crate::schema::{Error, Instance, Patient, PatientList, Series, Study, StudyList};
use crate::scope::Seen;
use crate::{AppState, RestError};

pub(crate) fn patient_json(patient: RestPatient) -> Value {
    json!({
        "ID": patient.public_id,
        // nothing is protected from recycling: pacs-rs never recycles
        "IsProtected": false,
        "IsStable": patient.is_stable,
        "Labels": [],
        "LastUpdate": patient.last_update,
        "MainDicomTags": patient.main_tags,
        "Studies": patient.studies,
        "Type": "Patient",
    })
}

pub(crate) fn study_json(study: RestStudy) -> Value {
    json!({
        "ID": study.public_id,
        "IsStable": study.is_stable,
        "Labels": [],
        "LastUpdate": study.last_update,
        "MainDicomTags": study.main_tags,
        "ParentPatient": study.patient_public_id,
        "PatientMainDicomTags": study.patient_main_tags,
        "Series": study.series,
        "Type": "Study",
    })
}

pub(crate) fn series_json(series: RestSeries) -> Value {
    json!({
        // computing these needs the acquisition's expected image count; not done yet
        "ExpectedNumberOfInstances": null,
        "ID": series.public_id,
        "Instances": series.instances,
        "IsStable": series.is_stable,
        "Labels": [],
        "LastUpdate": series.last_update,
        "MainDicomTags": series.main_tags,
        "ParentStudy": series.parent_study,
        "Status": "Unknown",
        "Type": "Series",
    })
}

pub(crate) fn instance_json(instance: RestInstance) -> Value {
    let mut value = json!({
        "FileSize": instance.size_bytes,
        "ID": instance.public_id,
        "Labels": [],
        "MainDicomTags": instance.main_tags,
        "ParentSeries": instance.parent_series,
        "Type": "Instance",
    });
    if let Some(n) = instance.instance_number {
        value["IndexInSeries"] = n.into();
    }
    value
}

/// `?expand`, `?expand=true`: objects instead of IDs. `?expand=false` is IDs.
fn expand(params: &HashMap<String, String>) -> bool {
    params
        .get("expand")
        .is_some_and(|v| !matches!(v.as_str(), "false" | "0"))
}

fn number(params: &HashMap<String, String>, name: &str) -> Result<Option<usize>, RestError> {
    params
        .get(name)
        .map(|v| {
            v.parse()
                .map_err(|_| RestError::bad_request(format!("Bad {name}"), format!("{name}={v}")))
        })
        .transpose()
}

/// `ids` from `?since` on, at most `?limit` of them.
fn page(ids: Vec<String>, params: &HashMap<String, String>) -> Result<Vec<String>, RestError> {
    let since = number(params, "since")?.unwrap_or(0);
    Ok(match number(params, "limit")? {
        Some(limit) => ids.into_iter().skip(since).take(limit).collect(),
        None => ids.into_iter().skip(since).collect(),
    })
}

/// Lists the patients.
#[utoipa::path(
    get,
    path = "/patients",
    tag = "Patients",
    params(
        ("expand" = Option<bool>, Query, description = "Answer with the resources instead of their identifiers. `?expand` alone means true."),
        ("limit" = Option<u64>, Query, description = "At most this many."),
        ("since" = Option<u64>, Query, description = "Skip this many first: with `limit`, a page.")
    ),
    responses(
        (status = 200, description = "The patients' identifiers, or the patients.", body = PatientList),
        (status = 400, description = "`limit` or `since` is not a number.", body = Error)
    )
)]
pub(crate) async fn patients(
    Seen(state): Seen,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    let ids = page(index.patient_public_ids(None).await?, &params)?;
    if !expand(&params) {
        return Ok(Json(json!(ids)));
    }
    let mut patients = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(patient) = index.rest_patient(&id).await? {
            patients.push(patient_json(patient));
        }
    }
    Ok(Json(Value::Array(patients)))
}

/// A patient.
#[utoipa::path(
    get,
    path = "/patients/{id}",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "The patient.", body = Patient),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let patient = state.archive.index().rest_patient(&id).await?;
    Ok(Json(patient_json(patient.ok_or(RestError::NotFound)?)))
}

/// A patient's studies.
#[utoipa::path(
    get,
    path = "/patients/{id}/studies",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "The studies.", body = Vec<Study>),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient_studies(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    let patient = index.rest_patient(&id).await?.ok_or(RestError::NotFound)?;
    let mut studies = Vec::new();
    for id in patient.studies {
        if let Some(s) = index.rest_study(&id).await? {
            studies.push(study_json(s));
        }
    }
    Ok(Json(Value::Array(studies)))
}

/// A patient's series, of every study.
#[utoipa::path(
    get,
    path = "/patients/{id}/series",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "The series.", body = Vec<Series>),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient_series(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    if index.rest_patient(&id).await?.is_none() {
        return Err(RestError::NotFound);
    }
    let mut series = Vec::new();
    for id in index.patient_series_ids(&id).await? {
        if let Some(s) = index.rest_series(&id).await? {
            series.push(series_json(s));
        }
    }
    Ok(Json(Value::Array(series)))
}

/// A patient's instances, of every study.
#[utoipa::path(
    get,
    path = "/patients/{id}/instances",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "The instances.", body = Vec<Instance>),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient_instances(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    if index.rest_patient(&id).await?.is_none() {
        return Err(RestError::NotFound);
    }
    let instances = index.rest_patient_instances(&id).await?;
    Ok(Json(Value::Array(
        instances.into_iter().map(instance_json).collect(),
    )))
}

/// How much a patient holds.
#[utoipa::path(
    get,
    path = "/patients/{id}/statistics",
    tag = "Patients",
    params(("id" = String, Path, description = "The patient's identifier.")),
    responses(
        (status = 200, description = "Counts and sizes.", body = crate::schema::Statistics),
        (status = 404, description = "No patient has this identifier; the body is empty.")
    )
)]
pub(crate) async fn patient_statistics(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let stats = state
        .archive
        .index()
        .patient_statistics(&id)
        .await?
        .ok_or(RestError::NotFound)?;
    let mut value = statistics_json(stats);
    value["CountStudies"] = stats.studies.into();
    Ok(Json(value))
}

/// Lists the studies.
#[utoipa::path(
    get,
    path = "/studies",
    tag = "Studies",
    params(
        ("expand" = Option<bool>, Query, description = "Answer with the resources instead of their identifiers. `?expand` alone means true."),
        ("limit" = Option<u64>, Query, description = "At most this many."),
        ("since" = Option<u64>, Query, description = "Skip this many first: with `limit`, a page.")
    ),
    responses(
        (status = 200, description = "The studies' identifiers, or the studies.", body = StudyList),
        (status = 400, description = "`limit` or `since` is not a number.", body = Error)
    )
)]
pub(crate) async fn studies(
    Seen(state): Seen,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    let ids = page(index.study_public_ids().await?, &params)?;
    if !expand(&params) {
        return Ok(Json(json!(ids)));
    }
    let mut studies = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(study) = index.rest_study(&id).await? {
            studies.push(study_json(study));
        }
    }
    Ok(Json(Value::Array(studies)))
}

/// A study.
#[utoipa::path(
    get,
    path = "/studies/{id}",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    responses(
        (status = 200, description = "The study.", body = Study),
        (status = 404, description = "No study has this identifier; the body is empty.")
    )
)]
pub(crate) async fn study(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let study = state.archive.index().rest_study(&id).await?;
    Ok(Json(study_json(study.ok_or(RestError::NotFound)?)))
}

/// A study's series, in Series Number order.
#[utoipa::path(
    get,
    path = "/studies/{id}/series",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    responses(
        (status = 200, description = "The series.", body = Vec<Series>),
        (status = 404, description = "No study has this identifier; the body is empty.")
    )
)]
pub(crate) async fn study_series(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    let study = index.rest_study(&id).await?.ok_or(RestError::NotFound)?;
    let mut series = Vec::new();
    for id in study.series {
        if let Some(s) = index.rest_series(&id).await? {
            series.push(series_json(s));
        }
    }
    Ok(Json(Value::Array(series)))
}

/// A study's instances, of every series.
#[utoipa::path(
    get,
    path = "/studies/{id}/instances",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    responses(
        (status = 200, description = "The instances.", body = Vec<Instance>),
        (status = 404, description = "No study has this identifier; the body is empty.")
    )
)]
pub(crate) async fn study_instances(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    if index.rest_study(&id).await?.is_none() {
        return Err(RestError::NotFound);
    }
    let instances = index.rest_study_instances(&id).await?;
    Ok(Json(Value::Array(
        instances.into_iter().map(instance_json).collect(),
    )))
}

/// How much a study holds.
#[utoipa::path(
    get,
    path = "/studies/{id}/statistics",
    tag = "Studies",
    params(("id" = String, Path, description = "The study's identifier.")),
    responses(
        (status = 200, description = "Counts and sizes.", body = crate::schema::Statistics),
        (status = 404, description = "No study has this identifier; the body is empty.")
    )
)]
pub(crate) async fn study_statistics(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let stats = state
        .archive
        .index()
        .study_statistics(&id)
        .await?
        .ok_or(RestError::NotFound)?;
    Ok(Json(statistics_json(stats)))
}

fn statistics_json(stats: Statistics) -> Value {
    // sizes are strings and the MB figures whole mebibytes, as the reference answers;
    // files are stored as received, so every size is the same
    let size = stats.size_bytes.to_string();
    let mb = stats.size_bytes / (1024 * 1024);
    json!({
        "CountInstances": stats.instances,
        "CountSeries": stats.series,
        "DicomDiskSize": size,
        "DicomDiskSizeMB": mb,
        "DicomUncompressedSize": size,
        "DicomUncompressedSizeMB": mb,
        "DiskSize": size,
        "DiskSizeMB": mb,
        "UncompressedSize": size,
        "UncompressedSizeMB": mb,
    })
}

/// A series.
#[utoipa::path(
    get,
    path = "/series/{id}",
    tag = "Series",
    params(("id" = String, Path, description = "The series's identifier.")),
    responses(
        (status = 200, description = "The series.", body = Series),
        (status = 404, description = "No series has this identifier; the body is empty.")
    )
)]
pub(crate) async fn series(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let series = state.archive.index().rest_series(&id).await?;
    Ok(Json(series_json(series.ok_or(RestError::NotFound)?)))
}

/// A series' instances, in Instance Number order.
#[utoipa::path(
    get,
    path = "/series/{id}/instances",
    tag = "Series",
    params(("id" = String, Path, description = "The series's identifier.")),
    responses(
        (status = 200, description = "The instances.", body = Vec<Instance>),
        (status = 404, description = "No series has this identifier; the body is empty.")
    )
)]
pub(crate) async fn series_instances(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let index = state.archive.index();
    let series = index.rest_series(&id).await?.ok_or(RestError::NotFound)?;
    let mut instances = Vec::new();
    for id in series.instances {
        if let Some(i) = index.rest_instance(&id).await? {
            instances.push(instance_json(i));
        }
    }
    Ok(Json(Value::Array(instances)))
}

/// An instance.
#[utoipa::path(
    get,
    path = "/instances/{id}",
    tag = "Instances",
    params(("id" = String, Path, description = "The instance's identifier.")),
    responses(
        (status = 200, description = "The instance.", body = Instance),
        (status = 404, description = "No instance has this identifier; the body is empty.")
    )
)]
pub(crate) async fn instance(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Json<Value>, RestError> {
    let instance = state.archive.index().rest_instance(&id).await?;
    Ok(Json(instance_json(instance.ok_or(RestError::NotFound)?)))
}

/// An instance's DICOM file.
///
/// The file as stored: in JPEG-LS Lossless when pacs-rs compressed it on the way in. DICOMweb
/// retrieves it in another transfer syntax.
#[utoipa::path(
    get,
    path = "/instances/{id}/file",
    tag = "Instances",
    params(("id" = String, Path, description = "The instance's identifier.")),
    responses(
        (status = 200, description = "The DICOM file.", body = crate::schema::Binary, content_type = "application/dicom"),
        (status = 404, description = "No instance has this identifier; the body is empty.")
    )
)]
pub(crate) async fn instance_file(
    Seen(state): Seen,
    Path(id): Path<String>,
) -> Result<Response, RestError> {
    let instance = state
        .archive
        .index()
        .rest_instance(&id)
        .await?
        .ok_or(RestError::NotFound)?;
    let file = state.archive.file_at(&instance.object_key).await?;
    Ok(([(CONTENT_TYPE, "application/dicom")], file).into_response())
}

/// `GET /{level}/{id}/{ancestor}`: the resource's patient, study or series.
pub(crate) mod parents {
    use super::*;

    async fn patient_of_study(state: &AppState, study: &str) -> Result<Json<Value>, RestError> {
        let index = state.archive.index();
        let study = index.rest_study(study).await?.ok_or(RestError::NotFound)?;
        let patient = index.rest_patient(&study.patient_public_id).await?;
        Ok(Json(patient_json(patient.ok_or(RestError::NotFound)?)))
    }

    async fn study(state: &AppState, study: &str) -> Result<Json<Value>, RestError> {
        let study = state.archive.index().rest_study(study).await?;
        Ok(Json(study_json(study.ok_or(RestError::NotFound)?)))
    }

    async fn series_of(state: &AppState, id: &str) -> Result<pacs_index::RestSeries, RestError> {
        state
            .archive
            .index()
            .rest_series(id)
            .await?
            .ok_or(RestError::NotFound)
    }

    async fn instance_of(state: &AppState, id: &str) -> Result<RestInstance, RestError> {
        state
            .archive
            .index()
            .rest_instance(id)
            .await?
            .ok_or(RestError::NotFound)
    }

    /// The patient of a study.
    #[utoipa::path(
        get,
        path = "/studies/{id}/patient",
        tag = "Studies",
        params(("id" = String, Path, description = "The study's identifier.")),
        responses(
            (status = 200, description = "The patient.", body = Patient),
            (status = 404, description = "No study has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn study_patient(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        patient_of_study(&state, &id).await
    }

    /// The study of a series.
    #[utoipa::path(
        get,
        path = "/series/{id}/study",
        tag = "Series",
        params(("id" = String, Path, description = "The series's identifier.")),
        responses(
            (status = 200, description = "The study.", body = Study),
            (status = 404, description = "No series has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn series_study(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        study(&state, &series_of(&state, &id).await?.parent_study).await
    }

    /// The patient of a series.
    #[utoipa::path(
        get,
        path = "/series/{id}/patient",
        tag = "Series",
        params(("id" = String, Path, description = "The series's identifier.")),
        responses(
            (status = 200, description = "The patient.", body = Patient),
            (status = 404, description = "No series has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn series_patient(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        patient_of_study(&state, &series_of(&state, &id).await?.parent_study).await
    }

    /// The series of an instance.
    #[utoipa::path(
        get,
        path = "/instances/{id}/series",
        tag = "Instances",
        params(("id" = String, Path, description = "The instance's identifier.")),
        responses(
            (status = 200, description = "The series.", body = Series),
            (status = 404, description = "No instance has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn instance_series(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        let instance = instance_of(&state, &id).await?;
        Ok(Json(series_json(
            series_of(&state, &instance.parent_series).await?,
        )))
    }

    /// The study of an instance.
    #[utoipa::path(
        get,
        path = "/instances/{id}/study",
        tag = "Instances",
        params(("id" = String, Path, description = "The instance's identifier.")),
        responses(
            (status = 200, description = "The study.", body = Study),
            (status = 404, description = "No instance has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn instance_study(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        study(&state, &instance_of(&state, &id).await?.parent_study).await
    }

    /// The patient of an instance.
    #[utoipa::path(
        get,
        path = "/instances/{id}/patient",
        tag = "Instances",
        params(("id" = String, Path, description = "The instance's identifier.")),
        responses(
            (status = 200, description = "The patient.", body = Patient),
            (status = 404, description = "No instance has this identifier; the body is empty.")
        )
    )]
    pub(crate) async fn instance_patient(
        Seen(state): Seen,
        Path(id): Path<String>,
    ) -> Result<Json<Value>, RestError> {
        patient_of_study(&state, &instance_of(&state, &id).await?.parent_study).await
    }
}
