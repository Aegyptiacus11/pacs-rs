//! The Clean Descriptors Option (PS3.15 E.3.5): free text is kept unless it holds what the
//! Profile removes elsewhere in the data set, the patient's names and identifiers, the
//! referring and performing people, the institution, and the words a project adds.

use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

/// The words that identify, lower-cased: every component of the names, the identifiers,
/// the birth date in a few common writings. Short components (initials, "DR") are left out,
/// so that they do not match inside ordinary words.
pub fn identifiers(object: &InMemDicomObject, extra: &[String]) -> Vec<String> {
    let mut words = Vec::new();
    let text = |tag| {
        object
            .element(tag)
            .ok()
            .and_then(|e| e.to_multi_str().ok().map(|v| v.to_vec()))
            .unwrap_or_default()
    };
    for tag in [
        tags::PATIENT_NAME,
        tags::OTHER_PATIENT_NAMES,
        tags::PATIENT_BIRTH_NAME,
        tags::PATIENT_MOTHER_BIRTH_NAME,
        tags::REFERRING_PHYSICIAN_NAME,
        tags::PERFORMING_PHYSICIAN_NAME,
        tags::OPERATORS_NAME,
        tags::NAME_OF_PHYSICIANS_READING_STUDY,
        tags::REQUESTING_PHYSICIAN,
    ] {
        for value in text(tag) {
            for part in value.split(['^', '=', ' ', ',']) {
                words.push(part.to_owned());
            }
        }
    }
    for tag in [
        tags::PATIENT_ID,
        // Other Patient IDs, retired but still sent
        dicom_core::Tag(0x0010, 0x1000),
        tags::ACCESSION_NUMBER,
        tags::INSTITUTION_NAME,
        tags::INSTITUTION_ADDRESS,
        tags::STATION_NAME,
    ] {
        words.extend(text(tag));
    }
    for birth in text(tags::PATIENT_BIRTH_DATE) {
        let b = birth.trim();
        if b.len() == 8 {
            let (y, m, d) = (&b[..4], &b[4..6], &b[6..]);
            words.extend([
                b.to_owned(),
                format!("{d}/{m}/{y}"),
                format!("{m}/{d}/{y}"),
                format!("{y}-{m}-{d}"),
                format!("{d}.{m}.{y}"),
            ]);
        }
    }
    words.extend(extra.iter().cloned());
    let mut words: Vec<String> = words
        .into_iter()
        .map(|w| w.trim_matches([' ', '\0']).to_lowercase())
        .filter(|w| w.chars().count() >= 3)
        .collect();
    words.sort();
    words.dedup();
    words
}

/// The value, unless it contains one of `identifiers` (as written, ignoring case).
pub fn clean_text(value: &str, identifiers: &[String]) -> Option<String> {
    let lower = value.to_lowercase();
    (!identifiers.iter().any(|w| lower.contains(w.as_str()))).then(|| value.to_owned())
}
