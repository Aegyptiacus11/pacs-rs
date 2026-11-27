//! The answers of the REST API as types, for its description (`router_with_api`).
//!
//! The handlers build their answers as JSON values; these types say what is in them. They
//! refuse a field they do not name, and `tests/rest.rs` reads the handlers' real answers
//! into them, so a field added, renamed or dropped in a handler fails a test until it is
//! described here.

use std::collections::BTreeMap;

use serde::Deserialize;
use utoipa::ToSchema;

/// A patient: the studies that share a Patient ID.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Patient {
    /// The patient's identifier: the SHA-1 of the Patient ID.
    #[serde(rename = "ID")]
    pub id: String,
    /// Always `false`: pacs-rs never removes a patient to make room.
    pub is_protected: bool,
    /// Always `false` for now.
    pub is_stable: bool,
    /// Its labels, sorted (`/{level}/{id}/labels`).
    pub labels: Vec<String>,
    /// When something under the patient last changed, as `YYYYMMDDTHHMMSS` in UTC.
    pub last_update: String,
    /// Patient ID, Patient's Name, Birth Date and Sex, where the files give them.
    pub main_dicom_tags: BTreeMap<String, String>,
    /// The identifiers of the patient's studies.
    pub studies: Vec<String>,
    /// `Patient`.
    pub r#type: String,
}

/// A study.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Study {
    /// The study's identifier: the SHA-1 of `PatientID|StudyInstanceUID`.
    #[serde(rename = "ID")]
    pub id: String,
    /// Always `false` for now.
    pub is_stable: bool,
    /// Its labels, sorted (`/{level}/{id}/labels`).
    pub labels: Vec<String>,
    /// When something under the study last changed, as `YYYYMMDDTHHMMSS` in UTC.
    pub last_update: String,
    /// Study Instance UID, Date, Time, ID and Description, Accession Number and Referring
    /// Physician's Name, where the files give them.
    pub main_dicom_tags: BTreeMap<String, String>,
    /// The identifier of the study's patient.
    pub parent_patient: String,
    /// The patient's attributes, repeated here to save a request.
    pub patient_main_dicom_tags: BTreeMap<String, String>,
    /// The identifiers of the study's series.
    pub series: Vec<String>,
    /// `Study`.
    pub r#type: String,
}

/// A series.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Series {
    /// Always `null`: pacs-rs does not work out how many instances a series should have.
    #[schema(required)]
    pub expected_number_of_instances: Option<i64>,
    /// The series' identifier: the SHA-1 of `PatientID|StudyInstanceUID|SeriesInstanceUID`.
    #[serde(rename = "ID")]
    pub id: String,
    /// The identifiers of the series' instances, in Instance Number order.
    pub instances: Vec<String>,
    /// Always `false` for now.
    pub is_stable: bool,
    /// Its labels, sorted (`/{level}/{id}/labels`).
    pub labels: Vec<String>,
    /// When something in the series last changed, as `YYYYMMDDTHHMMSS` in UTC.
    pub last_update: String,
    /// Series Instance UID, Number and Description, and Modality, where the files give them.
    pub main_dicom_tags: BTreeMap<String, String>,
    /// The identifier of the series' study.
    pub parent_study: String,
    /// Always `Unknown`, for the reason of `ExpectedNumberOfInstances`.
    pub status: String,
    /// `Series`.
    pub r#type: String,
}

/// An instance: one stored DICOM file.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Instance {
    /// Bytes of the file as stored, which is smaller than the file sent when pacs-rs
    /// compressed it.
    pub file_size: i64,
    /// The instance's identifier: the SHA-1 of
    /// `PatientID|StudyInstanceUID|SeriesInstanceUID|SOPInstanceUID`.
    #[serde(rename = "ID")]
    pub id: String,
    /// The Instance Number; left out when the file has none.
    pub index_in_series: Option<i64>,
    /// Its labels, sorted (`/{level}/{id}/labels`).
    pub labels: Vec<String>,
    /// SOP Instance UID, Instance Number and Image Position (Patient), where the file gives
    /// them.
    pub main_dicom_tags: BTreeMap<String, String>,
    /// The identifier of the instance's series.
    pub parent_series: String,
    /// `Instance`.
    pub r#type: String,
}

/// How much a patient or a study holds. The sizes are of the files as stored; the four
/// sizes are the same number, kept under the names the reference server gives them.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Statistics {
    pub count_instances: i64,
    pub count_series: i64,
    /// For a patient only.
    pub count_studies: Option<i64>,
    /// Bytes, as a string.
    pub dicom_disk_size: String,
    /// Whole mebibytes.
    #[serde(rename = "DicomDiskSizeMB")]
    pub dicom_disk_size_mb: i64,
    pub dicom_uncompressed_size: String,
    #[serde(rename = "DicomUncompressedSizeMB")]
    pub dicom_uncompressed_size_mb: i64,
    pub disk_size: String,
    #[serde(rename = "DiskSizeMB")]
    pub disk_size_mb: i64,
    pub uncompressed_size: String,
    #[serde(rename = "UncompressedSizeMB")]
    pub uncompressed_size_mb: i64,
}

/// What became of a file sent to `POST /instances`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Stored {
    /// The instance's identifier.
    #[serde(rename = "ID")]
    pub id: String,
    pub parent_patient: String,
    pub parent_series: String,
    pub parent_study: String,
    /// The instance's address: `/instances/{ID}`.
    pub path: String,
    /// `Success` when the file was stored, `AlreadyStored` when an instance with its SOP
    /// Instance UID was there already and was kept.
    pub status: String,
}

/// The closest level still there after a deletion.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Ancestor {
    /// Its identifier.
    #[serde(rename = "ID")]
    pub id: String,
    /// Its address, such as `/series/{ID}`.
    pub path: String,
    /// `Patient`, `Study` or `Series`.
    pub r#type: String,
}

/// The answer to a deletion.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Deleted {
    /// `null` when the deletion left the patient with nothing, so that it went too.
    #[schema(required)]
    pub remaining_ancestor: Option<Ancestor>,
}

/// Why a request was refused. An unknown identifier is a 404 with no body instead.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, rename_all = "PascalCase")]
pub struct Error {
    /// The status in words, such as `Bad Request`.
    pub http_error: String,
    /// The status as a number.
    pub http_status: u16,
    /// What kind of failure, such as `Bad file format` or `Not supported yet`.
    pub message: String,
    /// What exactly was wrong with this request.
    pub details: Option<String>,
}

/// Identifiers, or with `expand` the patients themselves.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum PatientList {
    Ids(Vec<String>),
    Expanded(Vec<Patient>),
}

/// Identifiers, or with `expand` the studies themselves.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum StudyList {
    Ids(Vec<String>),
    Expanded(Vec<Study>),
}

/// What a search found: identifiers, or with `Expand` the resources of the level searched.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(untagged)]
pub enum Found {
    Ids(Vec<String>),
    Patients(Vec<Patient>),
    Studies(Vec<Study>),
    Series(Vec<Series>),
    Instances(Vec<Instance>),
}

/// Bytes that are not JSON: a DICOM file, or a ZIP.
#[derive(Debug, ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct Binary(pub Vec<u8>);
