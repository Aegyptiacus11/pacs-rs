//! Elements written as UN (PS3.5 6.2.2) and lengths that lie, on files written byte by byte.
//! The sequence cases follow dcm4che's `DicomInputStreamTest.testUNSequence*`.

use std::borrow::Cow;

use dicom_core::{Tag, VR};
use dicom_dictionary_std::{tags, uids};
use pacs_core::synthetic::{
    ITEM, RawFile, SEQUENCE_END, SyntheticSeries, UNDEFINED, even, implicit,
    implicit_reference_sequence, item, un_sequence_file, un_study_uid_file,
};
use pacs_core::{Instance, wire};

const REFERENCED_STUDY: Tag = Tag(0x0008, 0x1110);
/// Private Group Reference: a US in the dictionary, as dcm4che's sequence cases use.
const NESTED_US: (u16, u16) = (0x0008, 0x0301);

fn explicit_le() -> RawFile {
    RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN).identifiers(1, b"UI")
}

fn nested_us(instance: &Instance, sequence: Tag) -> Vec<u16> {
    let items = instance.object.get(sequence).unwrap().items().unwrap();
    items[0]
        .element(Tag(NESTED_US.0, NESTED_US.1))
        .unwrap()
        .uint16_slice()
        .unwrap()
        .to_vec()
}

#[test]
fn standard_sequence_in_un_undefined_length() {
    let instance = Instance::from_bytes(&un_sequence_file(5)).unwrap();
    assert_eq!(instance.key.study_instance_uid, "2.25.51");
    assert_eq!(instance.key.series_instance_uid, "2.25.52");
    assert_eq!(instance.key.sop_instance_uid, "2.25.53");
    assert_eq!(instance.key.transfer_syntax_uid, uids::JPEG_LOSSLESS_SV1);
    let element = instance.object.get(REFERENCED_STUDY).unwrap();
    assert_eq!(element.vr(), VR::SQ);
    let items = element.items().unwrap();
    assert_eq!(items.len(), 1);
    let reference = items[0].element(tags::REFERENCED_SOP_INSTANCE_UID).unwrap();
    assert_eq!(reference.vr(), VR::UI);
    assert_eq!(
        reference.to_str().unwrap().trim_end_matches('\0'),
        "2.25.54"
    );
    // The elements after it and the pixel data are still there.
    assert_eq!(
        instance
            .object
            .get(tags::PIXEL_DATA)
            .unwrap()
            .fragments()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn standard_uid_in_un_defined_length() {
    let instance = Instance::from_bytes(&un_study_uid_file(5)).unwrap();
    assert_eq!(instance.key.study_instance_uid, "2.25.51");
    assert_eq!(
        instance.object.get(tags::STUDY_INSTANCE_UID).unwrap().vr(),
        VR::UI
    );
}

#[test]
fn dicom_json_shows_the_dictionary_vr() {
    let json = Instance::from_bytes(&un_sequence_file(5))
        .unwrap()
        .metadata_json()
        .unwrap();
    assert_eq!(json["00081110"]["vr"], "SQ");
    assert_eq!(json["00081110"]["Value"][0]["00081155"]["vr"], "UI");
    assert_eq!(
        json["00081110"]["Value"][0]["00081155"]["Value"][0],
        "2.25.54"
    );
}

#[test]
fn sequence_in_un_of_defined_length_holds_implicit_vr() {
    let content = item(&implicit(NESTED_US, &[0x23, 0x01]));
    let bytes = explicit_le()
        .long((0x0008, 0x1110), b"UN", &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, REFERENCED_STUDY), [0x0123]);
}

#[test]
fn sequence_in_un_holding_explicit_vr_is_read_as_such() {
    // Not what 6.2.2 says, but written in the field; dcm4che: testUNSequenceEVR_LE_UN_EVR_LE.
    let mut us = vec![0x08, 0x00, 0x01, 0x03];
    us.extend(b"US");
    us.extend([2, 0, 0x23, 0x01]);
    let mut content = item(&us);
    content.extend(SEQUENCE_END);
    let bytes = explicit_le()
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, REFERENCED_STUDY), [0x0123]);
}

#[test]
fn big_endian_file_with_implicit_little_endian_un() {
    // dcm4che: testUNSequenceEVR_BE_UN_IVR_LE.
    let mut content = item(&implicit(NESTED_US, &[0x23, 0x01]));
    content.extend(SEQUENCE_END);
    let bytes = RawFile::big_endian()
        .identifiers(1, b"UI")
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, REFERENCED_STUDY), [0x0123]);
}

#[test]
fn big_endian_file_with_big_endian_sequence_in_un() {
    // dcm4che: testUNSequenceEVR_BE_UN_EVR_BE.
    let content: Vec<u8> = [
        &[0xFF, 0xFE, 0xE0, 0x00, 0, 0, 0, 10][..],
        &[0x00, 0x08, 0x03, 0x01],
        b"US",
        &[0, 2, 0x01, 0x23],
        &[0xFF, 0xFE, 0xE0, 0xDD, 0, 0, 0, 0],
    ]
    .concat();
    let bytes = RawFile::big_endian()
        .identifiers(1, b"UI")
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, REFERENCED_STUDY), [0x0123]);
}

#[test]
fn big_endian_un_number_stays_little_endian_until_reverted() {
    // PS3.5 6.2.2: a value in UN is not byte-swapped, whatever the file's order.
    let bytes = RawFile::big_endian()
        .identifiers(1, b"UI")
        .long((0x0028, 0x0010), b"UN", &[0x00, 0x02])
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.int(tags::ROWS), Some(512));
}

#[test]
fn sequence_nested_inside_sequence_in_un() {
    let inner = {
        let mut bytes = item(&implicit(NESTED_US, &[0x23, 0x01]));
        bytes.extend(SEQUENCE_END);
        bytes
    };
    let mut outer = (0x0008u16).to_le_bytes().to_vec();
    outer.extend(0x1115u16.to_le_bytes());
    outer.extend(UNDEFINED.to_le_bytes());
    outer.extend(inner);
    let mut content = item(&outer);
    content.extend(SEQUENCE_END);
    let bytes = explicit_le()
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    let items = instance
        .object
        .get(REFERENCED_STUDY)
        .unwrap()
        .items()
        .unwrap();
    let nested = items[0].element(Tag(0x0008, 0x1115)).unwrap();
    assert_eq!(nested.vr(), VR::SQ);
    assert_eq!(nested.items().unwrap().len(), 1);
}

#[test]
fn un_inside_an_explicit_sequence_of_defined_length() {
    // The reverted element changes the size of what holds it, so the lengths must not survive.
    let mut un = vec![0x08, 0x00, 0x55, 0x11];
    un.extend(b"UN");
    un.extend([0, 0]);
    let uid = even("2.25.99");
    un.extend((uid.len() as u32).to_le_bytes());
    un.extend(&uid);
    let mut sequence = ITEM.to_vec();
    sequence.extend((un.len() as u32).to_le_bytes());
    sequence.extend(&un);
    let bytes = explicit_le()
        .long((0x0008, 0x1115), b"SQ", &sequence)
        .short((0x0020, 0x0013), b"IS", b"7 ")
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    let items = instance
        .object
        .get(Tag(0x0008, 0x1115))
        .unwrap()
        .items()
        .unwrap();
    let reference = items[0].element(tags::REFERENCED_SOP_INSTANCE_UID).unwrap();
    assert_eq!(reference.vr(), VR::UI);
    assert_eq!(instance.int(tags::INSTANCE_NUMBER), Some(7));
}

#[test]
fn private_un_of_defined_length_stays_as_it_was() {
    let bytes = explicit_le()
        .short((0x0043, 0x0010), b"LO", b"GEMS_PARM_01")
        .long((0x0043, 0x1001), b"UN", b"\x01\x02\x03\x04")
        .bytes();
    assert!(matches!(wire::checked(&bytes).unwrap(), Cow::Borrowed(_)));
    let instance = Instance::from_bytes(&bytes).unwrap();
    let private = instance.object.get(Tag(0x0043, 0x1001)).unwrap();
    assert_eq!(private.vr(), VR::UN);
    assert_eq!(private.to_bytes().unwrap().as_ref(), b"\x01\x02\x03\x04");
}

#[test]
fn private_un_of_undefined_length_is_an_implicit_sequence() {
    let mut content = item(&implicit(NESTED_US, &[0x23, 0x01]));
    content.extend(SEQUENCE_END);
    let bytes = explicit_le()
        .short((0x0043, 0x0010), b"LO", b"GEMS_PARM_01")
        .long_declared((0x0043, 0x1001), b"UN", UNDEFINED, &content)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, Tag(0x0043, 0x1001)), [0x0123]);
}

#[test]
fn un_too_long_for_the_16_bit_length_stays_un() {
    // PS3.5 6.2.2: this is what UN is for.
    let long = vec![b'A'; 70_000];
    let bytes = explicit_le()
        .long((0x0008, 0x1030), b"UN", &long)
        .short((0x0020, 0x0013), b"IS", b"7 ")
        .long((0x0028, 0x0010), b"UN", &[1, 0])
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(
        instance.object.get(tags::STUDY_DESCRIPTION).unwrap().vr(),
        VR::UN
    );
    assert_eq!(instance.int(tags::INSTANCE_NUMBER), Some(7));
    assert_eq!(instance.int(tags::ROWS), Some(1));
}

#[test]
fn un_of_a_size_its_vr_does_not_come_in_stays_un() {
    let bytes = explicit_le()
        .long((0x0028, 0x0010), b"UN", &[1, 0, 0])
        .short((0x0028, 0x0100), b"US", &16u16.to_le_bytes())
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.object.get(tags::ROWS).unwrap().vr(), VR::UN);
    assert_eq!(instance.int(tags::BITS_ALLOCATED), Some(16));
}

#[test]
fn sequence_in_un_of_defined_length_that_does_not_parse_stays_un() {
    let bytes = explicit_le()
        .long((0x0008, 0x1110), b"UN", b"not a sequence at all!")
        .short((0x0020, 0x0013), b"IS", b"7 ")
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.object.get(REFERENCED_STUDY).unwrap().vr(), VR::UN);
    assert_eq!(instance.int(tags::INSTANCE_NUMBER), Some(7));
}

#[test]
fn us_or_ss_in_un_goes_by_pixel_representation() {
    // dcm4che reads the same list by Pixel Representation.
    for (representation, vr) in [(0u16, VR::US), (1, VR::SS)] {
        let bytes = explicit_le()
            .short((0x0028, 0x0103), b"US", &representation.to_le_bytes())
            .long((0x0028, 0x0106), b"UN", &[0xFF, 0xFF])
            .bytes();
        let instance = Instance::from_bytes(&bytes).unwrap();
        assert_eq!(
            instance
                .object
                .get(tags::SMALLEST_IMAGE_PIXEL_VALUE)
                .unwrap()
                .vr(),
            vr
        );
    }
}

#[test]
fn native_pixel_data_in_un_reads_as_ow() {
    let bytes = explicit_le()
        .long((0x7FE0, 0x0010), b"UN", &[1, 2, 3, 4])
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.object.get(tags::PIXEL_DATA).unwrap().vr(), VR::OW);
}

#[test]
fn a_file_without_un_comes_back_as_the_same_bytes() {
    for bytes in [
        SyntheticSeries::new(3, 1).slice_bytes(0),
        RawFile::new(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .raw(&implicit((0x0020, 0x000D), &even("2.25.1")))
            .bytes(),
    ] {
        match wire::checked(&bytes).unwrap() {
            Cow::Borrowed(same) => assert_eq!(same.as_ptr(), bytes.as_ptr()),
            Cow::Owned(_) => panic!("copied a file with nothing to revert"),
        }
    }
}

#[test]
fn implicit_vr_file_is_walked_and_read() {
    let mut sequence = item(&implicit(NESTED_US, &[0x23, 0x01]));
    sequence.extend(SEQUENCE_END);
    let mut data = implicit((0x0008, 0x0016), &even(uids::CT_IMAGE_STORAGE));
    data.extend(implicit((0x0008, 0x0018), &even("2.25.3")));
    data.extend(0x0008u16.to_le_bytes());
    data.extend(0x1110u16.to_le_bytes());
    data.extend(UNDEFINED.to_le_bytes());
    data.extend(&sequence);
    data.extend(implicit((0x0020, 0x000D), &even("2.25.1")));
    data.extend(implicit((0x0020, 0x000E), &even("2.25.2")));
    let bytes = RawFile::new(uids::IMPLICIT_VR_LITTLE_ENDIAN)
        .raw(&data)
        .bytes();
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(nested_us(&instance, REFERENCED_STUDY), [0x0123]);
}

// Refusals, each before dicom-rs allocates.

fn refused(bytes: &[u8]) -> String {
    match wire::checked(bytes) {
        Err(e) => {
            let message = e.to_string();
            assert!(
                message.starts_with("not a readable DICOM file: "),
                "{message}"
            );
            assert!(Instance::from_bytes(bytes).is_err());
            message
        }
        Ok(_) => panic!("accepted"),
    }
}

const LIE: u32 = 0xC000_0000;

#[test]
fn a_value_longer_than_the_file_is_refused_for_every_long_vr() {
    for vr in [
        b"OB", b"OD", b"OF", b"OL", b"OV", b"OW", b"UC", b"UN", b"UR", b"UT", b"XX",
    ] {
        let bytes = explicit_le()
            .long_declared((0x0043, 0x1001), vr, LIE, b"data")
            .bytes();
        let message = refused(&bytes);
        assert!(message.contains("3221225472"), "{message}");
    }
}

#[test]
fn a_length_in_a_big_endian_or_implicit_file_is_refused_too() {
    refused(
        &RawFile::big_endian()
            .identifiers(1, b"UI")
            .long_declared((0x0043, 0x1001), b"OB", LIE, b"data")
            .bytes(),
    );
    let mut element = implicit((0x0043, 0x1001), b"data");
    element[4..8].copy_from_slice(&LIE.to_le_bytes());
    refused(
        &RawFile::new(uids::IMPLICIT_VR_LITTLE_ENDIAN)
            .raw(&element)
            .bytes(),
    );
}

#[test]
fn a_length_inside_an_item_is_refused() {
    let mut element = (0x0043u16).to_le_bytes().to_vec();
    element.extend(0x1001u16.to_le_bytes());
    element.extend(b"OB");
    element.extend([0, 0]);
    element.extend(LIE.to_le_bytes());
    let mut content = item(&element);
    content.extend(SEQUENCE_END);
    refused(
        &explicit_le()
            .long_declared((0x0008, 0x1115), b"SQ", UNDEFINED, &content)
            .bytes(),
    );
}

#[test]
fn a_length_past_its_item_is_refused() {
    let element = implicit((0x0043, 0x1001), b"data");
    let mut sequence = ITEM.to_vec();
    sequence.extend(((element.len() - 2) as u32).to_le_bytes());
    sequence.extend(&element);
    let mut content = sequence;
    content.extend(SEQUENCE_END);
    refused(
        &explicit_le()
            .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
            .bytes(),
    );
}

#[test]
fn an_item_longer_than_the_file_is_refused() {
    let mut content = ITEM.to_vec();
    content.extend(LIE.to_le_bytes());
    refused(
        &explicit_le()
            .long_declared((0x0008, 0x1115), b"SQ", UNDEFINED, &content)
            .bytes(),
    );
}

#[test]
fn an_offset_table_longer_than_the_file_is_refused() {
    let mut content = ITEM.to_vec();
    content.extend(LIE.to_le_bytes());
    refused(
        &explicit_le()
            .long_declared((0x7FE0, 0x0010), b"OB", UNDEFINED, &content)
            .bytes(),
    );
}

#[test]
fn a_fragment_cut_short_stays_readable_at_the_size_it_has() {
    // dicom-rs keeps what is there of a fragment, so such files were and stay accepted.
    let mut content = ITEM.to_vec();
    content.extend(0u32.to_le_bytes());
    content.extend(ITEM);
    content.extend(LIE.to_le_bytes());
    content.extend([0xFF, 0xD8]);
    let bytes = explicit_le()
        .long_declared((0x7FE0, 0x0010), b"OB", UNDEFINED, &content)
        .bytes();
    assert!(matches!(wire::checked(&bytes).unwrap(), Cow::Owned(_)));
    let instance = Instance::from_bytes(&bytes).unwrap();
    let pixels = instance.object.get(tags::PIXEL_DATA).unwrap();
    assert_eq!(pixels.fragments().unwrap()[0], [0xFF, 0xD8]);
}

#[test]
fn a_file_meta_length_longer_than_the_file_is_refused() {
    let mut bytes = RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN).bytes();
    // (0002,0102) Private Information, the last of group 2, added by hand.
    let mut extra = vec![0x02, 0x00, 0x02, 0x01];
    extra.extend(b"OB");
    extra.extend([0, 0]);
    extra.extend(LIE.to_le_bytes());
    let group_length_at = 132 + 8;
    let group_length = u32::from_le_bytes(
        bytes[group_length_at..group_length_at + 4]
            .try_into()
            .unwrap(),
    );
    bytes[group_length_at..group_length_at + 4]
        .copy_from_slice(&(group_length + extra.len() as u32).to_le_bytes());
    bytes.extend(extra);
    refused(&bytes);
}

#[test]
fn implicit_content_cut_short_is_refused() {
    let whole = implicit_reference_sequence("2.25.4");
    let bytes = explicit_le()
        .long_declared(
            (0x0008, 0x1110),
            b"UN",
            UNDEFINED,
            &whole[..whole.len() - 20],
        )
        .bytes();
    refused(&bytes);
}

#[test]
fn something_else_where_an_item_belongs_is_refused() {
    let bytes = explicit_le()
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, b"this is not an item.")
        .bytes();
    refused(&bytes);
}

#[test]
fn nesting_too_deep_is_refused_not_a_stack_overflow() {
    let mut level = 0x0008u16.to_le_bytes().to_vec();
    level.extend(0x1115u16.to_le_bytes());
    level.extend(UNDEFINED.to_le_bytes());
    level.extend(ITEM);
    level.extend(UNDEFINED.to_le_bytes());
    let mut content = ITEM.to_vec();
    content.extend(UNDEFINED.to_le_bytes());
    content.extend(level.repeat(200_000));
    let bytes = explicit_le()
        .long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &content)
        .bytes();
    assert!(refused(&bytes).contains("nested deeper"));
}

#[test]
fn what_is_not_part_10_passes_for_dicom_rs_to_refuse() {
    for bytes in [&b""[..], b"hello", &[0u8; 300], b"DICM"] {
        assert!(matches!(wire::checked(bytes).unwrap(), Cow::Borrowed(_)));
        assert!(Instance::from_bytes(bytes).is_err());
    }
}

#[test]
fn a_missing_uid_is_still_refused_after_reverting() {
    let bytes = RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
        .long((0x0020, 0x000D), b"UN", &even("2.25.1"))
        .bytes();
    let message = Instance::from_bytes(&bytes).err().unwrap().to_string();
    assert!(message.contains("missing required attribute"), "{message}");
}

#[test]
fn a_uid_that_is_not_one_is_refused_after_reverting() {
    let bad = RawFile::new(uids::EXPLICIT_VR_LITTLE_ENDIAN)
        .short((0x0008, 0x0016), b"UI", &even(uids::CT_IMAGE_STORAGE))
        .short((0x0008, 0x0018), b"UI", &even("2.25.3"))
        .long((0x0020, 0x000D), b"UN", b"../../etc/passwd")
        .short((0x0020, 0x000E), b"UI", &even("2.25.2"))
        .bytes();
    let message = Instance::from_bytes(&bad).err().unwrap().to_string();
    assert!(message.contains("not a valid UID"), "{message}");
}

/// Whatever the bytes, an answer: no panic, and no time or memory going to a length.
#[test]
fn cut_or_damaged_files_get_an_answer() {
    for file in [
        un_sequence_file(5),
        un_study_uid_file(5),
        RawFile::big_endian()
            .identifiers(1, b"UN")
            .long_declared(
                (0x0008, 0x1110),
                b"UN",
                UNDEFINED,
                &implicit_reference_sequence("2.25.4"),
            )
            .one_fragment()
            .bytes(),
    ] {
        for cut in 0..file.len() {
            let _ = Instance::from_bytes(&file[..cut]);
        }
        for at in 132..file.len() {
            for flip in [0x01, 0x80, 0xFF] {
                let mut damaged = file.clone();
                damaged[at] ^= flip;
                let _ = Instance::from_bytes(&damaged);
            }
        }
    }
}

// What a file costs to read.

#[test]
fn read_cost_counts_elements_items_and_values() {
    let cost = |file: RawFile| wire::read_cost(&file.bytes()).unwrap();
    let empty = cost(explicit_le());
    assert_eq!(
        cost(explicit_le().short((0x0020, 0x0013), b"IS", b"7 ")),
        empty + 840
    );
    assert_eq!(
        cost(explicit_le().short((0x3006, 0x0050), b"DS", b"1\\2\\3 ")),
        empty + 840 + 2 * 170
    );
    // Numbers and bytes stay as they came, whatever the size.
    assert_eq!(
        cost(explicit_le().long((0x7FE0, 0x0010), b"OW", &[0x5C; 4096])),
        empty + 840
    );
    // A backslash in text that is one value.
    assert_eq!(
        cost(explicit_le().long((0x0008, 0x0119), b"UT", b"a\\b ")),
        empty + 840
    );
    let mut sequence = item(&implicit(NESTED_US, &[0x23, 0x01]));
    sequence.extend(SEQUENCE_END);
    assert_eq!(
        cost(explicit_le().long_declared((0x0008, 0x1110), b"UN", UNDEFINED, &sequence)),
        empty + 840 + 110 + 840
    );
}

#[test]
fn read_cost_refuses_what_checked_refuses_and_passes_what_it_passes() {
    assert_eq!(wire::read_cost(b"not dicom").unwrap(), 0);
    let lying = explicit_le()
        .long_declared((0x0043, 0x1001), b"OB", LIE, b"data")
        .bytes();
    assert!(wire::read_cost(&lying).is_err());
}

fn bare(file: RawFile) -> Vec<u8> {
    let bytes = file.bytes();
    let length = u32::from_le_bytes(bytes[140..144].try_into().unwrap()) as usize;
    bytes[144 + length..].to_vec()
}

#[test]
fn a_bare_data_set_is_checked_in_its_transfer_syntax() {
    const NO_LIMIT: u64 = u64::MAX;
    let ts = uids::EXPLICIT_VR_LITTLE_ENDIAN;
    let plain = bare(explicit_le());
    assert!(matches!(
        wire::checked_data_set(&plain, ts, NO_LIMIT).unwrap(),
        Cow::Borrowed(_)
    ));
    let un = bare(RawFile::new(ts).long((0x0020, 0x000D), b"UN", &even("2.25.1")));
    assert!(matches!(
        wire::checked_data_set(&un, ts, NO_LIMIT).unwrap(),
        Cow::Owned(_)
    ));
    let lying = bare(explicit_le().long_declared((0x0043, 0x1001), b"OB", LIE, b"data"));
    assert!(wire::checked_data_set(&lying, ts, NO_LIMIT).is_err());
    // Read as implicit VR these bytes are other elements, and the walk goes by what it is told.
    let implicit_lying = {
        let mut element = implicit((0x0043, 0x1001), b"data");
        element[4..8].copy_from_slice(&LIE.to_le_bytes());
        element
    };
    assert!(
        wire::checked_data_set(&implicit_lying, uids::IMPLICIT_VR_LITTLE_ENDIAN, NO_LIMIT).is_err()
    );
    // Deflated bytes say nothing; dicom-rs decides.
    let deflated = uids::DEFLATED_EXPLICIT_VR_LITTLE_ENDIAN;
    assert!(matches!(
        wire::checked_data_set(b"zzz", deflated, NO_LIMIT).unwrap(),
        Cow::Borrowed(_)
    ));
}

#[test]
fn a_bare_data_set_over_its_limit_is_refused() {
    let ts = uids::EXPLICIT_VR_LITTLE_ENDIAN;
    let bytes = bare(explicit_le().long((0x0043, 0x1001), b"UC", &[b'\\'; 1000]));
    let cost = wire::read_cost(
        &explicit_le()
            .long((0x0043, 0x1001), b"UC", &[b'\\'; 1000])
            .bytes(),
    )
    .unwrap();
    assert!(wire::checked_data_set(&bytes, ts, cost).is_ok());
    let message = wire::checked_data_set(&bytes, ts, cost - 1)
        .unwrap_err()
        .to_string();
    assert!(message.contains("over the limit"), "{message}");
}
