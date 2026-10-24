//! Frames of pixel data, as WADO-RS retrieves them: in the stored transfer syntax, without
//! decoding anything.
//!
//! Native (uncompressed) pixel data is cut into frames of Rows x Columns x Samples per
//! Pixel x Bits Allocated / 8 bytes and labelled Explicit VR Little Endian, the only
//! transfer syntax PS3.18 Table 8.7.3-4 gives uncompressed bulk data. Encapsulated pixel
//! data is returned fragment by fragment, mapped to frames by the fragment count, the
//! Extended Offset Table or the Basic Offset Table.

use dicom_core::DicomValue;
use dicom_dictionary_std::{tags, uids};

use crate::Instance;

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
