use pacs_core::Instance;
use pacs_core::synthetic::SyntheticSeries;

#[test]
fn a_synthetic_series_parses_back_to_its_identifiers() {
    let series = SyntheticSeries::new(7, 3);
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
    let bytes = SyntheticSeries::new(8, 1).slice_bytes(0);
    assert_eq!(&bytes[128..132], b"DICM");
    let with = Instance::from_bytes(&bytes).unwrap();
    let without = Instance::from_bytes(&bytes[128..]).unwrap();
    assert_eq!(with.key, without.key);
}

#[test]
fn dicom_json_carries_the_identifiers_and_vrs() {
    let series = SyntheticSeries::new(9, 1);
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
        "SYNTHETIC^SERIES"
    );
    assert_eq!(json["00280010"]["Value"][0], 4);
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
