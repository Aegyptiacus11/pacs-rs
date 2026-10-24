//! DICOM instances as pacs-rs stores and indexes them.
//!
//! An instance arrives as a DICOM Part 10 file (preamble optional). [`Instance::from_bytes`]
//! parses it and pulls out the identifiers the index is keyed on: study, series and SOP
//! instance UIDs, the SOP class and the stored transfer syntax.

use dicom_core::Tag;
use dicom_dictionary_std::tags;
use dicom_object::file::ReadPreamble;
use dicom_object::{DefaultDicomObject, OpenFileOptions};

#[cfg(feature = "synthetic")]
pub mod synthetic;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not a readable DICOM file: {0}")]
    Read(#[from] dicom_object::ReadError),
    #[error("missing required attribute {name} {tag}")]
    Missing { name: &'static str, tag: Tag },
    #[error("attribute {name} {tag} is not a valid UID: {value:?}")]
    InvalidUid {
        name: &'static str,
        tag: Tag,
        value: String,
    },
    #[error("attribute {name} {tag} is not a string: {source}")]
    NotAString {
        name: &'static str,
        tag: Tag,
        source: dicom_core::value::ConvertValueError,
    },
}

/// The identifiers of one stored instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceKey {
    pub study_instance_uid: String,
    pub series_instance_uid: String,
    pub sop_instance_uid: String,
    pub sop_class_uid: String,
    pub transfer_syntax_uid: String,
}

/// A parsed DICOM instance and its identifiers.
pub struct Instance {
    pub key: InstanceKey,
    pub object: DefaultDicomObject,
}

impl Instance {
    /// Parse a DICOM Part 10 file, with or without its 128-byte preamble.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let object = OpenFileOptions::new()
            .read_preamble(ReadPreamble::Auto)
            .from_reader(bytes)?;
        let key = InstanceKey {
            study_instance_uid: uid(&object, "StudyInstanceUID", tags::STUDY_INSTANCE_UID)?,
            series_instance_uid: uid(&object, "SeriesInstanceUID", tags::SERIES_INSTANCE_UID)?,
            sop_instance_uid: uid(&object, "SOPInstanceUID", tags::SOP_INSTANCE_UID)?,
            sop_class_uid: uid(&object, "SOPClassUID", tags::SOP_CLASS_UID)?,
            transfer_syntax_uid: trim_uid(object.meta().transfer_syntax()),
        };
        Ok(Instance { key, object })
    }

    /// The data set as DICOM JSON (PS3.18 F.2), file meta group excluded.
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        dicom_json::to_value(&*self.object)
    }
}

fn uid(object: &DefaultDicomObject, name: &'static str, tag: Tag) -> Result<String, Error> {
    let element = object
        .element_opt(tag)
        .ok()
        .flatten()
        .ok_or(Error::Missing { name, tag })?;
    let value = element
        .to_str()
        .map_err(|source| Error::NotAString { name, tag, source })?;
    let value = trim_uid(&value);
    if !is_valid_uid(&value) {
        return Err(Error::InvalidUid { name, tag, value });
    }
    Ok(value)
}

/// Whether `uid` has the shape of a DICOM UID: dot-separated runs of digits, at most 64
/// characters (PS3.5 9.1). Leading zeros in a component are tolerated, as real devices
/// emit them. Store keys and URLs are built from UIDs, so nothing else may get through.
pub fn is_valid_uid(uid: &str) -> bool {
    !uid.is_empty()
        && uid.len() <= 64
        && uid
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// UIDs are padded with NUL to an even length on the wire.
fn trim_uid(uid: &str) -> String {
    uid.trim_end_matches(['\0', ' ']).to_owned()
}
