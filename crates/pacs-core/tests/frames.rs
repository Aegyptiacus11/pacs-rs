use pacs_core::Instance;
use pacs_core::frames::FrameError;
use pacs_core::real::{AlteredFile, RealSeries};

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

// Frames read on their own, from parts of the file: the same bytes as from the whole file,
// or no layout at all.

use pacs_core::frames::{Frame, FrameLayout, FramePlan};
use pacs_core::wire::{PixelDataAt, PixelDataSearch, pixel_data_at};

fn part(file: &[u8], range: std::ops::Range<u64>) -> &[u8] {
    &file[range.start as usize..range.end as usize]
}

fn found(file: &[u8]) -> PixelDataAt {
    match pixel_data_at(file) {
        PixelDataSearch::Found(at) => at,
        other => panic!("{other:?}"),
    }
}

/// The layout of a file, reading of it only what each step wants.
fn layout_from(file: &[u8], header: &Instance, at: PixelDataAt) -> Option<FrameLayout> {
    let mut plan = FrameLayout::plan(header, at, file.len() as u64)?;
    loop {
        match plan {
            FramePlan::Ready(layout) => return Some(layout),
            FramePlan::Read(search) => {
                let wanted = search.wants();
                plan = search.give(part(file, wanted))?;
            }
        }
    }
}

fn layout_of(file: &[u8]) -> Option<FrameLayout> {
    let at = found(file);
    let header = Instance::from_bytes(&file[..at.element]).unwrap();
    layout_from(file, &header, at)
}

fn alone(file: &[u8], layout: &FrameLayout, number: u32) -> Option<Frame> {
    layout.frame(number, part(file, layout.range(number).unwrap()))
}

/// Every frame read alone is the frame cut from the whole file.
fn assert_same_frames(file: &[u8]) -> FrameLayout {
    let whole = Instance::from_bytes(file).unwrap();
    let layout = layout_of(file).expect("a layout");
    assert_eq!(layout.frames(), whole.number_of_frames());
    for number in 1..=layout.frames() {
        assert_eq!(
            alone(file, &layout, number).as_ref(),
            whole.frame(number).as_ref().ok(),
            "frame {number}"
        );
    }
    assert_eq!(layout.shape().ok(), whole.shape().ok());
    assert_eq!(
        layout.range(0),
        Err(FrameError::OutOfRange(0, layout.frames()))
    );
    assert_eq!(
        layout.range(layout.frames() + 1),
        Err(FrameError::OutOfRange(layout.frames() + 1, layout.frames()))
    );
    layout
}

#[test]
fn the_pixel_data_is_found_from_the_start_of_the_file() {
    let file = RealSeries::new(60, 1).multiframe_bytes(0);
    let at = found(&file);
    assert_eq!(&file[at.element..at.element + 4], [0xE0, 0x7F, 0x10, 0x00]);
    let frame_len = le_bytes(&RealSeries::frame_pixels(0)).len() as u32;
    assert_eq!(at.len, Some(frame_len * RealSeries::FRAMES));
    assert_eq!(at.value + at.len.unwrap() as usize, file.len());
    assert!(!at.big_endian);
    // its header is enough, and nothing less is
    assert_eq!(pixel_data_at(&file[..at.value]), PixelDataSearch::Found(at));
    assert_eq!(pixel_data_at(&file[..at.value - 1]), PixelDataSearch::NotIn);
    assert_eq!(
        pixel_data_at(&file[..at.element / 2]),
        PixelDataSearch::NotIn
    );
    // a start that does not hold the File Meta group is not one to read more of
    assert_eq!(pixel_data_at(&file[..200]), PixelDataSearch::Unreadable);

    let jls = RealSeries::new(60, 1).jpeg_ls_multiframe_bytes(0);
    assert_eq!(found(&jls).len, None);
    assert!(found(&AlteredFile::big_endian().identifiers(60, b"UI").bytes()).big_endian);

    // what is not a Part 10 file is not searched
    assert_eq!(pixel_data_at(&file[132..]), PixelDataSearch::Unreadable);
    assert_eq!(pixel_data_at(b""), PixelDataSearch::Unreadable);
    // a file without pixel data
    let header = RealSeries::new(60, 1).header_bytes(0);
    assert_eq!(pixel_data_at(&header), PixelDataSearch::NotIn);
}

#[test]
fn native_frames_alone_are_the_frames_of_the_whole_file() {
    let file = RealSeries::new(61, 1).multiframe_bytes(0);
    let layout = assert_same_frames(&file);
    // one frame's bytes are read for a frame, not the pixel data
    let range = layout.range(4).unwrap();
    assert_eq!(
        range.end - range.start,
        le_bytes(&RealSeries::frame_pixels(3)).len() as u64
    );
}

#[test]
fn a_fragment_a_frame_alone_without_an_offset_table() {
    // the file as it is: ten frames, a fragment each, an empty offset table
    let file = RealSeries::new(62, 1).jpeg_ls_multiframe_bytes(0);
    let layout = assert_same_frames(&file);
    let fragments = RealSeries::jpeg_ls_frames();
    assert_eq!(
        alone(&file, &layout, 2).unwrap().bytes,
        fragments[1],
        "the fragment itself"
    );
}

/// Two frames in three fragments: the first frame is the first two.
fn two_frames_in_three() -> (Vec<Vec<u8>>, u32) {
    let mut frames = RealSeries::jpeg_ls_frames();
    let (second, first) = (frames.remove(1), frames.remove(0));
    let (head, tail) = first.split_at(1000);
    let offset = (8 + head.len() + 8 + tail.len()) as u32;
    (vec![head.to_vec(), tail.to_vec(), second], offset)
}

#[test]
fn frames_alone_by_the_basic_offset_table() {
    let (fragments, offset) = two_frames_in_three();
    let file = RealSeries::new(63, 1).refragmented(0, 2, vec![0, offset], fragments.clone());
    let layout = assert_same_frames(&file);
    assert_eq!(
        alone(&file, &layout, 1).unwrap().bytes,
        [fragments[0].clone(), fragments[1].clone()].concat()
    );
    assert_eq!(alone(&file, &layout, 2).unwrap().bytes, fragments[2]);
    // bytes that are not the frame's are not taken for it
    let range = layout.range(1).unwrap();
    assert_eq!(
        layout.frame(1, part(&file, range.start..range.end - 2)),
        None
    );
    assert_eq!(layout.frame(2, part(&file, range)), None);
}

#[test]
fn frames_alone_by_the_extended_offset_table() {
    use dicom_core::{DataElement, PrimitiveValue, VR};
    use dicom_dictionary_std::tags;

    let (fragments, offset) = two_frames_in_three();
    // an empty basic table, as PS3.5 A.4 asks beside an extended one
    let file = RealSeries::new(64, 1).refragmented(0, 2, vec![], fragments.clone());
    let at = found(&file);
    let mut header = Instance::from_bytes(&file[..at.element]).unwrap();
    assert_eq!(layout_from(&file, &header, at), None, "no table yet");
    header.object.put(DataElement::new(
        tags::EXTENDED_OFFSET_TABLE,
        VR::OV,
        PrimitiveValue::U64(vec![0, u64::from(offset)].into()),
    ));
    let layout = layout_from(&file, &header, at).expect("a layout");
    assert_eq!(
        alone(&file, &layout, 1).unwrap().bytes,
        [fragments[0].clone(), fragments[1].clone()].concat()
    );
    assert_eq!(alone(&file, &layout, 2).unwrap().bytes, fragments[2]);
}

#[test]
fn no_layout_for_a_file_to_read_whole() {
    let series = RealSeries::new(65, 1);
    // one frame: the frame is the file
    assert_eq!(layout_of(&series.slice_bytes(0)), None);
    assert_eq!(layout_of(&series.jpeg_ls_bytes(0)), None);

    let (fragments, offset) = two_frames_in_three();
    let refragmented =
        |frames, table: Vec<u32>| series.refragmented(0, frames, table, fragments.clone());
    // fragments that are not one a frame, and no table
    assert_eq!(layout_of(&refragmented(2, vec![])), None);
    assert_eq!(layout_of(&refragmented(4, vec![])), None);
    // a table that is not one entry a frame, does not start at the first fragment, goes
    // back, or points past the file
    assert_eq!(layout_of(&refragmented(2, vec![0])), None);
    assert_eq!(
        layout_of(&refragmented(2, vec![0, offset, offset + 8])),
        None
    );
    assert_eq!(layout_of(&refragmented(2, vec![8, offset])), None);
    assert_eq!(layout_of(&refragmented(2, vec![0, 0])), None);
    assert_eq!(layout_of(&refragmented(2, vec![0, u32::MAX - 1])), None);

    // a table that points inside a fragment: a layout, but no frame from it
    let file = refragmented(2, vec![0, offset - 4]);
    let layout = layout_of(&file).expect("a layout");
    assert_eq!(alone(&file, &layout, 1), None);
    assert_eq!(alone(&file, &layout, 2), None);

    // big endian pixel data is not what is returned
    let native = series.multiframe_bytes(0);
    let at = found(&native);
    let header = Instance::from_bytes(&native[..at.element]).unwrap();
    assert!(layout_from(&native, &header, at).is_some());
    let big = PixelDataAt {
        big_endian: true,
        ..at
    };
    assert_eq!(layout_from(&native, &header, big), None);

    // a file cut short of its last frame, native or encapsulated
    let cut = &native[..native.len() - 2];
    assert_eq!(layout_from(cut, &header, at), None);
    let jls = series.jpeg_ls_multiframe_bytes(0);
    assert_eq!(layout_of(&jls[..jls.len() - 8]), None, "no delimiter");
    assert_eq!(layout_of(&jls[..jls.len() - 200]), None);
}
