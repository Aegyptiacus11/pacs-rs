//! Study queries: the matching rules of PS3.4 C.2.2.2, the Request Identifier of C.4.1.1.3.1
//! and Table C.6-5, and the SCU's own filtering (C.4.1.2.1).

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use pacs_dimse::query::wildcard_match;
use pacs_dimse::{DateRange, StudyQuery};

fn text(object: &InMemDicomObject, tag: dicom_core::Tag) -> Option<String> {
    object
        .element(tag)
        .ok()
        .map(|e| e.to_str().unwrap().trim_matches([' ', '\0']).to_owned())
}

/// C.2.2.2.5.1: "<date1>-<date2>", "-<date1>", "<date1>-"; and one date, Single Value Matching.
#[test]
fn study_dates_are_a_date_or_a_range() {
    assert_eq!("20260105".parse(), Ok(DateRange::On("20260105".into())));
    let range: DateRange = "20260101-20260131".parse().unwrap();
    assert_eq!(range.key(), "20260101-20260131");
    assert_eq!("-20260131".parse::<DateRange>().unwrap().key(), "-20260131");
    assert_eq!("20260101-".parse::<DateRange>().unwrap().key(), "20260101-");
}

#[test]
fn malformed_dates_are_refused() {
    for bad in [
        "",
        "-",
        "2026-01-01",
        "2026010",
        "20261301",
        "20260230",
        "20250229",
        "2026010a",
        "20260131-20260101",
    ] {
        assert!(bad.parse::<DateRange>().is_err(), "{bad:?}");
    }
    assert!("20240229".parse::<DateRange>().is_ok(), "a leap day");
}

/// C.2.2.2.5.1: the ends are included.
#[test]
fn a_range_includes_its_ends() {
    let range: DateRange = "20260101-20260131".parse().unwrap();
    assert!(range.contains("20260101") && range.contains("20260131") && range.contains("20260115"));
    assert!(!range.contains("20251231") && !range.contains("20260201"));
    let before: DateRange = "-20260101".parse().unwrap();
    assert!(
        before.contains("19990101") && before.contains("20260101") && !before.contains("20260102")
    );
    assert!(!range.contains("not a date"));
}

/// C.2.2.2.8 and C.2.2.2.7: multiple values and empty value matching need extended
/// negotiation, which pacs-rs does not propose; so one value, and not `""`.
#[test]
fn text_keys_are_single_values() {
    let check = |q: StudyQuery| q.checked();
    assert!(
        check(StudyQuery {
            patient_id: Some("A\\B".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        check(StudyQuery {
            accession_number: Some("\"\"".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        check(StudyQuery {
            patient_name: Some("DOE\n".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        check(StudyQuery {
            patient_id: Some("  ".into()),
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        check(StudyQuery {
            patient_name: Some("DOE*".into()),
            ..Default::default()
        })
        .is_ok()
    );
}

/// CS values are upper case (PS3.5 Table 6.2-1); wild cards are defined for CS (C.2.2.2.4).
#[test]
fn a_modality_is_a_cs_value() {
    let q = StudyQuery {
        modality: Some("ct".into()),
        ..Default::default()
    }
    .checked()
    .unwrap();
    assert_eq!(q.modality.as_deref(), Some("CT"));
    assert!(
        StudyQuery {
            modality: Some("C*".into()),
            ..Default::default()
        }
        .checked()
        .is_ok()
    );
    assert!(
        StudyQuery {
            modality: Some("CT-PT".into()),
            ..Default::default()
        }
        .checked()
        .is_err()
    );
}

/// C.4.1.1.3.1 and Table C.6-5: Query/Retrieve Level STUDY, the keys given, the Required Keys
/// and the Optional ones asked for, empty (Universal Matching, C.2.2.2.3); no Specific
/// Character Set when every value is ASCII.
#[test]
fn the_request_identifier() {
    let q = StudyQuery {
        patient_id: Some("P1".into()),
        study_date: Some("20260101-".parse().unwrap()),
        modality: Some("CT".into()),
        ..Default::default()
    };
    let id = q.identifier();
    assert_eq!(
        text(&id, tags::QUERY_RETRIEVE_LEVEL).as_deref(),
        Some("STUDY")
    );
    assert_eq!(text(&id, tags::PATIENT_ID).as_deref(), Some("P1"));
    assert_eq!(text(&id, tags::STUDY_DATE).as_deref(), Some("20260101-"));
    assert_eq!(text(&id, tags::MODALITIES_IN_STUDY).as_deref(), Some("CT"));
    for required in [
        tags::STUDY_TIME,
        tags::ACCESSION_NUMBER,
        tags::PATIENT_NAME,
        tags::STUDY_ID,
        tags::STUDY_INSTANCE_UID,
    ] {
        assert_eq!(text(&id, required).as_deref(), Some(""), "{required}");
    }
    assert!(id.element(tags::SPECIFIC_CHARACTER_SET).is_err());
}

/// C.4.1.1.3.1: Specific Character Set when a value is not in the default repertoire; the
/// value is then sent in UTF-8 (ISO_IR 192).
#[test]
fn a_non_ascii_name_is_sent_in_utf_8() {
    let q = StudyQuery {
        patient_name: Some("Benaïssa^Amel".into()),
        ..Default::default()
    };
    let id = q.identifier();
    assert_eq!(
        text(&id, tags::SPECIFIC_CHARACTER_SET).as_deref(),
        Some("ISO_IR 192")
    );
    let ts = dicom_transfer_syntax_registry::entries::EXPLICIT_VR_LITTLE_ENDIAN.erased();
    let mut bytes = Vec::new();
    id.write_dataset_with_ts(&mut bytes, &ts).unwrap();
    let utf8 = "Benaïssa^Amel".as_bytes();
    assert!(bytes.windows(utf8.len()).any(|w| w == utf8));
    let back = InMemDicomObject::read_dataset_with_ts(&bytes[..], &ts).unwrap();
    assert_eq!(
        text(&back, tags::PATIENT_NAME).as_deref(),
        Some("Benaïssa^Amel")
    );
}

/// C.2.2.2.4: `*` any run of characters, none included; `?` exactly one; case-sensitive,
/// except PN. Without wild cards, Single Value Matching: exact (C.2.2.2.1).
#[test]
fn wild_card_matching() {
    assert!(wildcard_match("DOE*", "DOE", false));
    assert!(wildcard_match("DOE*", "DOE^JOHN", false));
    assert!(wildcard_match("*JOHN", "DOE^JOHN", false));
    assert!(wildcard_match("D?E", "DOE", false));
    assert!(!wildcard_match("D?E", "DE", false));
    assert!(wildcard_match("*", "", false));
    assert!(wildcard_match("A*B*C", "AxxBxxC", false));
    assert!(!wildcard_match("A*B*C", "AxxBxx", false));
    assert!(!wildcard_match("doe*", "DOE^JOHN", false));
    assert!(wildcard_match("doe*", "DOE^JOHN", true));
    assert!(!wildcard_match("P1", "P10", false));
}

fn study(patient_id: &str, date: &str, modalities: &[&str]) -> InMemDicomObject {
    let mut o = InMemDicomObject::new_empty();
    o.put(DataElement::new(
        tags::PATIENT_ID,
        VR::LO,
        PrimitiveValue::from(patient_id),
    ));
    o.put(DataElement::new(
        tags::STUDY_DATE,
        VR::DA,
        PrimitiveValue::from(date),
    ));
    o.put(DataElement::new(
        tags::MODALITIES_IN_STUDY,
        VR::CS,
        PrimitiveValue::Strs(modalities.iter().map(|m| m.to_string()).collect()),
    ));
    o
}

/// C.4.1.2.1: the SCU filters responses an SCP matched without supporting a key; an
/// attribute that came back empty cannot be checked; a multi-valued one passes when any
/// value does (C.2.2.3).
#[test]
fn the_scu_filters_what_comes_back() {
    let q = StudyQuery {
        patient_id: Some("P1".into()),
        study_date: Some("20260101-20260131".parse().unwrap()),
        modality: Some("CT".into()),
        ..Default::default()
    };
    assert!(q.accepts(&study("P1", "20260110", &["PT", "CT"])));
    assert!(!q.accepts(&study("P2", "20260110", &["CT"])));
    assert!(!q.accepts(&study("P1", "20260210", &["CT"])));
    assert!(!q.accepts(&study("P1", "20260110", &["MR"])));
    assert!(
        q.accepts(&study("P1", "", &[])),
        "empty values cannot be checked"
    );
    assert!(
        q.accepts(&InMemDicomObject::new_empty()),
        "absent ones neither"
    );
}
