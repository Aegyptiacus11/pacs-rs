//! What every instance pacs-rs makes itself shares (an Encapsulated PDF, a Key Object
//! Selection): the patient and study attributes of the study it joins, values put with the
//! character set they need, and the Part 10 file in Explicit VR Little Endian.

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};
use serde_json::{Map, Value};

use crate::Error;

const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";

pub(crate) fn text(map: &Map<String, Value>, key: &str) -> String {
    map.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

/// The Patient and General Study attributes, from the study's and its patient's main tags as
/// the index keeps them. A study with no Study Instance UID is refused.
pub(crate) fn patient_and_study(
    study: &Map<String, Value>,
    patient: &Map<String, Value>,
) -> Result<Vec<(Tag, VR, String)>, Error> {
    let study_uid = text(study, "StudyInstanceUID");
    if study_uid.is_empty() {
        return Err(Error::Missing {
            name: "StudyInstanceUID",
            tag: tags::STUDY_INSTANCE_UID,
        });
    }
    Ok(vec![
        (tags::PATIENT_NAME, VR::PN, text(patient, "PatientName")),
        (tags::PATIENT_ID, VR::LO, text(patient, "PatientID")),
        (
            tags::PATIENT_BIRTH_DATE,
            VR::DA,
            text(patient, "PatientBirthDate"),
        ),
        (tags::PATIENT_SEX, VR::CS, text(patient, "PatientSex")),
        (tags::STUDY_INSTANCE_UID, VR::UI, study_uid),
        (tags::STUDY_DATE, VR::DA, text(study, "StudyDate")),
        (tags::STUDY_TIME, VR::TM, text(study, "StudyTime")),
        (tags::STUDY_ID, VR::SH, text(study, "StudyID")),
        (
            tags::ACCESSION_NUMBER,
            VR::SH,
            text(study, "AccessionNumber"),
        ),
        (
            tags::REFERRING_PHYSICIAN_NAME,
            VR::PN,
            text(study, "ReferringPhysicianName"),
        ),
        (
            tags::STUDY_DESCRIPTION,
            VR::LO,
            text(study, "StudyDescription"),
        ),
    ])
}

/// Puts each value (an empty one as an empty element, as a type 2 attribute is), and UTF-8 as
/// the character set when one is not plain ASCII.
pub(crate) fn put_all(object: &mut InMemDicomObject, values: &[(Tag, VR, String)]) {
    for (tag, vr, value) in values {
        object.put(DataElement::new(
            *tag,
            *vr,
            if value.is_empty() {
                PrimitiveValue::Empty
            } else {
                PrimitiveValue::from(value.as_str())
            },
        ));
    }
    if values.iter().any(|(_, _, v)| !v.is_ascii()) {
        object.put(DataElement::new(
            tags::SPECIFIC_CHARACTER_SET,
            VR::CS,
            PrimitiveValue::from("ISO_IR 192"),
        ));
    }
}

/// The Part 10 file, Explicit VR Little Endian.
pub(crate) fn part10(
    object: InMemDicomObject,
    sop_class: &str,
    sop_instance: &str,
) -> Result<Vec<u8>, Error> {
    let file = object
        .with_meta(
            FileMetaTableBuilder::new()
                .transfer_syntax(EXPLICIT_VR_LE)
                .media_storage_sop_class_uid(sop_class)
                .media_storage_sop_instance_uid(sop_instance),
        )
        .map_err(|e| Error::Malformed(format!("no file meta: {e}")))?;
    let mut out = Vec::new();
    file.write_all(&mut out)
        .map_err(|e| Error::Malformed(format!("the instance could not be written: {e}")))?;
    Ok(out)
}
