//! Key images as a Key Object Selection Document (PS3.3 A.35.4), the object PACS and viewers
//! exchange to say which images of a study matter: a container titled "Of Interest" (DCM
//! 113000, TID 2010), an optional description (DCM 113012), and one IMAGE reference each,
//! with the evidence listed by series (Current Requested Procedure Evidence Sequence).

use dicom_core::value::DataSetSequence;
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use serde_json::{Map, Value};

use crate::{Error, made};

/// Key Object Selection Document Storage.
pub const KEY_OBJECT_SELECTION: &str = "1.2.840.10008.5.1.4.1.1.88.59";

/// One image chosen: its SOP Class and Instance, its series, and its frame in a multi-frame one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyImage {
    pub sop_class: String,
    pub sop_instance: String,
    pub series: String,
    pub frame: Option<u32>,
}

pub struct NewKeyImages<'a> {
    pub study: &'a Map<String, Value>,
    pub patient: &'a Map<String, Value>,
    /// What the selection is about, said in its Key Object Description; empty for none.
    pub description: &'a str,
    pub series_uid: &'a str,
    pub instance_uid: &'a str,
    pub series_number: u32,
    /// When it is made, in UTC ([`crate::document::utc`]).
    pub date: &'a str,
    pub time: &'a str,
}

fn item(values: &[(Tag, VR, String)]) -> InMemDicomObject {
    let mut object = InMemDicomObject::new_empty();
    for (tag, vr, value) in values {
        object.put(DataElement::new(
            *tag,
            *vr,
            PrimitiveValue::from(value.as_str()),
        ));
    }
    object
}

fn sequence(tag: Tag, items: Vec<InMemDicomObject>) -> DataElement<InMemDicomObject> {
    DataElement::new(tag, VR::SQ, DataSetSequence::from(items))
}

fn code(value: &str, meaning: &str) -> DataElement<InMemDicomObject> {
    sequence(
        tags::CONCEPT_NAME_CODE_SEQUENCE,
        vec![item(&[
            (tags::CODE_VALUE, VR::SH, value.into()),
            (tags::CODING_SCHEME_DESIGNATOR, VR::SH, "DCM".into()),
            (tags::CODE_MEANING, VR::LO, meaning.into()),
        ])],
    )
}

/// The Part 10 file of a new Key Object Selection of `images` (each once, in their order).
pub fn key_object_selection(new: &NewKeyImages, images: &[KeyImage]) -> Result<Vec<u8>, Error> {
    let mut chosen: Vec<&KeyImage> = Vec::new();
    for image in images {
        if image.sop_class.is_empty() || image.sop_instance.is_empty() || image.series.is_empty() {
            return Err(Error::Malformed(format!(
                "a key image without its SOP Class, SOP Instance or Series UID: {image:?}"
            )));
        }
        if !chosen.contains(&image) {
            chosen.push(image);
        }
    }
    if chosen.is_empty() {
        return Err(Error::Malformed("no key image".into()));
    }
    let mut values = made::patient_and_study(new.study, new.patient)?;
    let study_uid = values
        .iter()
        .find(|(t, _, _)| *t == tags::STUDY_INSTANCE_UID)
        .map(|v| v.2.clone())
        .unwrap_or_default();
    let description = new.description.trim();
    values.extend([
        (tags::SOP_CLASS_UID, VR::UI, KEY_OBJECT_SELECTION.to_owned()),
        (tags::SOP_INSTANCE_UID, VR::UI, new.instance_uid.to_owned()),
        (tags::MODALITY, VR::CS, "KO".to_owned()),
        (tags::SERIES_INSTANCE_UID, VR::UI, new.series_uid.to_owned()),
        (tags::SERIES_NUMBER, VR::IS, new.series_number.to_string()),
        (tags::SERIES_DESCRIPTION, VR::LO, "Key images".to_owned()),
        (tags::MANUFACTURER, VR::LO, "pacs-rs".to_owned()),
        (tags::INSTANCE_NUMBER, VR::IS, "1".to_owned()),
        (tags::CONTENT_DATE, VR::DA, new.date.to_owned()),
        (tags::CONTENT_TIME, VR::TM, new.time.to_owned()),
        (tags::INSTANCE_CREATION_DATE, VR::DA, new.date.to_owned()),
        (tags::INSTANCE_CREATION_TIME, VR::TM, new.time.to_owned()),
        (tags::TIMEZONE_OFFSET_FROM_UTC, VR::SH, "+0000".to_owned()),
        (tags::VALUE_TYPE, VR::CS, "CONTAINER".to_owned()),
        (tags::CONTINUITY_OF_CONTENT, VR::CS, "SEPARATE".to_owned()),
    ]);
    if !description.is_ascii() {
        values.push((
            tags::SPECIFIC_CHARACTER_SET,
            VR::CS,
            "ISO_IR 192".to_owned(),
        ));
    }
    let mut object = InMemDicomObject::new_empty();
    made::put_all(&mut object, &values);
    object.put(DataElement::new(
        tags::REFERENCED_PERFORMED_PROCEDURE_STEP_SEQUENCE,
        VR::SQ,
        DataSetSequence::<InMemDicomObject>::empty(),
    ));
    // the document's title: Of Interest
    object.put(code("113000", "Of Interest"));
    object.put(sequence(
        tags::CONTENT_TEMPLATE_SEQUENCE,
        vec![item(&[
            (tags::MAPPING_RESOURCE, VR::CS, "DCMR".into()),
            (tags::TEMPLATE_IDENTIFIER, VR::CS, "2010".into()),
        ])],
    ));
    // the evidence: every image, by series
    let mut series: Vec<(&str, Vec<&KeyImage>)> = Vec::new();
    for image in &chosen {
        match series.iter_mut().find(|(s, _)| *s == image.series) {
            Some((_, list)) => list.push(image),
            None => series.push((&image.series, vec![image])),
        }
    }
    let reference = |image: &KeyImage, frame: bool| {
        let mut values = vec![
            (
                tags::REFERENCED_SOP_CLASS_UID,
                VR::UI,
                image.sop_class.clone(),
            ),
            (
                tags::REFERENCED_SOP_INSTANCE_UID,
                VR::UI,
                image.sop_instance.clone(),
            ),
        ];
        if let (true, Some(n)) = (frame, image.frame) {
            values.push((tags::REFERENCED_FRAME_NUMBER, VR::IS, n.to_string()));
        }
        item(&values)
    };
    let mut evidence = item(&[(tags::STUDY_INSTANCE_UID, VR::UI, study_uid)]);
    evidence.put(sequence(
        tags::REFERENCED_SERIES_SEQUENCE,
        series
            .iter()
            .map(|(uid, images)| {
                let mut s = item(&[(tags::SERIES_INSTANCE_UID, VR::UI, (*uid).to_owned())]);
                let mut instances: Vec<&KeyImage> = Vec::new();
                for image in images {
                    if !instances
                        .iter()
                        .any(|i| i.sop_instance == image.sop_instance)
                    {
                        instances.push(image);
                    }
                }
                s.put(sequence(
                    tags::REFERENCED_SOP_SEQUENCE,
                    instances.into_iter().map(|i| reference(i, false)).collect(),
                ));
                s
            })
            .collect(),
    ));
    object.put(sequence(
        tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE,
        vec![evidence],
    ));
    // the content: the description, then each image
    let mut content = Vec::new();
    if !description.is_empty() {
        let mut text = item(&[
            (tags::RELATIONSHIP_TYPE, VR::CS, "CONTAINS".into()),
            (tags::VALUE_TYPE, VR::CS, "TEXT".into()),
            (
                tags::TEXT_VALUE,
                VR::UT,
                description.chars().take(1024).collect(),
            ),
        ]);
        text.put(code("113012", "Key Object Description"));
        content.push(text);
    }
    for image in &chosen {
        let mut entry = item(&[
            (tags::RELATIONSHIP_TYPE, VR::CS, "CONTAINS".into()),
            (tags::VALUE_TYPE, VR::CS, "IMAGE".into()),
        ]);
        entry.put(sequence(
            tags::REFERENCED_SOP_SEQUENCE,
            vec![reference(image, true)],
        ));
        content.push(entry);
    }
    object.put(sequence(tags::CONTENT_SEQUENCE, content));
    made::part10(object, KEY_OBJECT_SELECTION, new.instance_uid)
}

/// The images a Key Object Selection names, in its order, with their series from its
/// evidence; `None` for an instance that is no Key Object Selection.
pub fn key_images_of(object: &InMemDicomObject) -> Option<Vec<KeyImage>> {
    let s = |o: &InMemDicomObject, tag| {
        o.element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|v| v.trim_end_matches(['\0', ' ']).to_owned())
    };
    if s(object, tags::SOP_CLASS_UID)? != KEY_OBJECT_SELECTION {
        return None;
    }
    let items = |o: &InMemDicomObject, tag| {
        o.element(tag)
            .ok()
            .and_then(|e| e.items().map(|i| i.to_vec()))
            .unwrap_or_default()
    };
    let mut series_of = std::collections::HashMap::new();
    for study in items(object, tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE) {
        for series in items(&study, tags::REFERENCED_SERIES_SEQUENCE) {
            let uid = s(&series, tags::SERIES_INSTANCE_UID).unwrap_or_default();
            for sop in items(&series, tags::REFERENCED_SOP_SEQUENCE) {
                series_of.insert(
                    s(&sop, tags::REFERENCED_SOP_INSTANCE_UID).unwrap_or_default(),
                    uid.clone(),
                );
            }
        }
    }
    let mut out = Vec::new();
    for entry in items(object, tags::CONTENT_SEQUENCE) {
        if s(&entry, tags::VALUE_TYPE).as_deref() != Some("IMAGE") {
            continue;
        }
        for sop in items(&entry, tags::REFERENCED_SOP_SEQUENCE) {
            let sop_instance = s(&sop, tags::REFERENCED_SOP_INSTANCE_UID).unwrap_or_default();
            out.push(KeyImage {
                sop_class: s(&sop, tags::REFERENCED_SOP_CLASS_UID).unwrap_or_default(),
                series: series_of.get(&sop_instance).cloned().unwrap_or_default(),
                frame: s(&sop, tags::REFERENCED_FRAME_NUMBER).and_then(|n| n.trim().parse().ok()),
                sop_instance,
            });
        }
    }
    Some(out)
}
