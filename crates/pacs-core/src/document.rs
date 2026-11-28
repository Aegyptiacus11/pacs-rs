//! A PDF made into a DICOM instance of a study: Encapsulated PDF Storage (PS3.3 A.45.1), as a
//! report system or a scanned request would add one. The patient and study come from the study
//! it joins; the series is new, of modality DOC.
//!
//! Modules written: Patient and General Study (the study's own values), Encapsulated Document
//! Series, General Equipment, SC Equipment (conversion `WSD`, a workstation), Encapsulated
//! Document (C.24.2) and SOP Common. Explicit VR Little Endian.

use dicom_core::value::DataSetSequence;
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};
use serde_json::{Map, Value};

use crate::Error;

/// Encapsulated PDF Storage.
pub const ENCAPSULATED_PDF: &str = "1.2.840.10008.5.1.4.1.1.104.1";
const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

/// What a new document needs besides its PDF.
pub struct NewDocument<'a> {
    /// The study's main tags and its patient's, as the index keeps them (`MainDicomTags` and
    /// `PatientMainDicomTags` of `GET /studies/{id}`).
    pub study: &'a Map<String, Value>,
    pub patient: &'a Map<String, Value>,
    /// Document Title, and the series' description (cut to 64 characters there).
    pub title: &'a str,
    pub series_uid: &'a str,
    pub instance_uid: &'a str,
    /// The series' number in its study.
    pub series_number: u32,
    /// When it is made, in UTC ([`utc`]): `YYYYMMDD`, `HHMMSS`. The instance says so with
    /// Timezone Offset From UTC `+0000`.
    pub date: &'a str,
    pub time: &'a str,
}

/// A moment as a DICOM date and time in UTC: `("20261127", "224000")`. Before 1970, 1970.
pub fn utc(at: std::time::SystemTime) -> (String, String) {
    let seconds = at
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (days, of_day) = ((seconds / 86_400) as i64, seconds % 86_400);
    // the date `days` after 1970-01-01, proleptic Gregorian (Howard Hinnant's civil_from_days)
    let z = days + 719_468;
    let (era, day_of_era) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        format!("{year:04}{month:02}{day:02}"),
        format!(
            "{:02}{:02}{:02}",
            of_day / 3600,
            of_day % 3600 / 60,
            of_day % 60
        ),
    )
}

/// Why `pdf` cannot be a document, if it cannot.
pub fn not_a_pdf(pdf: &[u8]) -> Option<String> {
    if pdf.is_empty() {
        return Some("the body is empty".into());
    }
    // PDF 1.0 to 2.0 begin with this header (ISO 32000-2 7.5.2)
    if !pdf.starts_with(b"%PDF-") {
        return Some("not a PDF: it does not begin with %PDF-".into());
    }
    if pdf.len() as u64 > u32::MAX as u64 - 1 {
        return Some("too large for a DICOM element".into());
    }
    None
}

/// The Part 10 file of a new Encapsulated PDF instance.
pub fn encapsulated_pdf(new: &NewDocument, pdf: &[u8]) -> Result<Vec<u8>, Error> {
    if let Some(why) = not_a_pdf(pdf) {
        return Err(Error::Malformed(why));
    }
    let mut object = InMemDicomObject::new_empty();
    let mut put = |tag: Tag, vr: VR, value: &str| {
        object.put(DataElement::new(
            tag,
            vr,
            if value.is_empty() {
                PrimitiveValue::Empty
            } else {
                PrimitiveValue::from(value)
            },
        ));
    };
    let text = |map: &Map<String, Value>, key: &str| {
        map.get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let study_uid = text(new.study, "StudyInstanceUID");
    if study_uid.is_empty() {
        return Err(Error::Missing {
            name: "StudyInstanceUID",
            tag: tags::STUDY_INSTANCE_UID,
        });
    }
    let title: String = new.title.trim().chars().take(1024).collect();
    let description = title
        .chars()
        .take(64)
        .collect::<String>()
        .trim_end()
        .to_owned();
    let values = [
        (tags::PATIENT_NAME, VR::PN, text(new.patient, "PatientName")),
        (tags::PATIENT_ID, VR::LO, text(new.patient, "PatientID")),
        (
            tags::PATIENT_BIRTH_DATE,
            VR::DA,
            text(new.patient, "PatientBirthDate"),
        ),
        (tags::PATIENT_SEX, VR::CS, text(new.patient, "PatientSex")),
        (tags::STUDY_INSTANCE_UID, VR::UI, study_uid),
        (tags::STUDY_DATE, VR::DA, text(new.study, "StudyDate")),
        (tags::STUDY_TIME, VR::TM, text(new.study, "StudyTime")),
        (tags::STUDY_ID, VR::SH, text(new.study, "StudyID")),
        (
            tags::ACCESSION_NUMBER,
            VR::SH,
            text(new.study, "AccessionNumber"),
        ),
        (
            tags::REFERRING_PHYSICIAN_NAME,
            VR::PN,
            text(new.study, "ReferringPhysicianName"),
        ),
        (
            tags::STUDY_DESCRIPTION,
            VR::LO,
            text(new.study, "StudyDescription"),
        ),
        (tags::SOP_CLASS_UID, VR::UI, ENCAPSULATED_PDF.to_owned()),
        (tags::SOP_INSTANCE_UID, VR::UI, new.instance_uid.to_owned()),
        (tags::MODALITY, VR::CS, "DOC".to_owned()),
        (tags::SERIES_INSTANCE_UID, VR::UI, new.series_uid.to_owned()),
        (tags::SERIES_NUMBER, VR::IS, new.series_number.to_string()),
        (tags::SERIES_DESCRIPTION, VR::LO, description),
        (tags::MANUFACTURER, VR::LO, "pacs-rs".to_owned()),
        (tags::CONVERSION_TYPE, VR::CS, "WSD".to_owned()),
        (tags::INSTANCE_NUMBER, VR::IS, "1".to_owned()),
        (tags::CONTENT_DATE, VR::DA, new.date.to_owned()),
        (tags::CONTENT_TIME, VR::TM, new.time.to_owned()),
        (tags::INSTANCE_CREATION_DATE, VR::DA, new.date.to_owned()),
        (tags::INSTANCE_CREATION_TIME, VR::TM, new.time.to_owned()),
        (tags::TIMEZONE_OFFSET_FROM_UTC, VR::SH, "+0000".to_owned()),
        (tags::ACQUISITION_DATE_TIME, VR::DT, String::new()),
        // whether the document identifies the patient is not known: said so, the safe way
        (tags::BURNED_IN_ANNOTATION, VR::CS, "YES".to_owned()),
        (tags::DOCUMENT_TITLE, VR::ST, title),
        (
            tags::MIME_TYPE_OF_ENCAPSULATED_DOCUMENT,
            VR::LO,
            "application/pdf".to_owned(),
        ),
    ];
    let any_wide = values.iter().any(|(_, _, v)| !v.is_ascii());
    for (tag, vr, value) in &values {
        put(*tag, *vr, value);
    }
    if any_wide {
        put(tags::SPECIFIC_CHARACTER_SET, VR::CS, "ISO_IR 192");
    }
    object.put(DataElement::new(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        VR::SQ,
        DataSetSequence::<InMemDicomObject>::empty(),
    ));
    // an odd length is padded with a NUL to an even one; the length says where the PDF ends
    let mut document = pdf.to_vec();
    if document.len() % 2 == 1 {
        document.push(0);
    }
    object.put(DataElement::new(
        tags::ENCAPSULATED_DOCUMENT,
        VR::OB,
        PrimitiveValue::from(document),
    ));
    object.put(DataElement::new(
        tags::ENCAPSULATED_DOCUMENT_LENGTH,
        VR::UL,
        PrimitiveValue::from(pdf.len() as u32),
    ));
    let file = object
        .with_meta(
            FileMetaTableBuilder::new()
                .transfer_syntax(EXPLICIT_VR_LE)
                .media_storage_sop_class_uid(ENCAPSULATED_PDF)
                .media_storage_sop_instance_uid(new.instance_uid),
        )
        .map_err(|e| Error::Malformed(format!("no file meta: {e}")))?;
    let mut out = Vec::new();
    file.write_all(&mut out)
        .map_err(|e| Error::Malformed(format!("the document could not be written: {e}")))?;
    Ok(out)
}

/// The PDF of an Encapsulated PDF instance, without its padding; `None` for any other.
pub fn pdf_of(object: &InMemDicomObject) -> Option<Vec<u8>> {
    let mime = object
        .element(tags::MIME_TYPE_OF_ENCAPSULATED_DOCUMENT)
        .ok()?
        .to_str()
        .ok()?;
    if mime.trim_end_matches(['\0', ' ']) != "application/pdf" {
        return None;
    }
    let mut bytes = object
        .element(tags::ENCAPSULATED_DOCUMENT)
        .ok()?
        .to_bytes()
        .ok()?
        .into_owned();
    match object
        .element(tags::ENCAPSULATED_DOCUMENT_LENGTH)
        .ok()
        .and_then(|e| e.to_int::<u32>().ok())
    {
        Some(n) if (n as usize) <= bytes.len() => bytes.truncate(n as usize),
        // without its length (it is required only when padded), the NUL padding goes
        _ => {
            if bytes.last() == Some(&0) {
                bytes.pop();
            }
        }
    }
    Some(bytes)
}
