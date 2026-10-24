use pacs_core::Instance;
use pacs_core::frames::FrameError;
use pacs_core::synthetic::SyntheticSeries;

const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

fn le_bytes(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.to_le_bytes()).collect()
}

#[test]
fn a_native_single_frame_is_the_whole_pixel_data() {
    let series = SyntheticSeries::new(41, 1);
    let instance = Instance::from_bytes(&series.slice_bytes(0)).unwrap();
    let frame = instance.frame(1).unwrap();
    assert_eq!(frame.bytes, le_bytes(&series.pixels(0, 0)));
    assert_eq!(frame.transfer_syntax_uid, "1.2.840.10008.1.2.1");
    assert_eq!(frame.media_type, "application/octet-stream");
    assert_eq!(instance.frame(2), Err(FrameError::OutOfRange(2, 1)));
    assert_eq!(instance.frame(0), Err(FrameError::OutOfRange(0, 1)));
}

#[test]
fn native_multiframe_is_cut_per_frame() {
    let series = SyntheticSeries::new(42, 1);
    let instance = Instance::from_bytes(&series.multiframe_bytes(0, 3)).unwrap();
    assert_eq!(instance.number_of_frames(), 3);
    for f in 0..3 {
        assert_eq!(
            instance.frame(f + 1).unwrap().bytes,
            le_bytes(&series.pixels(0, f))
        );
    }
}

#[test]
fn one_fragment_per_frame() {
    let series = SyntheticSeries::new(43, 1);
    let fragments = vec![vec![1u8, 1], vec![2, 2, 2, 2], vec![3, 3]];
    let bytes = series.encapsulated_bytes(0, JPEG_LS, 3, vec![], fragments.clone());
    let instance = Instance::from_bytes(&bytes).unwrap();
    let frame = instance.frame(2).unwrap();
    assert_eq!(frame.bytes, fragments[1]);
    assert_eq!(frame.transfer_syntax_uid, JPEG_LS);
    assert_eq!(frame.media_type, "image/jls");
}

#[test]
fn a_single_frame_in_several_fragments_is_joined() {
    let series = SyntheticSeries::new(44, 1);
    let bytes = series.encapsulated_bytes(0, JPEG_LS, 1, vec![], vec![vec![1, 2], vec![3, 4]]);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.frame(1).unwrap().bytes, [1, 2, 3, 4]);
}

#[test]
fn the_basic_offset_table_maps_fragments_to_frames() {
    let series = SyntheticSeries::new(45, 1);
    // frame 1 = fragments 0 and 1, frame 2 = fragment 2; each item has an 8-byte header
    let fragments = vec![vec![1u8, 1], vec![2, 2], vec![3, 3, 3, 3]];
    let offsets = vec![0, (8 + 2) + (8 + 2)];
    let bytes = series.encapsulated_bytes(0, JPEG_LS, 2, offsets, fragments);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.frame(1).unwrap().bytes, [1, 1, 2, 2]);
    assert_eq!(instance.frame(2).unwrap().bytes, [3, 3, 3, 3]);
}

#[test]
fn without_an_offset_table_ambiguous_fragments_are_refused() {
    let series = SyntheticSeries::new(46, 1);
    let fragments = vec![vec![1u8, 1], vec![2, 2], vec![3, 3]];
    let bytes = series.encapsulated_bytes(0, JPEG_LS, 2, vec![], fragments);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert!(matches!(instance.frame(1), Err(FrameError::Unsupported(_))));
}
