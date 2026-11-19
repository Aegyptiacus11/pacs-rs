use pacs_core::Instance;
use pacs_core::real::RealSeries;

#[test]
fn a_series_parses_back_to_its_identifiers() {
    let series = RealSeries::new(7, 3);
    for (slice, bytes) in series.all_bytes().iter().enumerate() {
        let instance = Instance::from_bytes(bytes).unwrap();
        assert_eq!(instance.key.study_instance_uid, series.study_uid());
        assert_eq!(instance.key.series_instance_uid, series.series_uid());
        assert_eq!(
            instance.key.sop_instance_uid,
            series.instance_uid(slice as u32)
        );
        assert_eq!(instance.key.sop_class_uid, "1.2.840.10008.5.1.4.1.1.2");
        assert_eq!(instance.key.transfer_syntax_uid, "1.2.840.10008.1.2.1");
    }
}

#[test]
fn the_preamble_is_optional() {
    let bytes = RealSeries::new(8, 1).slice_bytes(0);
    assert_eq!(&bytes[128..132], b"DICM");
    let with = Instance::from_bytes(&bytes).unwrap();
    let without = Instance::from_bytes(&bytes[128..]).unwrap();
    assert_eq!(with.key, without.key);
}

#[test]
fn dicom_json_carries_the_identifiers_and_vrs() {
    let series = RealSeries::new(9, 1);
    let json = Instance::from_bytes(&series.slice_bytes(0))
        .unwrap()
        .to_json()
        .unwrap();
    assert_eq!(json["0020000D"]["vr"], "UI");
    assert_eq!(json["0020000D"]["Value"][0], series.study_uid());
    assert_eq!(json["00080060"]["Value"][0], "CT");
    assert_eq!(json["00100010"]["vr"], "PN");
    assert_eq!(
        json["00100010"]["Value"][0]["Alphabetic"],
        "CompressedSamples^CT1"
    );
    assert_eq!(json["00280010"]["Value"][0], 128);
    // file meta group (0002,xxxx) is not part of the data set
    assert!(json.get("00020010").is_none());
}

#[test]
fn garbage_is_an_error_not_a_panic() {
    assert!(Instance::from_bytes(b"not dicom").is_err());
}

#[test]
fn uids_are_digits_and_dots_only() {
    use pacs_core::is_valid_uid;
    assert!(is_valid_uid("1.2.840.10008.5.1.4.1.1.2"));
    assert!(is_valid_uid("2.25.0123"));
    assert!(!is_valid_uid(""));
    assert!(!is_valid_uid("1..2"));
    assert!(!is_valid_uid("1.2."));
    assert!(!is_valid_uid("../../etc/passwd"));
    assert!(!is_valid_uid("1.2/3"));
    assert!(!is_valid_uid(&"1".repeat(65)));
}

#[test]
fn metadata_leaves_out_pixel_data() {
    let instance = Instance::from_bytes(&RealSeries::new(10, 1).slice_bytes(0)).unwrap();
    assert!(instance.to_json().unwrap().get("7FE00010").is_some());
    let metadata = instance.metadata_json().unwrap();
    assert!(metadata.get("7FE00010").is_none());
    assert_eq!(metadata["00280010"]["Value"][0], 128);
}

/// PS3.10 7.2: the value of Data Set Trailing Padding has no significance and is ignored by
/// whoever reads the data set. The scanner's file ends with one.
#[test]
fn metadata_leaves_out_the_trailing_padding() {
    let instance = Instance::from_bytes(&RealSeries::new(10, 1).slice_bytes(0)).unwrap();
    assert!(
        instance.to_json().unwrap().get("FFFCFFFC").is_some(),
        "the file no longer ends with the padding this test is about"
    );
    assert!(instance.metadata_json().unwrap().get("FFFCFFFC").is_none());
}

/// FL and FD are the numbers the file's bytes hold, to the last bit: (0027,1050) is the four
/// bytes CC CC 7C C2 and (0023,1070) the eight bytes D6 37 8E 88 96 B3 C9 41 in the scanner's
/// file. The reference server's answer is one unit in the last place off for both.
#[test]
fn metadata_floats_are_the_bytes_of_the_file() {
    let instance = Instance::from_bytes(&RealSeries::new(10, 1).slice_bytes(0)).unwrap();
    let metadata = instance.metadata_json().unwrap();
    assert_eq!(
        metadata["00271050"]["Value"][0].as_f64(),
        Some(f64::from(f32::from_le_bytes([0xCC, 0xCC, 0x7C, 0xC2])))
    );
    assert_eq!(
        metadata["00231070"]["Value"][0].as_f64(),
        Some(f64::from_le_bytes([
            0xD6, 0x37, 0x8E, 0x88, 0x96, 0xB3, 0xC9, 0x41
        ]))
    );
}

#[test]
fn level_attributes_are_split_by_level() {
    use pacs_core::levels;
    let series = RealSeries::new(15, 1);
    let instance = Instance::from_bytes(&series.slice_bytes(0)).unwrap();
    let study = instance.attributes_json(levels::STUDY).unwrap();
    assert_eq!(study["0020000D"]["Value"][0], series.study_uid());
    assert_eq!(study["00100020"]["Value"][0], "NEMA-15");
    assert!(study.get("00080060").is_none());
    let series_json = instance.attributes_json(levels::SERIES).unwrap();
    assert_eq!(series_json["00080060"]["Value"][0], "CT");
    assert!(series_json.get("00100010").is_none());
    let inst = instance.attributes_json(levels::INSTANCE).unwrap();
    assert_eq!(inst["00200013"]["Value"][0], 1);
}

#[test]
fn strings_and_ints_for_the_index() {
    use dicom_dictionary_std::tags;
    let instance = Instance::from_bytes(&RealSeries::new(16, 2).slice_bytes(1)).unwrap();
    assert_eq!(instance.string(tags::MODALITY).as_deref(), Some("CT"));
    assert_eq!(
        instance.string(tags::STUDY_DATE).as_deref(),
        Some("20040119")
    );
    assert_eq!(instance.string(tags::REQUESTING_PHYSICIAN), None);
    assert_eq!(instance.int(tags::INSTANCE_NUMBER), Some(2));
    assert_eq!(instance.int(tags::ROWS), Some(128));
}

#[test]
fn a_latin1_name_round_trips() {
    let series = RealSeries::new(17, 1).with_patient("MÜLLER^RENÉ", Some("ISO_IR 100"));
    let bytes = series.slice_bytes(0);
    // written in ISO 8859-1, one byte per letter: no UTF-8 sequence for Ü (C3 9C)
    assert!(bytes.windows(2).all(|w| w != [0xC3, 0x9C]));
    assert!(bytes.windows(6).any(|w| w == b"M\xDCLLER"));
    let instance = Instance::from_bytes(&bytes).unwrap();
    let json = instance.to_json().unwrap();
    assert_eq!(json["00100010"]["Value"][0]["Alphabetic"], "MÜLLER^RENÉ");
}

#[test]
fn main_tags_are_raw_strings_by_keyword() {
    use pacs_core::main_tags;
    let series = RealSeries::new(18, 2).with_patient("MÜLLER^RENÉ", Some("ISO_IR 100"));
    let instance = Instance::from_bytes(&series.slice_bytes(1)).unwrap();
    let patient = instance.main_tags(main_tags::PATIENT);
    assert_eq!(patient["PatientName"], "MÜLLER^RENÉ");
    assert_eq!(patient["PatientID"], "NEMA-18");
    assert!(patient.get("OtherPatientIDs").is_none());
    let study = instance.main_tags(main_tags::STUDY);
    assert_eq!(study["StudyDate"], "20040119");
    assert_eq!(study["StudyInstanceUID"], series.study_uid());
    let inst = instance.main_tags(main_tags::INSTANCE);
    assert_eq!(inst["InstanceNumber"], "2");
    assert_eq!(
        inst["ImagePositionPatient"],
        "-158.135803\\-179.035797\\-75.699997"
    );
}
