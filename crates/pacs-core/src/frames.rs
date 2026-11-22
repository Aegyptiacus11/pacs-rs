//! Frames of pixel data, as WADO-RS retrieves them: in the stored transfer syntax, without
//! decoding anything.
//!
//! Native (uncompressed) pixel data is cut into frames of Rows x Columns x Samples per
//! Pixel x Bits Allocated / 8 bytes and labelled Explicit VR Little Endian, the only
//! transfer syntax PS3.18 Table 8.7.3-4 gives uncompressed bulk data. Encapsulated pixel
//! data is returned fragment by fragment, mapped to frames by the fragment count, the
//! Extended Offset Table or the Basic Offset Table.
//!
//! [`Instance::frame`] cuts a frame out of a whole file. [`FrameLayout`] is where the frames
//! of a stored file are, so that one can be read from the store without the rest: it is
//! made from the start of the file ([`crate::wire::pixel_data_at`], [`FrameLayout::plan`])
//! and, for encapsulated pixel data, a few bytes more ([`FrameSearch`]). It is made only
//! for a file in which [`Instance::frame`] would find the same bytes; for any other there
//! is none, and the whole file is read.

use std::ops::Range;

use dicom_core::DicomValue;
use dicom_dictionary_std::{tags, uids};

use crate::Instance;
use crate::codec::Shape;
use crate::wire::PixelDataAt;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("the instance has no pixel data")]
    NoPixelData,
    #[error("frame {0} does not exist; the instance has {1} frame(s)")]
    OutOfRange(u32, u32),
    #[error("cannot cut frames: {0}")]
    Unsupported(String),
}

/// One frame and how it is encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub bytes: Vec<u8>,
    pub transfer_syntax_uid: String,
    /// The bulk data media type (PS3.18 Tables 8.7.3-4 and 8.7.3-5).
    pub media_type: &'static str,
}

/// The media type of pixel data in transfer syntax `ts`, per PS3.18 Table 8.7.3-5;
/// `None` for one that has no bulk data media type (video, for instance).
pub fn compressed_media_type(ts: &str) -> Option<&'static str> {
    Some(match ts {
        "1.2.840.10008.1.2.4.50"
        | "1.2.840.10008.1.2.4.51"
        | "1.2.840.10008.1.2.4.57"
        | "1.2.840.10008.1.2.4.70" => "image/jpeg",
        "1.2.840.10008.1.2.5" => "image/dicom-rle",
        "1.2.840.10008.1.2.4.80" | "1.2.840.10008.1.2.4.81" => "image/jls",
        "1.2.840.10008.1.2.4.90" | "1.2.840.10008.1.2.4.91" => "image/jp2",
        "1.2.840.10008.1.2.4.92" | "1.2.840.10008.1.2.4.93" => "image/jpx",
        "1.2.840.10008.1.2.4.201" | "1.2.840.10008.1.2.4.202" | "1.2.840.10008.1.2.4.203" => {
            "image/jphc"
        }
        "1.2.840.10008.1.2.4.110" | "1.2.840.10008.1.2.4.111" | "1.2.840.10008.1.2.4.112" => {
            "image/jxl"
        }
        // PS3.18 Table 8.7.3-5: frames of no image format are octet streams, their transfer
        // syntax named
        "1.2.840.10008.1.2.8.1" => "application/octet-stream",
        _ => return None,
    })
}

pub const NATIVE_MEDIA_TYPE: &str = "application/octet-stream";

impl Instance {
    /// Number of Frames, 1 when absent.
    pub fn number_of_frames(&self) -> u32 {
        self.int(tags::NUMBER_OF_FRAMES)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|&n| n > 0)
            .unwrap_or(1)
    }

    /// Frame `number`, counting from 1 as WADO-RS does.
    pub fn frame(&self, number: u32) -> Result<Frame, FrameError> {
        let frames = self.number_of_frames();
        if number == 0 || number > frames {
            return Err(FrameError::OutOfRange(number, frames));
        }
        let element = self
            .object
            .get(tags::PIXEL_DATA)
            .ok_or(FrameError::NoPixelData)?;
        match element.value() {
            DicomValue::Primitive(value) => {
                let bytes = value.to_bytes();
                let len = self.native_frame_len()?;
                let start = (number as usize - 1) * len;
                let frame = bytes.get(start..start + len).ok_or_else(|| {
                    FrameError::Unsupported(format!(
                        "pixel data has {} bytes, too few for {frames} frame(s) of {len}",
                        bytes.len()
                    ))
                })?;
                Ok(Frame {
                    bytes: frame.to_vec(),
                    transfer_syntax_uid: uids::EXPLICIT_VR_LITTLE_ENDIAN.to_owned(),
                    media_type: NATIVE_MEDIA_TYPE,
                })
            }
            DicomValue::PixelSequence(sequence) => {
                let ts = &self.key.transfer_syntax_uid;
                let media_type = compressed_media_type(ts).ok_or_else(|| {
                    FrameError::Unsupported(format!("no frame media type for transfer syntax {ts}"))
                })?;
                let fragments = sequence.fragments();
                let bytes = encapsulated_frame(
                    fragments,
                    sequence.offset_table(),
                    self.extended_offsets().as_deref(),
                    frames,
                    number,
                )?;
                Ok(Frame {
                    bytes,
                    transfer_syntax_uid: ts.clone(),
                    media_type,
                })
            }
            DicomValue::Sequence(_) => Err(FrameError::Unsupported(
                "pixel data is a sequence of items".into(),
            )),
        }
    }

    fn native_frame_len(&self) -> Result<usize, FrameError> {
        let get = |tag, name: &str| {
            self.int(tag)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| FrameError::Unsupported(format!("{name} missing")))
        };
        let rows = get(tags::ROWS, "Rows")?;
        let columns = get(tags::COLUMNS, "Columns")?;
        let samples = self
            .int(tags::SAMPLES_PER_PIXEL)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(1);
        let bits = get(tags::BITS_ALLOCATED, "Bits Allocated")?;
        if bits % 8 != 0 {
            if self.number_of_frames() == 1 {
                return Ok((rows * columns * samples * bits).div_ceil(8));
            }
            return Err(FrameError::Unsupported(format!(
                "{bits}-bit frames are not byte-aligned"
            )));
        }
        Ok(rows * columns * samples * bits / 8)
    }

    fn extended_offsets(&self) -> Option<Vec<u64>> {
        let bytes = self
            .object
            .get(tags::EXTENDED_OFFSET_TABLE)?
            .to_bytes()
            .ok()?;
        Some(
            bytes
                .as_chunks::<8>()
                .0
                .iter()
                .map(|c| u64::from_le_bytes(*c))
                .collect(),
        )
    }
}

/// The bytes of frame `number` (from 1) of `frames`, from encapsulated fragments.
fn encapsulated_frame(
    fragments: &[Vec<u8>],
    basic_offsets: &[u32],
    extended_offsets: Option<&[u64]>,
    frames: u32,
    number: u32,
) -> Result<Vec<u8>, FrameError> {
    if fragments.is_empty() {
        return Err(FrameError::NoPixelData);
    }
    if frames == 1 {
        return Ok(fragments.concat());
    }
    if fragments.len() == frames as usize {
        return Ok(fragments[number as usize - 1].clone());
    }
    let offsets: Vec<u64> = match extended_offsets {
        Some(ext) if !ext.is_empty() => ext.to_vec(),
        _ if !basic_offsets.is_empty() => basic_offsets.iter().map(|&o| u64::from(o)).collect(),
        _ => {
            return Err(FrameError::Unsupported(format!(
                "{} fragments for {frames} frames and no offset table",
                fragments.len()
            )));
        }
    };
    if offsets.len() != frames as usize {
        return Err(FrameError::Unsupported(format!(
            "offset table has {} entries for {frames} frames",
            offsets.len()
        )));
    }
    // Offsets count bytes from the first fragment's item tag; each item has an
    // 8-byte header (tag and length) before its bytes.
    let start = offsets[number as usize - 1];
    let end = offsets.get(number as usize).copied().unwrap_or(u64::MAX);
    let mut position = 0u64;
    let mut frame = Vec::new();
    let mut found_start = false;
    for fragment in fragments {
        if position == start {
            found_start = true;
        }
        if position >= end {
            break;
        }
        if found_start {
            frame.extend_from_slice(fragment);
        }
        position += 8 + fragment.len() as u64;
    }
    if !found_start {
        return Err(FrameError::Unsupported(format!(
            "offset {start} is not at a fragment boundary"
        )));
    }
    Ok(frame)
}

const ITEM: [u8; 4] = [0xFE, 0xFF, 0x00, 0xE0];
const SEQUENCE_END: [u8; 4] = [0xFE, 0xFF, 0xDD, 0xE0];
const UNDEFINED: u32 = u32::MAX;

/// An item's length, if these eight bytes are the head of one.
fn item_len(head: &[u8]) -> Option<u32> {
    let (tag, len) = head.split_first_chunk::<4>()?;
    let len = u32::from_le_bytes(*len.first_chunk::<4>()?);
    (*tag == ITEM && len != UNDEFINED).then_some(len)
}

/// Where the frames of a stored file are, to read one without the rest of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameLayout {
    frames: u32,
    transfer_syntax_uid: String,
    media_type: &'static str,
    shape: Result<Shape, String>,
    spans: Spans,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Spans {
    /// Native pixel data: frames of `len` bytes from `start`.
    Native { start: u64, len: u64 },
    /// A fragment a frame: where the bytes of each are.
    Fragments(Vec<Range<u64>>),
    /// By an offset table: where the first item of each frame starts. The last frame's
    /// items end with the pixel data, before `end` at the latest.
    Table { starts: Vec<u64>, end: u64 },
}

/// The next step towards a [`FrameLayout`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramePlan {
    Ready(FrameLayout),
    /// More of the file is to be read first.
    Read(FrameSearch),
}

/// The search for the frames of encapsulated pixel data: it [`wants`](FrameSearch::wants)
/// some bytes of the file, and is [given](FrameSearch::give) them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSearch {
    layout: FrameLayout,
    extended_offsets: Option<Vec<u64>>,
    size: u64,
    block: u64,
    step: Step,
}

/// How much of the file is read at once to pass over fragments when no table says where
/// frames start. A read for each fragment's head is as many requests to the store as there
/// are frames; in blocks, small fragments are passed over many at a time, at the cost of
/// reading them.
const FRAGMENT_BLOCK: u64 = 4 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    /// The head of the Basic Offset Table item, at the start of the pixel data.
    TableHead { at: u64 },
    /// The Basic Offset Table.
    Table { at: u64, len: u64 },
    /// The next fragments from `at`, where one starts, when no table says where frames
    /// start: there has to be a fragment a frame. `read` bytes are read: a block, or the
    /// head alone of a fragment after one a block would not have held.
    Fragment {
        at: u64,
        read: u64,
        found: Vec<Range<u64>>,
    },
}

impl FrameLayout {
    /// How to find the frames of a file of `size` bytes, from `header`, the file cut where
    /// its Pixel Data element starts (`at.element`) and read as an instance. `None` when
    /// frames are not to be read on their own: there is one frame, so reading it is
    /// reading the file, or the file is not one to cut without all of it at hand.
    pub fn plan(header: &Instance, at: PixelDataAt, size: u64) -> Option<FramePlan> {
        let frames = header.number_of_frames();
        // Big endian pixel data is not returned as its bytes are in the file.
        if frames == 1 || at.big_endian {
            return None;
        }
        let shape = header.shape().map_err(|e| e.to_string());
        let value = at.value as u64;
        match at.len {
            Some(value_len) => {
                let len = header.native_frame_len().ok()? as u64;
                let end = value.checked_add(u64::from(value_len))?;
                if len == 0
                    || u64::from(frames).checked_mul(len)? > u64::from(value_len)
                    || end > size
                {
                    return None;
                }
                Some(FramePlan::Ready(FrameLayout {
                    frames,
                    transfer_syntax_uid: uids::EXPLICIT_VR_LITTLE_ENDIAN.to_owned(),
                    media_type: NATIVE_MEDIA_TYPE,
                    shape,
                    spans: Spans::Native { start: value, len },
                }))
            }
            None => {
                let ts = &header.key.transfer_syntax_uid;
                let search = FrameSearch {
                    layout: FrameLayout {
                        frames,
                        transfer_syntax_uid: ts.clone(),
                        media_type: compressed_media_type(ts)?,
                        shape,
                        spans: Spans::Fragments(Vec::new()),
                    },
                    extended_offsets: header.extended_offsets().filter(|table| !table.is_empty()),
                    size,
                    block: FRAGMENT_BLOCK,
                    step: Step::TableHead { at: value },
                };
                search.within_file().map(FramePlan::Read)
            }
        }
    }

    /// Number of Frames.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// The pixel layout every frame shares, or why the codecs do not handle it
    /// ([`Instance::shape`]).
    pub fn shape(&self) -> Result<Shape, String> {
        self.shape.clone()
    }

    /// About how many bytes this takes in memory.
    pub fn weight(&self) -> usize {
        size_of::<Self>()
            + self.transfer_syntax_uid.len()
            + match &self.spans {
                Spans::Native { .. } => 0,
                Spans::Fragments(found) => found.len() * size_of::<Range<u64>>(),
                Spans::Table { starts, .. } => starts.len() * size_of::<u64>(),
            }
    }

    /// The bytes of the file to read for frame `number`, counting from 1.
    pub fn range(&self, number: u32) -> Result<Range<u64>, FrameError> {
        if number == 0 || number > self.frames {
            return Err(FrameError::OutOfRange(number, self.frames));
        }
        let index = number as usize - 1;
        Ok(match &self.spans {
            Spans::Native { start, len } => {
                let from = start + index as u64 * len;
                from..from + len
            }
            Spans::Fragments(found) => found[index].clone(),
            Spans::Table { starts, end } => {
                starts[index]..starts.get(index + 1).copied().unwrap_or(*end)
            }
        })
    }

    /// Frame `number` from `bytes`, the bytes of the file in its [`range`](Self::range).
    /// `None` when they are not what the layout says is there, as in a file whose offset
    /// table is wrong: the whole file is to be read then.
    pub fn frame(&self, number: u32, bytes: &[u8]) -> Option<Frame> {
        let range = self.range(number).ok()?;
        if bytes.len() as u64 != range.end - range.start {
            return None;
        }
        let bytes = match &self.spans {
            Spans::Native { .. } | Spans::Fragments(_) => bytes.to_vec(),
            Spans::Table { .. } => items(bytes, number == self.frames)?,
        };
        Some(Frame {
            bytes,
            transfer_syntax_uid: self.transfer_syntax_uid.clone(),
            media_type: self.media_type,
        })
    }
}

/// The fragments in `bytes`, joined: items from its first byte to its last, or, for the
/// `last` frame, to the delimiter that ends the pixel data.
fn items(bytes: &[u8], last: bool) -> Option<Vec<u8>> {
    let mut frame = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    let mut found = false;
    while !rest.is_empty() {
        let (head, after) = rest.split_at_checked(8)?;
        if last && found && head[..4] == SEQUENCE_END {
            return Some(frame);
        }
        let (fragment, after) = after.split_at_checked(item_len(head)? as usize)?;
        frame.extend_from_slice(fragment);
        found = true;
        rest = after;
    }
    // The last frame without its delimiter is a file cut short.
    (found && !last).then_some(frame)
}

impl FrameSearch {
    /// The bytes of the file this step reads.
    pub fn wants(&self) -> Range<u64> {
        match &self.step {
            Step::TableHead { at } => *at..at + 8,
            Step::Table { at, len } => *at..at + len,
            // To the end of the file at the most, and a head at the least: a file that ends
            // before one is not searched further.
            Step::Fragment { at, read, .. } => *at..(at + read).min(self.size).max(at + 8),
        }
    }

    /// The same search passing over fragments `bytes` at a time, from its next step on,
    /// instead of [`FRAGMENT_BLOCK`]; a head, 8 bytes, at the least.
    pub fn in_blocks_of(mut self, bytes: u64) -> Self {
        self.block = bytes.max(8);
        self
    }

    /// The step that passes over the fragments from `at`, the last one passed being of
    /// `last` bytes.
    fn fragments_from(self, at: u64, last: u64, found: Vec<Range<u64>>) -> Option<FramePlan> {
        // Fragments as large as a block are not read to be passed over: their heads alone.
        let read = if last >= self.block { 8 } else { self.block };
        self.step(Step::Fragment { at, read, found })
    }

    /// The search, if what it wants is in the file.
    fn within_file(self) -> Option<Self> {
        (self.wants().end <= self.size).then_some(self)
    }

    fn step(mut self, step: Step) -> Option<FramePlan> {
        self.step = step;
        self.within_file().map(FramePlan::Read)
    }

    /// The next step, given the bytes [wanted](Self::wants). `None` when the frames are not
    /// to be read on their own after all.
    pub fn give(self, bytes: &[u8]) -> Option<FramePlan> {
        let wanted = self.wants();
        if bytes.len() as u64 != wanted.end - wanted.start {
            return None;
        }
        match self.step.clone() {
            Step::TableHead { at } => {
                let len = u64::from(item_len(bytes)?);
                if len % 4 != 0 {
                    return None;
                }
                if len == 0 {
                    return self.tabled(at + 8, &[]);
                }
                self.step(Step::Table { at: at + 8, len })
            }
            Step::Table { at, len } => self.tabled(at + len, bytes),
            Step::Fragment { at, mut found, .. } => {
                let frames = self.layout.frames as usize;
                let (mut rest, mut next, mut last) = (bytes, at, 0);
                // Each head that is whole in what was read; the fragment after it may end
                // past it, and the next read starts where that one ends.
                while let Some((head, after)) = rest.split_at_checked(8) {
                    if head[..4] == SEQUENCE_END {
                        let FrameSearch { mut layout, .. } = self;
                        if found.len() != frames {
                            return None;
                        }
                        layout.spans = Spans::Fragments(found);
                        return Some(FramePlan::Ready(layout));
                    }
                    last = u64::from(item_len(head)?);
                    let end = next + 8 + last;
                    if end > self.size || found.len() == frames {
                        return None;
                    }
                    found.push(next + 8..end);
                    next = end;
                    rest = after.get(last as usize..).unwrap_or_default();
                }
                self.fragments_from(next, last, found)
            }
        }
    }

    /// With the Basic Offset Table read, the first fragment being at `first`.
    fn tabled(self, first: u64, basic_offsets: &[u8]) -> Option<FramePlan> {
        let offsets: Vec<u64> = match &self.extended_offsets {
            Some(extended) => extended.clone(),
            None => basic_offsets
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| u64::from(u32::from_le_bytes(*c)))
                .collect(),
        };
        if offsets.is_empty() {
            return self.fragments_from(first, 0, Vec::new());
        }
        let FrameSearch {
            mut layout, size, ..
        } = self;
        // A table to go by has an entry a frame, the first frame at the first fragment, and
        // each frame after the one before.
        if offsets.len() != layout.frames as usize
            || offsets[0] != 0
            || !offsets.is_sorted_by(|a, b| a < b)
        {
            return None;
        }
        let starts = offsets
            .iter()
            .map(|offset| first.checked_add(*offset).filter(|start| start + 8 <= size))
            .collect::<Option<Vec<u64>>>()?;
        layout.spans = Spans::Table { starts, end: size };
        Some(FramePlan::Ready(layout))
    }
}
