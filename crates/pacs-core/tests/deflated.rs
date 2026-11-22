//! More than 16 bits stored losslessly: each frame deflated (Deflated Image Frame
//! Compression, PS3.5 10.20), on a real RT Dose of 32-bit values.

use pacs_core::Instance;
use pacs_core::codec::{self, DEFLATED_FRAMES};
use pacs_core::real::file_bytes;

#[test]
fn a_32_bit_dose_is_deflated_frame_by_frame_and_comes_back() {
    let original = Instance::from_bytes(&file_bytes("rtdose.dcm")).unwrap();
    let shape = original.shape().unwrap();
    assert_eq!(
        (shape.bits_allocated, original.number_of_frames()),
        (32, 15)
    );
    let deflated =
        Instance::from_bytes(&original.transcoded(DEFLATED_FRAMES, true).unwrap()).unwrap();
    assert_eq!(deflated.key.transfer_syntax_uid, DEFLATED_FRAMES);
    for number in 1..=15 {
        let frame = deflated.frame(number).unwrap();
        assert_eq!(frame.transfer_syntax_uid, DEFLATED_FRAMES);
        assert_eq!(
            codec::decode(DEFLATED_FRAMES, shape, &frame.bytes).unwrap(),
            original.native_frame(number, shape).unwrap(),
            "frame {number}"
        );
    }
    // and back to native, as a client that does not inflate gets it
    let back =
        Instance::from_bytes(&deflated.transcoded(codec::EXPLICIT_VR_LE, false).unwrap()).unwrap();
    assert_eq!(
        back.native_frame(7, shape).unwrap(),
        original.native_frame(7, shape).unwrap()
    );
}

#[test]
fn jpeg_ls_and_jpeg_xl_refuse_more_than_16_bits() {
    let original = Instance::from_bytes(&file_bytes("rtdose.dcm")).unwrap();
    for target in [codec::JPEG_LS_LOSSLESS, codec::JPEG_XL_LOSSLESS] {
        assert!(original.transcoded(target, true).is_err(), "{target}");
    }
}
