//! A real PDF (tests/data/documents/kits19.pdf) made into an Encapsulated PDF instance of a
//! study, read back, and its PDF taken out again; and what is refused.

use dicom_dictionary_std::tags;
use pacs_core::Instance;
use pacs_core::document::{
    ENCAPSULATED_PDF, NewDocument, encapsulated_pdf, not_a_pdf, pdf_of, utc,
};
use serde_json::{Map, Value, json};

const PDF: &[u8] = include_bytes!("data/documents/kits19.pdf");

fn maps() -> (Map<String, Value>, Map<String, Value>) {
    let study = json!({
        "StudyInstanceUID": "1.3.6.1.4.1.14519.5.2.1.6450.2626.316236238591966133680679150166",
        "StudyDate": "20090428", "StudyTime": "095300", "AccessionNumber": "A-17",
        "StudyDescription": "CT ABDOMEN AND PELVIS"
    });
    let patient = json!({"PatientName": "C3L-01958", "PatientID": "C3L-01958", "PatientSex": "M"});
    (
        study.as_object().unwrap().clone(),
        patient.as_object().unwrap().clone(),
    )
}

fn document<'a>(
    study: &'a Map<String, Value>,
    patient: &'a Map<String, Value>,
    title: &'a str,
) -> NewDocument<'a> {
    NewDocument {
        study,
        patient,
        title,
        series_uid: "2.25.1001",
        instance_uid: "2.25.1002",
        series_number: 1001,
        date: "20261127",
        time: "224000",
    }
}

#[test]
fn a_pdf_becomes_an_instance_of_its_study_and_comes_out_whole() {
    let (study, patient) = maps();
    let bytes = encapsulated_pdf(&document(&study, &patient, "KiTS19 data paper"), PDF).unwrap();
    let instance = Instance::from_bytes(&bytes).unwrap();
    let s = |tag| instance.string(tag);
    assert_eq!(s(tags::SOP_CLASS_UID).as_deref(), Some(ENCAPSULATED_PDF));
    assert_eq!(s(tags::MODALITY).as_deref(), Some("DOC"));
    assert_eq!(
        s(tags::STUDY_INSTANCE_UID).as_deref(),
        study["StudyInstanceUID"].as_str()
    );
    assert_eq!(s(tags::PATIENT_ID).as_deref(), Some("C3L-01958"));
    assert_eq!(s(tags::ACCESSION_NUMBER).as_deref(), Some("A-17"));
    assert_eq!(
        s(tags::SERIES_DESCRIPTION).as_deref(),
        Some("KiTS19 data paper")
    );
    assert_eq!(
        s(tags::DOCUMENT_TITLE).as_deref(),
        Some("KiTS19 data paper")
    );
    assert_eq!(s(tags::CONVERSION_TYPE).as_deref(), Some("WSD"));
    assert_eq!(
        s(tags::MIME_TYPE_OF_ENCAPSULATED_DOCUMENT).as_deref(),
        Some("application/pdf")
    );
    // its file meta says so too, and it is plain ASCII: no character set needed
    assert_eq!(instance.key.sop_instance_uid, "2.25.1002");
    assert!(s(tags::SPECIFIC_CHARACTER_SET).is_none());
    assert_eq!(pdf_of(&instance.object).as_deref(), Some(PDF));
}

#[test]
fn an_odd_length_is_padded_and_the_length_says_where_the_pdf_ends() {
    let (study, patient) = maps();
    // the real PDF less its last byte: an odd length
    let odd = &PDF[..PDF.len() - 1];
    assert_eq!(odd.len() % 2, 1);
    let bytes = encapsulated_pdf(&document(&study, &patient, "odd"), odd).unwrap();
    let instance = Instance::from_bytes(&bytes).unwrap();
    let stored = instance
        .object
        .element(tags::ENCAPSULATED_DOCUMENT)
        .unwrap()
        .to_bytes()
        .unwrap();
    assert_eq!(stored.len(), odd.len() + 1);
    assert_eq!(
        instance
            .object
            .element(tags::ENCAPSULATED_DOCUMENT_LENGTH)
            .unwrap()
            .to_int::<u32>()
            .unwrap() as usize,
        odd.len()
    );
    assert_eq!(pdf_of(&instance.object).as_deref(), Some(odd));
}

#[test]
fn a_title_or_name_beyond_ascii_makes_it_utf8_and_a_long_title_is_cut_for_the_series() {
    let (study, mut patient) = maps();
    patient.insert("PatientName".into(), json!("Bénali^Amina"));
    let title = "Compte rendu d'échographie hépatique ".repeat(4);
    let bytes = encapsulated_pdf(&document(&study, &patient, &title), PDF).unwrap();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(
        instance.string(tags::SPECIFIC_CHARACTER_SET).as_deref(),
        Some("ISO_IR 192")
    );
    assert_eq!(
        instance.string(tags::PATIENT_NAME).as_deref(),
        Some("Bénali^Amina")
    );
    let description = instance.string(tags::SERIES_DESCRIPTION).unwrap();
    assert!(
        description.chars().count() <= 64 && title.starts_with(&description),
        "{description}"
    );
    assert_eq!(
        instance.string(tags::DOCUMENT_TITLE).as_deref(),
        Some(title.trim())
    );
}

#[test]
fn what_is_not_a_pdf_or_has_no_study_is_refused() {
    let (study, patient) = maps();
    assert_eq!(not_a_pdf(b""), Some("the body is empty".into()));
    // a real DICOM file is not a PDF
    let dicom = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/pydicom/CT_small.dcm"
    ))
    .unwrap();
    assert!(not_a_pdf(&dicom).unwrap().contains("%PDF-"));
    assert!(encapsulated_pdf(&document(&study, &patient, "x"), &dicom).is_err());
    let empty = Map::new();
    let error = encapsulated_pdf(&document(&empty, &patient, "x"), PDF).unwrap_err();
    assert!(error.to_string().contains("StudyInstanceUID"), "{error}");
    // an instance that is no document has no PDF to give
    assert_eq!(pdf_of(&Instance::from_bytes(&dicom).unwrap().object), None);
}

#[test]
fn the_moment_is_written_as_a_dicom_date_and_time_in_utc() {
    use std::time::{Duration, UNIX_EPOCH};
    assert_eq!(utc(UNIX_EPOCH), ("19700101".into(), "000000".into()));
    // 2000-02-29T23:59:59Z, a leap day of a century leap year
    assert_eq!(
        utc(UNIX_EPOCH + Duration::from_secs(951_868_799)),
        ("20000229".into(), "235959".into())
    );
    // 2026-11-27T22:40:00Z
    assert_eq!(
        utc(UNIX_EPOCH + Duration::from_secs(1_795_819_200)),
        ("20261127".into(), "224000".into())
    );
}
