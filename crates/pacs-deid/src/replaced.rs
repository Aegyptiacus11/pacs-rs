//! The way back from a de-identified copy, for a server that keeps it (write-back of
//! results to the original study): each replacement UID with the UID it replaced, and a
//! result made on the copy put back under the originals ([`reidentify`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use dicom_core::dictionary::{DataDictionary, DataDictionaryEntry};
use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, VR};
use dicom_dictionary_std::{StandardDataDictionary, tags};
use dicom_object::InMemDicomObject;
use serde_json::{Map, Value as JsonValue};

use crate::keys::Keys;

fn uids(object: &InMemDicomObject, out: &mut BTreeSet<String>) {
    for element in object.iter() {
        if let Some(items) = element.items() {
            for item in items {
                uids(item, out);
            }
        } else if element.vr() == VR::UI
            && let Ok(values) = element.to_multi_str()
        {
            out.extend(
                values
                    .iter()
                    .map(|v| v.trim_matches([' ', '\0']).to_owned())
                    .filter(|v| !v.is_empty()),
            );
        }
    }
}

/// Each UID of `deidentified` that replaces one of `original` (by `keys`), with the one it
/// replaced, ordered by the replacement. UIDs kept as they were are left out.
pub fn replaced_uids(
    original: &InMemDicomObject,
    deidentified: &InMemDicomObject,
    keys: &Keys,
) -> Vec<(String, String)> {
    let (mut before, mut after) = (BTreeSet::new(), BTreeSet::new());
    uids(original, &mut before);
    uids(deidentified, &mut after);
    let mut pairs = BTreeMap::new();
    for uid in before {
        let replacement = keys.uid(&uid);
        if replacement != uid && after.contains(&replacement) {
            pairs.insert(replacement, uid);
        }
    }
    pairs.into_iter().collect()
}

/// Every UID `object` holds, in sequences too: what to look up the originals of before
/// [`reidentify`].
pub fn uids_in(object: &InMemDicomObject) -> Vec<String> {
    let mut found = BTreeSet::new();
    uids(object, &mut found);
    found.into_iter().collect()
}

/// Why a result cannot be re-identified.
#[derive(Debug, thiserror::Error)]
pub enum ReidentifyError {
    /// An original main tag this crate cannot put back.
    #[error("the original attribute {0:?} is not one of the standard's keywords")]
    UnknownKeyword(String),
}

fn with_uids(object: &InMemDicomObject, originals: &HashMap<String, String>) -> InMemDicomObject {
    let mut out = InMemDicomObject::new_empty();
    for element in object.iter() {
        let (tag, vr) = (element.header().tag, element.vr());
        if let Some(items) = element.items() {
            let items: Vec<InMemDicomObject> = items
                .iter()
                .map(|item| with_uids(item, originals))
                .collect();
            out.put(DataElement::new(
                tag,
                VR::SQ,
                Value::Sequence(DataSetSequence::new(items, Length::UNDEFINED)),
            ));
        } else if vr == VR::UI
            && let Ok(values) = element.to_multi_str()
        {
            let values: Vec<String> = values
                .iter()
                .map(|v| {
                    let v = v.trim_matches([' ', '\0']);
                    originals.get(v).cloned().unwrap_or_else(|| v.to_owned())
                })
                .collect();
            out.put(DataElement::new(
                tag,
                vr,
                PrimitiveValue::Strs(values.into()),
            ));
        } else {
            out.put(element.clone());
        }
    }
    out
}

/// A result made on a de-identified copy, as it goes back to the hospital: every UID the
/// copy replaced (`originals`, replacement to original) is the original again, the patient
/// and study attributes are the originals (`patient` and `study`, main tags by keyword),
/// Patient Identity Removed is NO, and what the de-identification recorded of itself goes.
/// UIDs the result made itself (its own SOP Instance and Series) are kept.
pub fn reidentify(
    object: &InMemDicomObject,
    originals: &HashMap<String, String>,
    patient: &Map<String, JsonValue>,
    study: &Map<String, JsonValue>,
) -> Result<InMemDicomObject, ReidentifyError> {
    let mut out = with_uids(object, originals);
    for (keyword, value) in patient.iter().chain(study) {
        let entry = StandardDataDictionary
            .by_name(keyword)
            .ok_or_else(|| ReidentifyError::UnknownKeyword(keyword.clone()))?;
        let tag = entry.tag_range().inner();
        let vr = entry.vr().relaxed();
        let text = value.as_str().unwrap_or_default();
        let values: Vec<String> = if text.is_empty() {
            Vec::new()
        } else {
            text.split('\\').map(str::to_owned).collect()
        };
        out.put(DataElement::new(
            tag,
            vr,
            if values.is_empty() {
                PrimitiveValue::Empty
            } else {
                PrimitiveValue::Strs(values.into())
            },
        ));
    }
    out.put(DataElement::new(
        tags::PATIENT_IDENTITY_REMOVED,
        VR::CS,
        PrimitiveValue::from("NO"),
    ));
    for tag in [
        tags::DEIDENTIFICATION_METHOD,
        tags::DEIDENTIFICATION_METHOD_CODE_SEQUENCE,
        tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED,
    ] {
        out.remove_element(tag);
    }
    Ok(out)
}
