//! Which instances go to quarantine instead of being de-identified ([`Quarantine`]).

use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

use crate::profile::Quarantine;

/// Why `object` must not be de-identified automatically, if it must not.
pub fn reason(object: &InMemDicomObject, rules: &Quarantine) -> Option<String> {
    let text = |tag| {
        object
            .element(tag)
            .ok()
            .and_then(|e| e.to_multi_str().ok().map(|v| v.to_vec()))
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim_matches([' ', '\0']).to_owned())
            .collect::<Vec<_>>()
    };
    if let Some(class) = text(tags::SOP_CLASS_UID).first() {
        let listed = rules.sop_classes.iter().any(|rule| {
            if rule.ends_with('.') {
                class.starts_with(rule.as_str())
            } else {
                class == rule
            }
        });
        if listed {
            return Some(format!("SOP Class {class} is quarantined by the profile"));
        }
    }
    let yes = |tag| {
        text(tag)
            .first()
            .is_some_and(|v| v.eq_ignore_ascii_case("YES"))
    };
    if rules.burned_in_annotation && yes(tags::BURNED_IN_ANNOTATION) {
        return Some("Burned In Annotation is YES".into());
    }
    if rules.recognizable_visual_features && yes(tags::RECOGNIZABLE_VISUAL_FEATURES) {
        return Some("Recognizable Visual Features is YES".into());
    }
    let image_type = text(tags::IMAGE_TYPE);
    if let Some(value) = rules
        .image_types
        .iter()
        .find(|t| image_type.iter().any(|v| v.eq_ignore_ascii_case(t)))
    {
        return Some(format!("Image Type has {value}"));
    }
    None
}
