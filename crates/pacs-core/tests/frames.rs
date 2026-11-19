use pacs_core::Instance;
use pacs_core::frames::FrameError;
use pacs_core::real::RealSeries;

const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";

fn le_bytes(pixels: &[u16]) -> Vec<u8> {
    pixels.iter().flat_map(|p| p.to_le_bytes()).collect()
}

#[test]
fn a_native_single_frame_is_the_whole_pixel_data() {
    let series = RealSeries::new(41, 1);
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
    let series = RealSeries::new(42, 1);
    let instance = Instance::from_bytes(&series.multiframe_bytes(0)).unwrap();
    assert_eq!(instance.number_of_frames(), RealSeries::FRAMES);
    for f in 0..RealSeries::FRAMES {
        assert_eq!(
            instance.frame(f + 1).unwrap().bytes,
            le_bytes(&RealSeries::frame_pixels(f))
        );
    }
}

#[test]
fn one_fragment_per_frame() {
    // the file as it is: ten frames, a fragment each, an empty offset table
    let series = RealSeries::new(43, 1);
    let fragments = RealSeries::jpeg_ls_frames();
    let instance = Instance::from_bytes(&series.jpeg_ls_multiframe_bytes(0)).unwrap();
    assert_eq!(instance.number_of_frames(), RealSeries::FRAMES);
    let frame = instance.frame(2).unwrap();
    assert_eq!(frame.bytes, fragments[1]);
    assert_ne!(fragments[1], fragments[0]);
    assert_eq!(frame.transfer_syntax_uid, JPEG_LS);
    assert_eq!(frame.media_type, "image/jls");
}

#[test]
fn a_single_frame_in_several_fragments_is_joined() {
    let series = RealSeries::new(44, 1);
    let whole = RealSeries::jpeg_ls_frames().remove(0);
    let (head, tail) = whole.split_at(1000);
    let bytes = series.refragmented(0, 1, vec![], vec![head.to_vec(), tail.to_vec()]);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.frame(1).unwrap().bytes, whole);
}

#[test]
fn the_basic_offset_table_maps_fragments_to_frames() {
    let series = RealSeries::new(45, 1);
    // frame 1 = fragments 0 and 1, frame 2 = fragment 2; each item has an 8-byte header
    let frames = RealSeries::jpeg_ls_frames();
    let (head, tail) = frames[0].split_at(1000);
    let fragments = vec![head.to_vec(), tail.to_vec(), frames[1].clone()];
    let offsets = vec![0, (8 + head.len() as u32) + (8 + tail.len() as u32)];
    let bytes = series.refragmented(0, 2, offsets, fragments);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert_eq!(instance.frame(1).unwrap().bytes, frames[0]);
    assert_eq!(instance.frame(2).unwrap().bytes, frames[1]);
}

#[test]
fn without_an_offset_table_ambiguous_fragments_are_refused() {
    let series = RealSeries::new(46, 1);
    let frames = RealSeries::jpeg_ls_frames();
    let (head, tail) = frames[0].split_at(1000);
    let fragments = vec![head.to_vec(), tail.to_vec(), frames[1].clone()];
    let bytes = series.refragmented(0, 2, vec![], fragments);
    let instance = Instance::from_bytes(&bytes).unwrap();
    assert!(matches!(instance.frame(1), Err(FrameError::Unsupported(_))));
}
