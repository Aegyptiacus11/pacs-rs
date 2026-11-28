//! A patient's attributes changed in an instance, as modifying a patient does to every file of
//! theirs: Patient's Name, Patient ID, Patient's Birth Date and Patient's Sex, each checked
//! against its value representation (PS3.5 6.2) before any file is touched. The rest of the
//! file, its pixel data, transfer syntax and UIDs included, stays as it was.

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;

use crate::{Error, Instance};

/// New values for a patient's attributes: `None` leaves one as it is, an empty string removes
/// its value (Patient ID excepted: a patient is found by it, so it may not be emptied).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PatientChange {
    /// Patient's Name, as DICOM writes it: `FAMILY^GIVEN^MIDDLE^PREFIX^SUFFIX`.
    pub name: Option<String>,
    pub id: Option<String>,
    /// Patient's Birth Date, `YYYYMMDD`.
    pub birth_date: Option<String>,
    /// Patient's Sex: `M`, `F` or `O`.
    pub sex: Option<String>,
}

impl PatientChange {
    /// Whether it changes nothing.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.id.is_none() && self.birth_date.is_none() && self.sex.is_none()
    }

    /// Why these values cannot be written, if they cannot: each must fit its value
    /// representation.
    pub fn invalid(&self) -> Option<String> {
        if self.is_empty() {
            return Some(
                "nothing to change: give PatientName, PatientID, PatientBirthDate or PatientSex"
                    .into(),
            );
        }
        if let Some(name) = &self.name {
            // PN: up to five components a group, 64 characters a group (PS3.5 Table 6.2-1)
            if name.contains(['\\', '\n', '\r']) {
                return Some("PatientName has a backslash or a line break".into());
            }
            for group in name.split('=') {
                if group.chars().count() > 64 {
                    return Some("PatientName is longer than 64 characters".into());
                }
                if group.split('^').count() > 5 {
                    return Some("PatientName has more than five components".into());
                }
            }
        }
        if let Some(id) = &self.id {
            // LO: 64 characters, no backslash, no control character
            if id.trim().is_empty() {
                return Some("PatientID may not be empty".into());
            }
            if id.chars().count() > 64 {
                return Some("PatientID is longer than 64 characters".into());
            }
            if id.contains('\\') || id.chars().any(char::is_control) {
                return Some("PatientID has a backslash or a control character".into());
            }
        }
        if let Some(date) = self.birth_date.as_deref().filter(|d| !d.is_empty())
            && !valid_date(date)
        {
            return Some(format!("PatientBirthDate {date} is not a date as YYYYMMDD"));
        }
        if let Some(sex) = self.sex.as_deref().filter(|s| !s.is_empty())
            && !matches!(sex, "M" | "F" | "O")
        {
            return Some(format!("PatientSex {sex} is not M, F or O"));
        }
        None
    }

    /// The values as text, for the audit trail and the trash.
    pub fn values(&self) -> impl Iterator<Item = (&'static str, &str)> {
        [
            ("PatientName", &self.name),
            ("PatientID", &self.id),
            ("PatientBirthDate", &self.birth_date),
            ("PatientSex", &self.sex),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.as_deref().map(|v| (k, v)))
    }
}

/// A calendar date as DA (PS3.5 6.2): eight digits, a real day.
fn valid_date(date: &str) -> bool {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let (y, m, d): (u32, u32, u32) = (
        date[..4].parse().unwrap_or(0),
        date[4..6].parse().unwrap_or(0),
        date[6..].parse().unwrap_or(0),
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    y > 0 && (1..=days).contains(&d)
}

impl Instance {
    /// This instance as a Part 10 file with the patient's attributes changed. A value that
    /// is not plain ASCII makes the file UTF-8 (Specific Character Set `ISO_IR 192`): every
    /// text of the data set is written again in it, which keeps what any other character set
    /// held.
    pub fn with_patient(&self, change: &PatientChange) -> Result<Vec<u8>, Error> {
        if let Some(why) = change.invalid() {
            return Err(Error::Malformed(why));
        }
        let mut object = self.object.clone();
        let mut put = |tag, vr, value: &str| {
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
        if let Some(name) = &change.name {
            put(tags::PATIENT_NAME, VR::PN, name);
        }
        if let Some(id) = &change.id {
            put(tags::PATIENT_ID, VR::LO, id.trim());
        }
        if let Some(date) = &change.birth_date {
            put(tags::PATIENT_BIRTH_DATE, VR::DA, date);
        }
        if let Some(sex) = &change.sex {
            put(tags::PATIENT_SEX, VR::CS, sex);
        }
        if change.values().any(|(_, v)| !v.is_ascii()) {
            object.put(DataElement::new(
                tags::SPECIFIC_CHARACTER_SET,
                VR::CS,
                PrimitiveValue::from("ISO_IR 192"),
            ));
        }
        let mut out = Vec::new();
        object
            .write_all(&mut out)
            .map_err(|e| Error::Malformed(format!("the file could not be written again: {e}")))?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::real::RealSeries;

    fn change() -> PatientChange {
        PatientChange::default()
    }

    #[test]
    fn values_are_checked_against_their_representation() {
        assert!(change().invalid().unwrap().starts_with("nothing to change"));
        let bad = [
            PatientChange {
                id: Some(" ".into()),
                ..change()
            },
            PatientChange {
                id: Some("a\\b".into()),
                ..change()
            },
            PatientChange {
                id: Some("x".repeat(65)),
                ..change()
            },
            PatientChange {
                name: Some("A^B^C^D^E^F".into()),
                ..change()
            },
            PatientChange {
                name: Some("DOE\\JANE".into()),
                ..change()
            },
            PatientChange {
                birth_date: Some("19990231".into()),
                ..change()
            },
            PatientChange {
                birth_date: Some("1999-01-01".into()),
                ..change()
            },
            PatientChange {
                sex: Some("X".into()),
                ..change()
            },
        ];
        for b in bad {
            assert!(b.invalid().is_some(), "{b:?} passed");
        }
        let good = PatientChange {
            name: Some("Benali^Amina".into()),
            id: Some("H-0042".into()),
            birth_date: Some("20000229".into()),
            sex: Some("F".into()),
        };
        assert_eq!(good.invalid(), None);
        // emptying is allowed, except the ID
        assert_eq!(
            PatientChange {
                birth_date: Some(String::new()),
                sex: Some(String::new()),
                ..change()
            }
            .invalid(),
            None
        );
    }

    #[test]
    fn a_file_gets_the_new_values_and_keeps_the_rest() {
        let bytes = RealSeries::new(1, 1).all_bytes().remove(0);
        let before = Instance::from_bytes(&bytes).unwrap();
        let out = before
            .with_patient(&PatientChange {
                name: Some("Benali^Amina".into()),
                id: Some("H-0042".into()),
                birth_date: Some("19800102".into()),
                sex: Some("F".into()),
            })
            .unwrap();
        let after = Instance::from_bytes(&out).unwrap();
        assert_eq!(
            after.string(tags::PATIENT_NAME).as_deref(),
            Some("Benali^Amina")
        );
        assert_eq!(after.string(tags::PATIENT_ID).as_deref(), Some("H-0042"));
        assert_eq!(
            after.string(tags::PATIENT_BIRTH_DATE).as_deref(),
            Some("19800102")
        );
        assert_eq!(after.string(tags::PATIENT_SEX).as_deref(), Some("F"));
        assert_eq!(after.key, before.key);
        assert_eq!(
            after
                .object
                .element(tags::PIXEL_DATA)
                .unwrap()
                .to_bytes()
                .unwrap(),
            before
                .object
                .element(tags::PIXEL_DATA)
                .unwrap()
                .to_bytes()
                .unwrap()
        );
    }

    #[test]
    fn a_name_beyond_ascii_makes_the_file_utf8_and_reads_back() {
        let bytes = RealSeries::new(2, 1).all_bytes().remove(0);
        let out = Instance::from_bytes(&bytes)
            .unwrap()
            .with_patient(&PatientChange {
                name: Some("Bénali^Amina=بن علي^أمينة".into()),
                ..change()
            })
            .unwrap();
        let after = Instance::from_bytes(&out).unwrap();
        assert_eq!(
            after.string(tags::SPECIFIC_CHARACTER_SET).as_deref(),
            Some("ISO_IR 192")
        );
        assert_eq!(
            after.string(tags::PATIENT_NAME).as_deref(),
            Some("Bénali^Amina=بن علي^أمينة")
        );
    }

    #[test]
    fn a_bad_value_writes_nothing() {
        let bytes = RealSeries::new(3, 1).all_bytes().remove(0);
        let error = Instance::from_bytes(&bytes)
            .unwrap()
            .with_patient(&PatientChange {
                sex: Some("X".into()),
                ..change()
            })
            .unwrap_err();
        assert!(
            error.to_string().contains("PatientSex X is not M, F or O"),
            "{error}"
        );
    }
}
