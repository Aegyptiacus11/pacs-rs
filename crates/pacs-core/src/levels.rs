//! Which attributes belong to which level of the study / series / instance hierarchy.
//!
//! The lists are the stored attributes of the QIDO-RS search response payloads, PS3.18
//! Tables 10.6.3-3 (study), 10.6.3-4 (series) and 10.6.3-5 (instance), plus Specific
//! Character Set and Study Description. Attributes the server computes - Modalities in
//! Study, the Number of ... Related counts, Retrieve URL, Instance Availability - are not
//! here.

use dicom_core::Tag;
use dicom_dictionary_std::tags;

pub const STUDY: &[Tag] = &[
    tags::SPECIFIC_CHARACTER_SET,
    tags::STUDY_DATE,
    tags::STUDY_TIME,
    tags::ACCESSION_NUMBER,
    tags::REFERRING_PHYSICIAN_NAME,
    tags::TIMEZONE_OFFSET_FROM_UTC,
    tags::STUDY_DESCRIPTION,
    tags::PATIENT_NAME,
    tags::PATIENT_ID,
    tags::PATIENT_BIRTH_DATE,
    tags::PATIENT_SEX,
    tags::STUDY_INSTANCE_UID,
    tags::STUDY_ID,
];

pub const SERIES: &[Tag] = &[
    tags::SPECIFIC_CHARACTER_SET,
    tags::MODALITY,
    tags::TIMEZONE_OFFSET_FROM_UTC,
    tags::SERIES_DESCRIPTION,
    tags::SERIES_INSTANCE_UID,
    tags::SERIES_NUMBER,
    tags::PERFORMED_PROCEDURE_STEP_START_DATE,
    tags::PERFORMED_PROCEDURE_STEP_START_TIME,
    tags::REQUEST_ATTRIBUTES_SEQUENCE,
];

pub const INSTANCE: &[Tag] = &[
    tags::SPECIFIC_CHARACTER_SET,
    tags::SOP_CLASS_UID,
    tags::SOP_INSTANCE_UID,
    tags::TIMEZONE_OFFSET_FROM_UTC,
    tags::INSTANCE_NUMBER,
    tags::ROWS,
    tags::COLUMNS,
    tags::BITS_ALLOCATED,
    tags::NUMBER_OF_FRAMES,
];

/// Pixel Data and its float variants: never inlined in metadata.
pub const BULK_PIXEL_DATA: &[Tag] = &[
    tags::FLOAT_PIXEL_DATA,
    tags::DOUBLE_FLOAT_PIXEL_DATA,
    tags::PIXEL_DATA,
];
