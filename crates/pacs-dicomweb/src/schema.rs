//! The answers of DICOMweb as types, for the API's description. The bodies themselves are
//! the standard's (PS3.18): DICOM JSON, and multipart of DICOM files or frames.

use serde::Deserialize;
use utoipa::ToSchema;

/// A data set in DICOM JSON (PS3.18 Annex F): its attributes by tag, each with its `vr` and
/// its `Value`.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(value_type = Object)]
pub struct Attributes(pub serde_json::Value);

/// A DICOMweb request refused: why.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Problem {
    pub error: String,
}

/// Bytes that are not JSON: a multipart body of DICOM files, or of frames.
#[derive(Debug, ToSchema)]
#[schema(value_type = String, format = Binary)]
pub struct Binary(pub Vec<u8>);
