//! A study-level query of the Study Root Query/Retrieve Information Model (PS3.4 C.6.2.1.2):
//! the keys pacs-rs searches by, checked against the matching rules of PS3.4 C.2.2.2, the
//! Request Identifier they make (C.4.1.1.3.1), and the filtering an SCU applies to what
//! comes back (C.4.1.2.1).

use std::str::FromStr;

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;

/// Study Root Query/Retrieve Information Model - FIND (PS3.4 C.6.2.3).
pub const STUDY_ROOT_FIND: &str = "1.2.840.10008.5.1.4.1.2.2.1";

/// What to search a remote archive's studies by. Every key left out matches anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StudyQuery {
    /// Patient ID (LO): single value, or wild card matching with `*` and `?`.
    pub patient_id: Option<String>,
    /// Patient's Name (PN): single value or wild card matching.
    pub patient_name: Option<String>,
    /// Accession Number (SH): single value or wild card matching.
    pub accession_number: Option<String>,
    /// Study Date (DA): one date or a range.
    pub study_date: Option<DateRange>,
    /// Modalities in Study (CS): one modality, or a wild card pattern.
    pub modality: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{key}: {reason}")]
pub struct QueryError {
    pub key: &'static str,
    pub reason: String,
}

fn invalid(key: &'static str, reason: impl Into<String>) -> QueryError {
    QueryError {
        key,
        reason: reason.into(),
    }
}

/// A Study Date key: one date (Single Value Matching) or a range, open at either end
/// (Range Matching, PS3.4 C.2.2.2.5.1). Dates are `YYYYMMDD` (PS3.5 Table 6.2-1, DA).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DateRange {
    On(String),
    Between {
        from: Option<String>,
        to: Option<String>,
    },
}

impl FromStr for DateRange {
    type Err = QueryError;

    /// `YYYYMMDD`, `YYYYMMDD-YYYYMMDD`, `-YYYYMMDD` or `YYYYMMDD-`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let date = |d: &str| -> Result<Option<String>, QueryError> {
            if d.is_empty() {
                return Ok(None);
            }
            check_date(d)?;
            Ok(Some(d.to_owned()))
        };
        match s.split_once('-') {
            None => Ok(DateRange::On(date(s)?.ok_or_else(|| {
                invalid("StudyDate", "a date or a range is needed")
            })?)),
            Some((from, to)) => {
                let (from, to) = (date(from)?, date(to)?);
                match (&from, &to) {
                    (None, None) => Err(invalid("StudyDate", "a range needs at least one end")),
                    (Some(f), Some(t)) if f > t => Err(invalid(
                        "StudyDate",
                        format!("{f} is after {t}; a range goes from the earlier date"),
                    )),
                    _ => Ok(DateRange::Between { from, to }),
                }
            }
        }
    }
}

fn check_date(d: &str) -> Result<(), QueryError> {
    let bad = || invalid("StudyDate", format!("{d:?} is not a date written YYYYMMDD"));
    if d.len() != 8 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let year: u32 = d[..4].parse().map_err(|_| bad())?;
    let month: u32 = d[4..6].parse().map_err(|_| bad())?;
    let day: u32 = d[6..].parse().map_err(|_| bad())?;
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return Err(bad()),
    };
    if day == 0 || day > days {
        return Err(bad());
    }
    Ok(())
}

impl DateRange {
    /// The key's value: the date, or the two ends around "-".
    pub fn key(&self) -> String {
        match self {
            DateRange::On(d) => d.clone(),
            DateRange::Between { from, to } => format!(
                "{}-{}",
                from.as_deref().unwrap_or(""),
                to.as_deref().unwrap_or("")
            ),
        }
    }

    /// Whether `date` (DA, `YYYYMMDD`) is on or inside the range, ends included. A value
    /// that is not a date is not.
    pub fn contains(&self, date: &str) -> bool {
        if check_date(date).is_err() {
            return false;
        }
        match self {
            DateRange::On(d) => date == d,
            DateRange::Between { from, to } => {
                from.as_deref().is_none_or(|f| date >= f) && to.as_deref().is_none_or(|t| date <= t)
            }
        }
    }
}

/// A text key: one value, so no backslash (Multiple Value Matching needs extended
/// negotiation, C.2.2.2.8); no control characters; not the two quotation marks of Empty Value
/// Matching, which also needs extended negotiation (C.2.2.2.7).
fn check_text(key: &'static str, value: &str) -> Result<(), QueryError> {
    if value.trim().is_empty() {
        return Err(invalid(key, "empty; leave the key out to match anything"));
    }
    if value.contains('\\') {
        return Err(invalid(key, "one value only, no backslash"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(invalid(key, "control characters are not allowed"));
    }
    if value.trim() == "\"\"" {
        return Err(invalid(key, "empty value matching is not supported"));
    }
    Ok(())
}

impl StudyQuery {
    /// Check every key against the matching it is sent for. Modality is upper-cased, as CS
    /// values are (PS3.5 Table 6.2-1).
    pub fn checked(mut self) -> Result<Self, QueryError> {
        for (key, value) in [
            ("PatientID", &self.patient_id),
            ("PatientName", &self.patient_name),
            ("AccessionNumber", &self.accession_number),
            ("ModalitiesInStudy", &self.modality),
        ] {
            if let Some(value) = value {
                check_text(key, value)?;
            }
        }
        if let Some(modality) = &mut self.modality {
            *modality = modality.trim().to_ascii_uppercase();
            if !modality
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || " _*?".contains(c))
            {
                return Err(invalid(
                    "ModalitiesInStudy",
                    "a modality is letters, digits, space and underscore (CS)",
                ));
            }
        }
        Ok(self)
    }

    /// The Request Identifier (PS3.4 C.4.1.1.3.1): Query/Retrieve Level STUDY, each key
    /// given, and the Required Keys of Table C.6-5 and a few Optional ones empty (Universal
    /// Matching, C.2.2.2.3) so that their values come back. Specific Character Set is
    /// ISO_IR 192 (UTF-8) when a key is not ASCII, and absent otherwise.
    pub fn identifier(&self) -> InMemDicomObject {
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        let mut identifier = InMemDicomObject::new_empty();
        let mut put = |tag: Tag, vr: VR, value: String| {
            identifier.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
        };
        let any_non_ascii = [
            &self.patient_id,
            &self.patient_name,
            &self.accession_number,
            &self.modality,
        ]
        .iter()
        .any(|v| v.as_deref().is_some_and(|v| !v.is_ascii()));
        if any_non_ascii {
            put(tags::SPECIFIC_CHARACTER_SET, VR::CS, "ISO_IR 192".into());
        }
        put(
            tags::STUDY_DATE,
            VR::DA,
            self.study_date
                .as_ref()
                .map(DateRange::key)
                .unwrap_or_default(),
        );
        put(tags::STUDY_TIME, VR::TM, String::new());
        put(tags::ACCESSION_NUMBER, VR::SH, text(&self.accession_number));
        put(tags::QUERY_RETRIEVE_LEVEL, VR::CS, "STUDY".into());
        put(tags::MODALITIES_IN_STUDY, VR::CS, text(&self.modality));
        put(tags::REFERRING_PHYSICIAN_NAME, VR::PN, String::new());
        put(tags::STUDY_DESCRIPTION, VR::LO, String::new());
        put(tags::PATIENT_NAME, VR::PN, text(&self.patient_name));
        put(tags::PATIENT_ID, VR::LO, text(&self.patient_id));
        put(tags::PATIENT_BIRTH_DATE, VR::DA, String::new());
        put(tags::PATIENT_SEX, VR::CS, String::new());
        put(tags::STUDY_INSTANCE_UID, VR::UI, String::new());
        put(tags::STUDY_ID, VR::SH, String::new());
        put(tags::NUMBER_OF_STUDY_RELATED_SERIES, VR::IS, String::new());
        put(
            tags::NUMBER_OF_STUDY_RELATED_INSTANCES,
            VR::IS,
            String::new(),
        );
        identifier
    }

    /// Whether a Response Identifier meets every key, by the SCU's own filtering (PS3.4
    /// C.4.1.2.1: an SCP may not support an Optional Key). A key whose attribute came back
    /// absent or empty cannot be checked, and passes. Patient's Name is compared regardless
    /// of case (C.2.2.2.1.1 allows the SCP that); a multi-valued attribute passes when any
    /// value does (C.2.2.3).
    pub fn accepts(&self, found: &InMemDicomObject) -> bool {
        let values = |tag: Tag| -> Vec<String> {
            found
                .element(tag)
                .ok()
                .and_then(|e| e.to_multi_str().ok())
                .map(|v| {
                    v.iter()
                        .map(|s| s.trim_matches([' ', '\0']).to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        };
        let text_ok = |key: &Option<String>, tag: Tag, ignore_case: bool| {
            let Some(pattern) = key else { return true };
            let found = values(tag);
            found.is_empty()
                || found
                    .iter()
                    .any(|v| wildcard_match(pattern.trim(), v, ignore_case))
        };
        let date_ok = match &self.study_date {
            None => true,
            Some(range) => {
                let found = values(tags::STUDY_DATE);
                found.is_empty() || found.iter().any(|d| range.contains(d))
            }
        };
        date_ok
            && text_ok(&self.patient_id, tags::PATIENT_ID, false)
            && text_ok(&self.patient_name, tags::PATIENT_NAME, true)
            && text_ok(&self.accession_number, tags::ACCESSION_NUMBER, false)
            && text_ok(&self.modality, tags::MODALITIES_IN_STUDY, false)
    }
}

/// Wild Card Matching (PS3.4 C.2.2.2.4): `*` matches any run of characters, none included,
/// `?` any one character; without either it is Single Value Matching, an exact comparison.
pub fn wildcard_match(pattern: &str, value: &str, ignore_case: bool) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if ignore_case {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, v) = (fold(pattern), fold(value));
    // the usual linear matcher with one backtrack point for the last `*`
    let (mut pi, mut vi) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while vi < v.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == v[vi]) {
            pi += 1;
            vi += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, vi));
            pi += 1;
        } else if let Some((sp, sv)) = star {
            pi = sp + 1;
            vi = sv + 1;
            star = Some((sp, sv + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// A Response Identifier as DICOM JSON (PS3.18 F.2), values cleaned as pacs-rs's own
/// DICOMweb answers are.
pub fn to_json(identifier: &InMemDicomObject) -> serde_json::Value {
    let mut value = dicom_json::to_value(identifier).unwrap_or(serde_json::Value::Null);
    pacs_core::json::normalize(&mut value);
    value
}
