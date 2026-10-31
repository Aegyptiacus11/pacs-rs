//! The Command Set encoding (PS3.7 6.3.1, Annex E), status classes (PS3.7 Annex C), AE
//! titles (PS3.5 Table 6.2-1) and the file meta information of received instances
//! (PS3.10 7.1).

use pacs_dimse::command::{self, Command, CommandError, NO_DATA_SET, field, tag};
use pacs_dimse::scp::part10;
use pacs_dimse::{AeError, AeTitle, RemoteAe, Status, StatusClass, VERIFICATION};

/// PS3.7 6.3.1: Implicit VR Little Endian, Command Group Length first, tags increasing.
/// The C-ECHO-RQ of Table 9.3-12, byte for byte.
#[test]
fn c_echo_rq_is_encoded_as_ps3_7_says() {
    let bytes = command::c_echo_rq(7).encode();
    let mut expected = Vec::new();
    let element = |out: &mut Vec<u8>, e: u16, v: &[u8]| {
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&e.to_le_bytes());
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v);
    };
    let mut body = Vec::new();
    element(&mut body, 0x0002, b"1.2.840.10008.1.1\0");
    element(&mut body, 0x0100, &0x0030u16.to_le_bytes());
    element(&mut body, 0x0110, &7u16.to_le_bytes());
    element(&mut body, 0x0800, &0x0101u16.to_le_bytes());
    element(&mut expected, 0x0000, &(body.len() as u32).to_le_bytes());
    expected.extend_from_slice(&body);
    assert_eq!(bytes, expected);
}

#[test]
fn a_command_set_reads_back_as_written() {
    let written = command::c_echo_rq(9);
    let read = Command::decode(&written.encode()).unwrap();
    assert_eq!(read.command_field(), Ok(field::C_ECHO_RQ));
    assert_eq!(read.message_id(), Ok(9));
    assert_eq!(read.ui(tag::AFFECTED_SOP_CLASS_UID), Ok(VERIFICATION));
    assert_eq!(read.has_data_set(), Ok(false));
}

/// PS3.7 6.3.1: "ordered by increasing Command Element Tag number", each at most once.
#[test]
fn tags_out_of_order_are_refused() {
    let mut bytes = Command::new().with_us(0x0110, 1).encode()[12..].to_vec();
    bytes.extend_from_slice(&Command::new().with_us(0x0100, 1).encode()[12..]);
    assert_eq!(
        Command::decode(&bytes),
        Err(CommandError::OutOfOrder(0x0100, 0x0110))
    );
}

/// PS3.7 6.3.1: "All Command Element Tags have the Group Number 0000."
#[test]
fn a_data_element_in_the_command_set_is_refused() {
    let bytes = [0x08, 0x00, 0x16, 0x00, 2, 0, 0, 0, b'1', 0];
    assert_eq!(
        Command::decode(&bytes),
        Err(CommandError::WrongGroup(0x0016, 0x0008))
    );
}

/// Table E.1-1: Command Group Length is the number of bytes after it.
#[test]
fn a_wrong_group_length_is_refused() {
    let mut bytes = command::c_echo_rq(1).encode();
    bytes[8] += 2;
    assert!(matches!(
        Command::decode(&bytes),
        Err(CommandError::GroupLength { .. })
    ));
}

#[test]
fn a_truncated_command_set_is_refused() {
    let bytes = command::c_echo_rq(1).encode();
    assert_eq!(
        Command::decode(&bytes[..bytes.len() - 1]),
        Err(CommandError::Truncated)
    );
}

/// PS3.5 6.2: UI values are padded to even length with NUL, text values with a space.
#[test]
fn values_are_padded_to_even_length() {
    let c = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, "1.2.3")
        .with_text(tag::ERROR_COMMENT, "abc");
    assert_eq!(c.get(tag::AFFECTED_SOP_CLASS_UID), Some(&b"1.2.3\0"[..]));
    assert_eq!(c.get(tag::ERROR_COMMENT), Some(&b"abc "[..]));
    assert_eq!(c.ui(tag::AFFECTED_SOP_CLASS_UID), Ok("1.2.3"));
}

/// PS3.7 9.3.1.2, 9.3.5.2: the response's Command Field has bit 15 set, it answers the
/// request's Message ID, carries no data set and the request's affected UIDs.
#[test]
fn a_response_answers_its_request() {
    let rq = Command::new()
        .with_ui(tag::AFFECTED_SOP_CLASS_UID, "1.2.840.10008.5.1.4.1.1.2")
        .with_us(tag::COMMAND_FIELD, field::C_STORE_RQ)
        .with_us(tag::MESSAGE_ID, 42)
        .with_us(tag::COMMAND_DATA_SET_TYPE, 0)
        .with_ui(tag::AFFECTED_SOP_INSTANCE_UID, "2.25.1");
    let rsp = command::response(&rq, Status::SUCCESS, None);
    assert_eq!(rsp.command_field(), Ok(field::C_STORE_RSP));
    assert_eq!(rsp.us(tag::MESSAGE_ID_BEING_RESPONDED_TO), Ok(42));
    assert_eq!(rsp.us(tag::COMMAND_DATA_SET_TYPE), Ok(NO_DATA_SET));
    assert_eq!(rsp.status(), Ok(Status::SUCCESS));
    assert_eq!(
        rsp.ui(tag::AFFECTED_SOP_CLASS_UID),
        Ok("1.2.840.10008.5.1.4.1.1.2")
    );
    assert_eq!(rsp.ui(tag::AFFECTED_SOP_INSTANCE_UID), Ok("2.25.1"));
    assert!(rsp.get(tag::MESSAGE_ID).is_none());
}

/// Error Comment is LO: at most 64 characters, no backslash (PS3.5 Table 6.2-1).
#[test]
fn an_error_comment_fits_lo() {
    let rq = command::c_echo_rq(1);
    let long = format!("a\\b{}", "x".repeat(100));
    let rsp = command::response(&rq, Status::UNRECOGNIZED_OPERATION, Some(&long));
    let comment = rsp.text(tag::ERROR_COMMENT).unwrap();
    assert_eq!(comment.len(), 64);
    assert!(!comment.contains('\\'));
}

/// PS3.7 Annex C: the Status Class of each code range.
#[test]
fn status_classes_follow_annex_c() {
    use StatusClass::*;
    for (code, class) in [
        (0x0000, Success),
        (0x0001, Warning),
        (0x0107, Warning),
        (0x0116, Warning),
        (0xB000, Warning),
        (0xB007, Warning),
        (0x0122, Failure),
        (0x0211, Failure),
        (0xA700, Failure),
        (0xA900, Failure),
        (0xC000, Failure),
        (0xFE00, Cancel),
        (0xFF00, Pending),
        (0xFF01, Pending),
        (0x0300, Unknown),
        (0xD000, Unknown),
    ] {
        assert_eq!(Status(code).class(), class, "{code:04X}");
    }
}

/// PS3.4 Table B.2-1: pacs-rs's C-STORE failure codes are in the ranges it allows.
#[test]
fn c_store_codes_are_in_their_table_b_2_1_ranges() {
    for (status, range) in [
        (Status::OUT_OF_RESOURCES_STORE, 0xA700..=0xA7FF),
        (Status::OUT_OF_RESOURCES_INDEX, 0xA700..=0xA7FF),
        (Status::OUT_OF_RESOURCES_TOO_LARGE, 0xA700..=0xA7FF),
        (Status::DATA_SET_DOES_NOT_MATCH_SOP_CLASS, 0xA900..=0xA9FF),
        (Status::CANNOT_UNDERSTAND, 0xC000..=0xCFFF),
        (Status::CANNOT_UNDERSTAND_INSTANCE_UID, 0xC000..=0xCFFF),
        (Status::CANNOT_UNDERSTAND_CONFLICT, 0xC000..=0xCFFF),
    ] {
        assert!(range.contains(&status.0), "{status}");
    }
}

/// PS3.5 Table 6.2-1, AE: 16 characters at most, no backslash or control characters, not
/// only spaces; leading and trailing spaces are not significant.
#[test]
fn ae_titles_follow_the_ae_vr() {
    assert_eq!(AeTitle::new(" PACSRS ").unwrap().as_str(), "PACSRS");
    assert!(AeTitle::new("PACSRS").unwrap().matches("PACSRS          "));
    assert_eq!(AeTitle::new("    "), Err(AeError::Empty));
    assert!(matches!(
        AeTitle::new("A23456789012345678"),
        Err(AeError::TooLong(_))
    ));
    assert!(matches!(
        AeTitle::new("A\\B"),
        Err(AeError::BadCharacter(_))
    ));
    assert!(matches!(
        AeTitle::new("A\tB"),
        Err(AeError::BadCharacter(_))
    ));
    assert!(AeTitle::new("16_CHARACTERS_OK").is_ok());
}

#[test]
fn remote_aes_are_read_from_id_title_host_port() {
    let r: RemoteAe = "hospital=HOSP_PACS@10.0.0.5:104".parse().unwrap();
    assert_eq!(
        (r.id.as_str(), r.ae_title.as_str(), r.host.as_str(), r.port),
        ("hospital", "HOSP_PACS", "10.0.0.5", 104)
    );
    let v6: RemoteAe = "ws=WS1@[::1]:11112".parse().unwrap();
    assert_eq!((v6.host.as_str(), v6.port), ("::1", 11112));
    assert!("HOSP@host:104".parse::<RemoteAe>().is_err());
    assert!("h=HOSP@host:0".parse::<RemoteAe>().is_err());
    assert!("h=HOSP@host".parse::<RemoteAe>().is_err());
}

/// PS3.10 7.1: preamble, "DICM", group 0002 in Explicit VR Little Endian with its length,
/// Media Storage SOP Class and Instance UIDs from the request, the negotiated transfer
/// syntax, and the data set unchanged after it.
#[test]
fn a_received_data_set_becomes_a_part_10_file() {
    let series = pacs_core::synthetic::SyntheticSeries::new(3, 1);
    let original = series.slice_bytes(0);
    let meta_length = u32::from_le_bytes(original[140..144].try_into().unwrap()) as usize;
    let data_set = &original[144 + meta_length..];
    let file = part10(
        data_set,
        "1.2.840.10008.5.1.4.1.1.2",
        &series.instance_uid(0),
        "1.2.840.10008.1.2.1",
        "STORESCU",
        "PACSRS",
    );
    assert!(file[..128].iter().all(|b| *b == 0));
    assert!(file.ends_with(data_set));
    let instance = pacs_core::Instance::from_bytes(&file).unwrap();
    assert_eq!(instance.key.sop_instance_uid, series.instance_uid(0));
    assert_eq!(instance.key.transfer_syntax_uid, "1.2.840.10008.1.2.1");
    let meta = instance.object.meta();
    assert_eq!(
        meta.media_storage_sop_class_uid(),
        "1.2.840.10008.5.1.4.1.1.2"
    );
    assert_eq!(
        meta.implementation_class_uid(),
        pacs_dimse::IMPLEMENTATION_CLASS_UID
    );
    assert_eq!(meta.information_version, [0, 1]);
}
