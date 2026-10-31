//! De-identification by PS3.15 Annex E: one test per rule of E.1.1, the action codes of
//! Table E.1-1a, the Options of E.3 that pacs-rs implements, the quarantine, and the
//! project's overrides.

use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use pacs_deid::{Action, Keys, Opt, Outcome, Profile, ProfileError, deidentify, explain};

const CT: &str = "1.2.840.10008.5.1.4.1.1.2";

fn keys() -> Keys {
    Keys::new(*b"a project secret of 32 bytes ok!").unwrap()
}

fn put(o: &mut InMemDicomObject, tag: Tag, vr: VR, value: &str) {
    o.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
}

/// A CT instance of patient P123 with what identifies and what does not.
fn instance(sop: &str, study_date: &str) -> InMemDicomObject {
    let mut o = InMemDicomObject::new_empty();
    put(&mut o, tags::SOP_CLASS_UID, VR::UI, CT);
    put(&mut o, tags::SOP_INSTANCE_UID, VR::UI, sop);
    put(&mut o, tags::STUDY_INSTANCE_UID, VR::UI, "1.2.3.4.100");
    put(&mut o, tags::SERIES_INSTANCE_UID, VR::UI, "1.2.3.4.100.1");
    put(
        &mut o,
        tags::FRAME_OF_REFERENCE_UID,
        VR::UI,
        "1.2.3.4.100.9",
    );
    put(&mut o, tags::PATIENT_NAME, VR::PN, "BENALI^AMINE");
    put(&mut o, tags::PATIENT_ID, VR::LO, "P123");
    put(&mut o, tags::PATIENT_BIRTH_DATE, VR::DA, "19700615");
    put(&mut o, tags::PATIENT_SEX, VR::CS, "M");
    put(&mut o, tags::PATIENT_WEIGHT, VR::DS, "80");
    put(&mut o, tags::ACCESSION_NUMBER, VR::SH, "ACC9");
    put(&mut o, tags::INSTITUTION_NAME, VR::LO, "CHU Mustapha");
    put(&mut o, tags::STUDY_DATE, VR::DA, study_date);
    put(&mut o, tags::STUDY_TIME, VR::TM, "101500");
    put(
        &mut o,
        tags::ACQUISITION_DATE_TIME,
        VR::DT,
        &format!("{study_date}101530.25"),
    );
    put(&mut o, tags::STUDY_DESCRIPTION, VR::LO, "CT ABDOMEN");
    put(&mut o, tags::SERIES_DESCRIPTION, VR::LO, "Arterial 1.25mm");
    put(&mut o, tags::MANUFACTURER, VR::LO, "GE MEDICAL SYSTEMS");
    put(&mut o, tags::DEVICE_SERIAL_NUMBER, VR::LO, "SN-77");
    put(&mut o, tags::MODALITY, VR::CS, "CT");
    put(&mut o, tags::ROWS, VR::US, "512");
    // a private creator and its element
    put(&mut o, Tag(0x0019, 0x0010), VR::LO, "GEMS_ACQU_01");
    put(&mut o, Tag(0x0019, 0x1023), VR::DS, "1.375");
    o
}

fn run(o: &InMemDicomObject, profile: &Profile) -> InMemDicomObject {
    match deidentify(o, profile, &keys()) {
        Outcome::Deidentified(o) => *o,
        Outcome::Quarantined(why) => panic!("quarantined: {why}"),
    }
}

fn value(o: &InMemDicomObject, tag: Tag) -> Option<String> {
    o.element(tag).ok().map(|e| {
        e.to_str()
            .map(|s| s.trim_matches([' ', '\0']).to_owned())
            .unwrap_or_default()
    })
}

fn profile(json: &str) -> Profile {
    Profile::from_json(json).unwrap()
}

/// Table E.1-1 as read: a few rows by their Basic Profile and Option actions.
#[test]
fn the_table_is_the_standards() {
    let basic = Profile::basic("t");
    assert_eq!(
        explain(&basic, 0x0010, 0x0010),
        Some(("Patient's Name", Action::Z))
    );
    assert_eq!(
        explain(&basic, 0x0008, 0x0018),
        Some(("SOP Instance UID", Action::U))
    );
    assert_eq!(
        explain(&basic, 0x0008, 0x1030),
        Some(("Study Description", Action::X))
    );
    // X/Z/D resolved to D, which keeps any IOD valid
    assert_eq!(
        explain(&basic, 0x0018, 0x1000).map(|e| e.1),
        Some(Action::D)
    );
    let device = profile(r#"{"name": "t", "options": ["retain-device-identity"]}"#);
    assert_eq!(
        explain(&device, 0x0018, 0x1000).map(|e| e.1),
        Some(Action::K)
    );
    let clean = profile(r#"{"name": "t", "options": ["clean-descriptors"]}"#);
    assert_eq!(
        explain(&clean, 0x0008, 0x1030).map(|e| e.1),
        Some(Action::C)
    );
    // overlays are a repeating group
    assert_eq!(
        explain(&basic, 0x6002, 0x3000).map(|e| e.1),
        Some(Action::X)
    );
    assert_eq!(
        explain(&basic, 0x0008, 0x0070),
        None,
        "Manufacturer is not listed"
    );
}

/// E.1.1: Patient's Name and ID replaced by a pseudonym (Z allows a dummy), birth date and
/// accession number emptied, X attributes removed, X/Z/D ones given a dummy.
#[test]
fn the_basic_profile_protects_the_patient() {
    let out = run(
        &instance("1.2.3.4.100.1.1", "20260105"),
        &Profile::basic("t"),
    );
    let pseudonym = value(&out, tags::PATIENT_ID).unwrap();
    assert_eq!(pseudonym.len(), 10);
    assert_eq!(
        value(&out, tags::PATIENT_NAME).as_deref(),
        Some(pseudonym.as_str())
    );
    assert_eq!(value(&out, tags::PATIENT_BIRTH_DATE).as_deref(), Some(""));
    assert_eq!(value(&out, tags::ACCESSION_NUMBER).as_deref(), Some(""));
    assert_eq!(value(&out, tags::STUDY_DESCRIPTION), None);
    assert_eq!(
        value(&out, tags::INSTITUTION_NAME).as_deref(),
        Some("ANONYMIZED")
    );
    assert_eq!(
        value(&out, tags::DEVICE_SERIAL_NUMBER).as_deref(),
        Some("ANONYMIZED")
    );
    assert_eq!(value(&out, tags::STUDY_DATE).as_deref(), Some(""));
    // not in the table: kept
    assert_eq!(
        value(&out, tags::MANUFACTURER).as_deref(),
        Some("GE MEDICAL SYSTEMS")
    );
    assert_eq!(value(&out, tags::MODALITY).as_deref(), Some("CT"));
    assert_eq!(
        value(&out, tags::PATIENT_SEX).as_deref(),
        Some(""),
        "Patient's Sex is Z without the option"
    );
}

/// E.1.1: UIDs replaced, consistently across instances (same study, same new Study Instance
/// UID) and inside sequences (a reference keeps pointing at the replaced instance).
#[test]
fn uids_are_replaced_consistently() {
    let basic = Profile::basic("t");
    let mut a = instance("1.2.3.4.100.1.1", "20260105");
    let reference = InMemDicomObject::from_element_iter([
        DataElement::new(
            tags::REFERENCED_SOP_CLASS_UID,
            VR::UI,
            PrimitiveValue::from(CT),
        ),
        DataElement::new(
            tags::REFERENCED_SOP_INSTANCE_UID,
            VR::UI,
            PrimitiveValue::from("1.2.3.4.100.1.2"),
        ),
    ]);
    a.put(DataElement::new(
        tags::REFERENCED_IMAGE_SEQUENCE,
        VR::SQ,
        Value::Sequence(DataSetSequence::new(vec![reference], Length::UNDEFINED)),
    ));
    let (a, b) = (
        run(&a, &basic),
        run(&instance("1.2.3.4.100.1.2", "20260105"), &basic),
    );
    let study = value(&a, tags::STUDY_INSTANCE_UID).unwrap();
    assert!(study.starts_with("2.25.") && study.len() <= 64, "{study}");
    assert_eq!(value(&b, tags::STUDY_INSTANCE_UID), Some(study));
    assert_ne!(
        value(&a, tags::SOP_INSTANCE_UID),
        value(&b, tags::SOP_INSTANCE_UID)
    );
    let item = &a
        .element(tags::REFERENCED_IMAGE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()[0];
    assert_eq!(
        value(item, tags::REFERENCED_SOP_INSTANCE_UID),
        value(&b, tags::SOP_INSTANCE_UID)
    );
    assert_eq!(
        value(item, tags::REFERENCED_SOP_CLASS_UID).as_deref(),
        Some(CT),
        "SOP classes are kept"
    );
}

/// E.1.1 and Table E.1-1: private attributes are removed, creators included; a project may
/// keep a private group it trusts with an override.
#[test]
fn private_attributes_are_removed_unless_kept_by_override() {
    let out = run(
        &instance("1.2.3.4.100.1.1", "20260105"),
        &Profile::basic("t"),
    );
    assert!(out.element(Tag(0x0019, 0x0010)).is_err() && out.element(Tag(0x0019, 0x1023)).is_err());
    let kept = profile(
        r#"{"name": "t", "overrides": [{"tag": "(0019,xxxx)", "action": "K", "reason": "GE acquisition values"}]}"#,
    );
    let out = run(&instance("1.2.3.4.100.1.1", "20260105"), &kept);
    assert_eq!(value(&out, Tag(0x0019, 0x1023)).as_deref(), Some("1.375"));
}

/// E.1.1: group 0004 is removed from anything but a DICOMDIR.
#[test]
fn group_0004_is_removed() {
    let mut o = instance("1.2.3.4.100.1.1", "20260105");
    put(&mut o, Tag(0x0004, 0x1500), VR::CS, "DIR");
    assert!(
        run(&o, &Profile::basic("t"))
            .element(Tag(0x0004, 0x1500))
            .is_err()
    );
}

/// E.1.1: Patient Identity Removed YES; CID 7050 codes for the Profile and each Option; the
/// method names the profile and each override; Longitudinal Temporal Information Modified.
#[test]
fn the_method_is_recorded() {
    let p = profile(
        r#"{"name": "liver-2026", "options": ["retain-longitudinal-modified-dates", "clean-descriptors"],
            "overrides": [{"tag": "(0008,103E)", "action": "K", "reason": "phase names"}]}"#,
    );
    let out = run(&instance("1.2.3.4.100.1.1", "20260105"), &p);
    assert_eq!(
        value(&out, tags::PATIENT_IDENTITY_REMOVED).as_deref(),
        Some("YES")
    );
    let codes: Vec<String> = out
        .element(tags::DEIDENTIFICATION_METHOD_CODE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()
        .iter()
        .map(|i| value(i, tags::CODE_VALUE).unwrap())
        .collect();
    assert_eq!(codes, ["113100", "113107", "113105"]);
    let method = out
        .element(tags::DEIDENTIFICATION_METHOD)
        .unwrap()
        .to_multi_str()
        .unwrap()
        .to_vec();
    assert_eq!(method[0], "pacs-rs profile liver-2026");
    assert!(
        method[1].starts_with("override (0008,103E) K: phase names"),
        "{method:?}"
    );
    assert_eq!(
        value(&out, tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED).as_deref(),
        Some("MODIFIED")
    );
    let basic = run(
        &instance("1.2.3.4.100.1.1", "20260105"),
        &Profile::basic("t"),
    );
    assert_eq!(
        value(&basic, tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED).as_deref(),
        Some("REMOVED")
    );
}

/// E.3.6, Modified Dates: every date of a patient moves by one shift, in the profile's range,
/// so intervals survive; times are kept; DT keeps its time; Birth Date stays empty (Z).
#[test]
fn modified_dates_shift_one_patient_by_one_offset() {
    let p = profile(
        r#"{"name": "t", "options": ["retain-longitudinal-modified-dates"], "date_shift_days": [-400, -10]}"#,
    );
    let first = run(&instance("1.2.3.4.100.1.1", "20260105"), &p);
    let mut later = instance("1.2.3.4.200.1.1", "20260304");
    put(&mut later, tags::STUDY_INSTANCE_UID, VR::UI, "1.2.3.4.200");
    let later = run(&later, &p);
    let a = value(&first, tags::STUDY_DATE).unwrap();
    assert_ne!(a, "20260105");
    let shift_back = |from: &str| {
        (0..=400).find(|n| pacs_deid::dates::shift_da(from, *n).as_deref() == Some("20260105"))
    };
    let offset = shift_back(&a).expect("shifted back by at most 400 days");
    assert!((10..=400).contains(&offset));
    assert_eq!(
        pacs_deid::dates::shift_da("20260304", -offset),
        value(&later, tags::STUDY_DATE),
        "same shift, interval kept"
    );
    assert_eq!(value(&first, tags::STUDY_TIME).as_deref(), Some("101500"));
    assert!(
        value(&first, tags::ACQUISITION_DATE_TIME)
            .unwrap()
            .ends_with("101530.25")
    );
    assert_eq!(value(&first, tags::PATIENT_BIRTH_DATE).as_deref(), Some(""));
}

/// E.1.1: dates may be handled by VR; with Modified Dates, a date the table does not list is
/// shifted too.
#[test]
fn modified_dates_reach_unlisted_dates() {
    let p = profile(r#"{"name": "t", "options": ["retain-longitudinal-modified-dates"]}"#);
    let mut o = instance("1.2.3.4.100.1.1", "20260105");
    put(&mut o, Tag(0x0018, 0x1012), VR::DA, "20260105"); // Date of Secondary Capture
    put(&mut o, Tag(0x0040, 0x0244), VR::DA, "20260105"); // Performed Procedure Step Start Date
    let out = run(&o, &p);
    assert_eq!(
        value(&out, Tag(0x0018, 0x1012)),
        value(&out, tags::STUDY_DATE)
    );
    assert_eq!(
        value(&out, Tag(0x0040, 0x0244)),
        value(&out, tags::STUDY_DATE)
    );
}

/// E.3.6, Full Dates: dates kept, Longitudinal Temporal Information Modified UNMODIFIED.
#[test]
fn full_dates_keep_dates() {
    let p = profile(r#"{"name": "t", "options": ["retain-longitudinal-full-dates"]}"#);
    let out = run(&instance("1.2.3.4.100.1.1", "20260105"), &p);
    assert_eq!(value(&out, tags::STUDY_DATE).as_deref(), Some("20260105"));
    assert_eq!(
        value(&out, tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED).as_deref(),
        Some("UNMODIFIED")
    );
}

/// E.3.7: sex and weight kept; Patient's Age added from the dates when absent (E.1.1 note),
/// as Birth Date is emptied.
#[test]
fn patient_characteristics_are_kept_and_age_added() {
    let p = profile(r#"{"name": "t", "options": ["retain-patient-characteristics"]}"#);
    let out = run(&instance("1.2.3.4.100.1.1", "20260105"), &p);
    assert_eq!(value(&out, tags::PATIENT_SEX).as_deref(), Some("M"));
    assert_eq!(value(&out, tags::PATIENT_WEIGHT).as_deref(), Some("80"));
    assert_eq!(value(&out, tags::PATIENT_AGE).as_deref(), Some("055Y"));
    let mut aged = instance("1.2.3.4.100.1.1", "20260105");
    put(&mut aged, tags::PATIENT_AGE, VR::AS, "054Y");
    assert_eq!(
        value(&run(&aged, &p), tags::PATIENT_AGE).as_deref(),
        Some("054Y"),
        "a given age is kept"
    );
}

/// E.3.5: a description is kept unless it holds the patient's name or identifiers, or a word
/// the project lists; the patient's name is matched by its components, ignoring case.
#[test]
fn clean_descriptors_keep_safe_text_only() {
    let p = profile(
        r#"{"name": "t", "options": ["clean-descriptors"], "extra_identifiers": ["Mustapha"]}"#,
    );
    let mut o = instance("1.2.3.4.100.1.1", "20260105");
    put(&mut o, tags::STUDY_DESCRIPTION, VR::LO, "CT abdomen Benali");
    put(
        &mut o,
        Tag(0x0018, 0x1030),
        VR::LO,
        "LIVER 3 PHASES MUSTAPHA",
    ); // Protocol Name
    let out = run(&o, &p);
    assert_eq!(
        value(&out, tags::SERIES_DESCRIPTION).as_deref(),
        Some("Arterial 1.25mm")
    );
    assert_eq!(value(&out, tags::STUDY_DESCRIPTION).as_deref(), Some(""));
    assert_eq!(value(&out, Tag(0x0018, 0x1030)).as_deref(), Some(""));
}

/// An override wins over the table and is recorded; here, keep Study Description without
/// Clean Descriptors.
#[test]
fn overrides_win_over_the_table() {
    let p = profile(r#"{"name": "t", "overrides": [{"tag": "(0008,1030)", "action": "K"}]}"#);
    assert_eq!(
        value(
            &run(&instance("1.2.3.4.100.1.1", "20260105"), &p),
            tags::STUDY_DESCRIPTION
        )
        .as_deref(),
        Some("CT ABDOMEN")
    );
}

/// remove_when_allowed: X wherever Table E.1-1 allows it (X/Z/D), and only there (Z).
#[test]
fn remove_when_allowed_removes_rather_than_dummies() {
    let p = profile(r#"{"name": "t", "remove_when_allowed": true}"#);
    let out = run(&instance("1.2.3.4.100.1.1", "20260105"), &p);
    assert_eq!(value(&out, tags::INSTITUTION_NAME), None);
    assert_eq!(
        value(&out, tags::STUDY_DATE).as_deref(),
        Some(""),
        "Study Date is Z, no X to take"
    );
}

/// Quarantine: Secondary Capture, Burned In Annotation YES, Image Type SCREEN SAVE; a CT
/// is not; a project may clear the list.
#[test]
fn what_may_carry_burned_in_text_is_quarantined() {
    let basic = Profile::basic("t");
    let reason = |o: &InMemDicomObject, p: &Profile| match deidentify(o, p, &keys()) {
        Outcome::Quarantined(why) => Some(why),
        Outcome::Deidentified(_) => None,
    };
    let mut sc = instance("1.2.3.4.100.1.1", "20260105");
    put(
        &mut sc,
        tags::SOP_CLASS_UID,
        VR::UI,
        "1.2.840.10008.5.1.4.1.1.7",
    );
    assert!(reason(&sc, &basic).unwrap().contains("SOP Class"));
    let mut burned = instance("1.2.3.4.100.1.1", "20260105");
    put(&mut burned, tags::BURNED_IN_ANNOTATION, VR::CS, "YES");
    assert!(reason(&burned, &basic).unwrap().contains("Burned In"));
    let mut screen = instance("1.2.3.4.100.1.1", "20260105");
    screen.put(DataElement::new(
        tags::IMAGE_TYPE,
        VR::CS,
        PrimitiveValue::Strs(
            [
                "DERIVED".to_owned(),
                "SECONDARY".into(),
                "SCREEN SAVE".into(),
            ]
            .into_iter()
            .collect(),
        ),
    ));
    assert!(reason(&screen, &basic).unwrap().contains("SCREEN SAVE"));
    let mut mpr = instance("1.2.3.4.100.1.1", "20260105");
    mpr.put(DataElement::new(
        tags::IMAGE_TYPE,
        VR::CS,
        PrimitiveValue::Strs(
            ["DERIVED".to_owned(), "SECONDARY".into(), "MPR".into()]
                .into_iter()
                .collect(),
        ),
    ));
    assert_eq!(reason(&mpr, &basic), None, "a reformat is not a screen");
    let open = profile(r#"{"name": "t", "quarantine": {"sop_classes": []}}"#);
    assert_eq!(reason(&sc, &open), None);
}

/// Profiles are checked: options pacs-rs does not implement, the two date options together,
/// a shift range that includes 0, unknown fields.
#[test]
fn profiles_are_checked() {
    let err = |json| Profile::from_json(json).unwrap_err();
    assert_eq!(
        err(r#"{"name": "t", "options": ["clean-graphics"]}"#),
        ProfileError::Unsupported(Opt::CleanGraphics)
    );
    assert_eq!(
        err(
            r#"{"name": "t", "options": ["retain-longitudinal-full-dates", "retain-longitudinal-modified-dates"]}"#
        ),
        ProfileError::BothDateOptions
    );
    assert_eq!(
        err(r#"{"name": "t", "date_shift_days": [-10, 10]}"#),
        ProfileError::ZeroShift
    );
    assert!(matches!(
        err(r#"{"name": "t", "colour": "blue"}"#),
        ProfileError::Json(_)
    ));
    assert!(matches!(
        err(r#"{"name": "t", "overrides": [{"tag": "(0019,zz10)", "action": "K"}]}"#),
        ProfileError::Json(_)
    ));
}

/// A patient without Patient ID: one pseudonym per study, still consistent within it.
#[test]
fn a_patient_without_id_is_one_per_study() {
    let basic = Profile::basic("t");
    let mut a = instance("1.2.3.4.100.1.1", "20260105");
    let mut b = instance("1.2.3.4.100.1.2", "20260105");
    put(&mut a, tags::PATIENT_ID, VR::LO, "");
    put(&mut b, tags::PATIENT_ID, VR::LO, "");
    assert_eq!(
        value(&run(&a, &basic), tags::PATIENT_ID),
        value(&run(&b, &basic), tags::PATIENT_ID)
    );
}
