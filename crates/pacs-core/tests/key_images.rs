//! Key images of real CT slices (`RealSeries`) as a Key Object Selection: written, read back
//! as another system would, and what is refused.

use dicom_dictionary_std::tags;
use pacs_core::Instance;
use pacs_core::key_images::{
    KEY_OBJECT_SELECTION, KeyImage, NewKeyImages, key_images_of, key_object_selection,
};
use pacs_core::real::RealSeries;
use serde_json::{Map, Value, json};

fn maps(series: &RealSeries) -> (Map<String, Value>, Map<String, Value>) {
    let study = json!({"StudyInstanceUID": series.study_uid(), "StudyDate": "20090428", "AccessionNumber": "A-17"});
    let patient = json!({"PatientName": "C3L-01958", "PatientID": series.patient_id()});
    (
        study.as_object().unwrap().clone(),
        patient.as_object().unwrap().clone(),
    )
}

fn new<'a>(
    study: &'a Map<String, Value>,
    patient: &'a Map<String, Value>,
    description: &'a str,
) -> NewKeyImages<'a> {
    NewKeyImages {
        study,
        patient,
        description,
        series_uid: "2.25.2001",
        instance_uid: "2.25.2002",
        series_number: 2001,
        date: "20261127",
        time: "230000",
    }
}

/// Slice `n` of `series` as a key image, as its own file says.
fn key(series: &RealSeries, n: u32) -> KeyImage {
    let instance = Instance::from_bytes(&series.slice_bytes(n)).unwrap();
    KeyImage {
        sop_class: instance.key.sop_class_uid,
        sop_instance: instance.key.sop_instance_uid,
        series: instance.key.series_instance_uid,
        frame: None,
    }
}

#[test]
fn key_images_are_written_as_a_key_object_selection_and_read_back() {
    let ct = RealSeries::new(61, 3).for_patient(61);
    let (study, patient) = maps(&ct);
    let images = [key(&ct, 2), key(&ct, 0), key(&ct, 2)];
    let bytes =
        key_object_selection(&new(&study, &patient, "Mass, right kidney"), &images).unwrap();
    let ko = Instance::from_bytes(&bytes).unwrap();
    let s = |tag| ko.string(tag);
    assert_eq!(
        s(tags::SOP_CLASS_UID).as_deref(),
        Some(KEY_OBJECT_SELECTION)
    );
    assert_eq!(s(tags::MODALITY).as_deref(), Some("KO"));
    assert_eq!(s(tags::STUDY_INSTANCE_UID), Some(ct.study_uid()));
    assert_eq!(s(tags::PATIENT_ID), Some(ct.patient_id()));
    assert_eq!(s(tags::VALUE_TYPE).as_deref(), Some("CONTAINER"));
    // titled Of Interest, under TID 2010
    let title = ko
        .object
        .element(tags::CONCEPT_NAME_CODE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()[0]
        .clone();
    assert_eq!(
        title.element(tags::CODE_VALUE).unwrap().to_str().unwrap(),
        "113000"
    );
    let template = ko
        .object
        .element(tags::CONTENT_TEMPLATE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()[0]
        .clone();
    assert_eq!(
        template
            .element(tags::TEMPLATE_IDENTIFIER)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "2010"
    );
    // the description first, then each image once, in the order chosen
    let content = ko
        .object
        .element(tags::CONTENT_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()
        .to_vec();
    assert_eq!(content.len(), 3);
    assert_eq!(
        content[0]
            .element(tags::TEXT_VALUE)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "Mass, right kidney"
    );
    assert_eq!(
        key_images_of(&ko.object).unwrap(),
        vec![key(&ct, 2), key(&ct, 0)]
    );
}

#[test]
fn images_of_two_series_are_listed_by_series_and_a_frame_is_kept() {
    let (a, b) = (
        RealSeries::new(62, 2).for_patient(62),
        RealSeries::new(63, 1).for_patient(62),
    );
    let (study, patient) = maps(&a);
    let mut framed = key(&b, 0);
    framed.frame = Some(1);
    let images = [key(&a, 1), framed.clone(), key(&a, 0)];
    let ko =
        Instance::from_bytes(&key_object_selection(&new(&study, &patient, ""), &images).unwrap())
            .unwrap();
    let evidence = ko
        .object
        .element(tags::CURRENT_REQUESTED_PROCEDURE_EVIDENCE_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()[0]
        .clone();
    let series = evidence
        .element(tags::REFERENCED_SERIES_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()
        .to_vec();
    assert_eq!(series.len(), 2);
    assert_eq!(
        series[0]
            .element(tags::REFERENCED_SOP_SEQUENCE)
            .unwrap()
            .items()
            .unwrap()
            .len(),
        2
    );
    // no description: images only
    assert_eq!(
        ko.object
            .element(tags::CONTENT_SEQUENCE)
            .unwrap()
            .items()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(key_images_of(&ko.object).unwrap(), images.to_vec());
}

#[test]
fn nothing_chosen_a_reference_without_uids_or_a_study_without_one_is_refused() {
    let ct = RealSeries::new(64, 1).for_patient(64);
    let (study, patient) = maps(&ct);
    assert!(
        key_object_selection(&new(&study, &patient, ""), &[])
            .unwrap_err()
            .to_string()
            .contains("no key image")
    );
    let mut blank = key(&ct, 0);
    blank.series.clear();
    assert!(key_object_selection(&new(&study, &patient, ""), &[blank]).is_err());
    let empty = Map::new();
    let error = key_object_selection(&new(&empty, &patient, ""), &[key(&ct, 0)]).unwrap_err();
    assert!(error.to_string().contains("StudyInstanceUID"), "{error}");
    // an image is no Key Object Selection
    assert_eq!(
        key_images_of(&Instance::from_bytes(&ct.slice_bytes(0)).unwrap().object),
        None
    );
}

#[test]
fn a_description_beyond_ascii_makes_it_utf8() {
    let ct = RealSeries::new(65, 1).for_patient(65);
    let (study, patient) = maps(&ct);
    let ko = Instance::from_bytes(
        &key_object_selection(
            &new(&study, &patient, "Masse rénale droite"),
            &[key(&ct, 0)],
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        ko.string(tags::SPECIFIC_CHARACTER_SET).as_deref(),
        Some("ISO_IR 192")
    );
    let content = ko
        .object
        .element(tags::CONTENT_SEQUENCE)
        .unwrap()
        .items()
        .unwrap()[0]
        .clone();
    assert_eq!(
        content
            .element(tags::TEXT_VALUE)
            .unwrap()
            .to_str()
            .unwrap()
            .trim(),
        "Masse rénale droite"
    );
}
