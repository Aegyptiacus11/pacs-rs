//! A whole instance in another transfer syntax: every frame decoded and encoded again, the
//! data set otherwise as it was.

use dicom_core::value::PixelFragmentSequence;
use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;

use crate::Instance;
use crate::codec::{self, CodecError, Shape};
use crate::frames::FrameError;

#[derive(Debug, thiserror::Error)]
pub enum TranscodeError {
    /// Not something this server transcodes: colour, 32-bit, an unknown syntax...
    #[error("cannot transcode: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Frame(#[from] FrameError),
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// A frame encoded, then decoded, did not give back the original pixels.
    #[error("frame {0} did not decode back to the original pixels")]
    NotLossless(u32),
    #[error("writing the transcoded file: {0}")]
    Write(String),
}

impl Instance {
    /// The pixel layout every frame shares, if this instance is one the codecs handle:
    /// monochrome or palette colour of one sample per pixel, or RGB or YBR_FULL of three,
    /// 8 or 16 bits allocated, or 32 for monochrome. The planar configuration is that of the native frames; a
    /// compressed frame decodes by pixel.
    pub fn shape(&self) -> Result<Shape, TranscodeError> {
        let unsupported = |what: String| TranscodeError::Unsupported(what);
        let get = |tag, name: &str| {
            self.int(tag)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| unsupported(format!("{name} missing")))
        };
        let samples = self.int(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
        let photometric = self
            .string(tags::PHOTOMETRIC_INTERPRETATION)
            .unwrap_or_default();
        // PS3.3 C.7.6.3.1.2; YBR_FULL_422 and the other subsampled ones are not a native
        // frame of three full planes, and are left as they are
        let known = matches!(
            (samples, photometric.as_str()),
            (1, "MONOCHROME1" | "MONOCHROME2" | "PALETTE COLOR") | (3, "RGB" | "YBR_FULL")
        );
        if !known {
            return Err(unsupported(format!(
                "{samples} samples per pixel, photometric interpretation {photometric:?}"
            )));
        }
        let planar = codec::is_native(&self.key.transfer_syntax_uid)
            && self.int(tags::PLANAR_CONFIGURATION) == Some(1);
        let shape = Shape::new(
            get(tags::ROWS, "Rows")?,
            get(tags::COLUMNS, "Columns")?,
            get(tags::BITS_ALLOCATED, "Bits Allocated")?,
            get(tags::BITS_STORED, "Bits Stored")?,
        )
        .with_samples(samples as u16, planar);
        // 32 bits: monochrome only, as RT Dose and other values beyond 16 bits are
        let fits = match shape.bits_allocated {
            8 | 16 => true,
            32 => shape.samples == 1,
            _ => false,
        };
        if !fits {
            return Err(unsupported(format!(
                "{} bits allocated, {} samples per pixel",
                shape.bits_allocated, shape.samples
            )));
        }
        Ok(shape)
    }

    /// Frame `number` (from 1) as native little-endian pixels.
    pub fn native_frame(&self, number: u32, shape: Shape) -> Result<Vec<u8>, TranscodeError> {
        let frame = self.frame(number)?;
        Ok(codec::decode(
            &frame.transfer_syntax_uid,
            shape,
            &frame.bytes,
        )?)
    }

    /// This instance as a DICOM Part 10 file in transfer syntax `target`. With `verify`,
    /// each encoded frame is decoded again and compared with the original pixels, and any
    /// difference is an error rather than a file.
    pub fn transcoded(&self, target: &str, verify: bool) -> Result<Vec<u8>, TranscodeError> {
        if !codec::can_encode(target) {
            return Err(TranscodeError::Unsupported(format!(
                "no encoder for transfer syntax {target}"
            )));
        }
        if !codec::can_decode(&self.key.transfer_syntax_uid) {
            return Err(TranscodeError::Unsupported(format!(
                "no decoder for transfer syntax {}",
                self.key.transfer_syntax_uid
            )));
        }
        let shape = self.shape()?;
        let frames = self.number_of_frames();
        let mut object = self.object.clone();
        if codec::is_native(target) {
            let mut pixels = Vec::with_capacity(shape.frame_len() * frames as usize);
            for number in 1..=frames {
                pixels.extend(self.native_frame(number, shape)?);
            }
            let vr = if shape.bits_allocated == 8 {
                VR::OB
            } else {
                VR::OW
            };
            object.put(DataElement::new(
                tags::PIXEL_DATA,
                vr,
                PrimitiveValue::U8(pixels.into()),
            ));
        } else {
            let mut fragments = Vec::with_capacity(frames as usize);
            for number in 1..=frames {
                let native = self.native_frame(number, shape)?;
                let encoded = codec::encode(target, shape, &native)?;
                // what comes back is by pixel, as the original is once interleaved
                if verify
                    && codec::decode(target, shape, &encoded)? != *codec::by_pixel(shape, &native)
                {
                    return Err(TranscodeError::NotLossless(number));
                }
                fragments.push(pad_even(encoded));
            }
            // One fragment per frame, so an empty Basic Offset Table would be enough to find
            // each (PS3.5 A.4), but only by passing over every fragment before it: with the
            // table, a frame of a stored file is found without reading the others
            // (`FrameLayout`). A table that cannot hold the offsets is left empty.
            let table = if frames > 1 {
                basic_offsets(fragments.iter().map(|f| f.len() as u64)).unwrap_or_default()
            } else {
                Vec::new()
            };
            object.put(DataElement::new(
                tags::PIXEL_DATA,
                VR::OB,
                PixelFragmentSequence::new(table, fragments),
            ));
        }
        // colour by pixel: compressed (PS3.5 8.2.3, 8.2.4), or decoded from a compressed one
        if shape.samples > 1 && !(codec::is_native(target) && shape.planar) {
            object.put(DataElement::new(
                tags::PLANAR_CONFIGURATION,
                VR::US,
                PrimitiveValue::from(0u16),
            ));
        }
        object.remove_element(tags::EXTENDED_OFFSET_TABLE);
        object.remove_element(tags::EXTENDED_OFFSET_TABLE_LENGTHS);
        object.update_meta(|meta| meta.transfer_syntax = target.to_owned());
        let mut bytes = Vec::new();
        object
            .write_all(&mut bytes)
            .map_err(|e| TranscodeError::Write(e.to_string()))?;
        Ok(bytes)
    }
}

/// The Basic Offset Table of frames of one fragment each, of these lengths: for each
/// frame, the bytes from the first fragment's item to its own, an item being its 8-byte
/// head and its fragment (PS3.5 A.4). `None` when an offset does not fit its 32 bits.
fn basic_offsets(fragment_lens: impl Iterator<Item = u64>) -> Option<Vec<u32>> {
    let mut at = 0u64;
    fragment_lens
        .map(|len| {
            let offset = u32::try_from(at).ok();
            at = at.saturating_add(8).saturating_add(len);
            offset
        })
        .collect()
}

/// Fragments have an even length; a codestream of odd length gets a trailing zero, which
/// JPEG-LS and JPEG XL decoders ignore.
fn pad_even(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use dicom_core::DicomValue;
    use dicom_dictionary_std::uids;

    use super::*;
    use crate::codec::{EXPLICIT_VR_LE, JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS};
    use crate::real::RealSeries;

    fn series() -> RealSeries {
        RealSeries::new(7, 1)
    }

    #[test]
    fn every_way_round_and_back() {
        let original = Instance::from_bytes(&series().multiframe_bytes(0)).unwrap();
        let shape = original.shape().unwrap();
        let reference: Vec<Vec<u8>> = (1..=RealSeries::FRAMES)
            .map(|n| original.native_frame(n, shape).unwrap())
            .collect();
        for first in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS, EXPLICIT_VR_LE] {
            let once = Instance::from_bytes(&original.transcoded(first, true).unwrap()).unwrap();
            assert_eq!(once.key.transfer_syntax_uid, first);
            assert_eq!(once.key.sop_instance_uid, original.key.sop_instance_uid);
            for second in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS, EXPLICIT_VR_LE] {
                let twice = Instance::from_bytes(&once.transcoded(second, true).unwrap()).unwrap();
                assert_eq!(twice.key.transfer_syntax_uid, second);
                for (n, expected) in (1..=RealSeries::FRAMES).zip(&reference) {
                    assert_eq!(&twice.native_frame(n, shape).unwrap(), expected);
                }
            }
        }
    }

    #[test]
    fn frames_come_out_one_per_fragment() {
        let original = Instance::from_bytes(&series().multiframe_bytes(0)).unwrap();
        let jls =
            Instance::from_bytes(&original.transcoded(JPEG_LS_LOSSLESS, true).unwrap()).unwrap();
        let frame = jls.frame(2).unwrap();
        assert_eq!(frame.media_type, "image/jls");
        assert_eq!(frame.transfer_syntax_uid, JPEG_LS_LOSSLESS);
    }

    #[test]
    fn compressed_frames_are_found_by_the_offset_table() {
        use crate::frames::{FrameLayout, FramePlan};
        use crate::wire::{PixelDataSearch, pixel_data_at};

        let original = Instance::from_bytes(&series().multiframe_bytes(0)).unwrap();
        for ts in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS] {
            let file = original.transcoded(ts, true).unwrap();
            let whole = Instance::from_bytes(&file).unwrap();
            let DicomValue::PixelSequence(sequence) =
                whole.object.element(tags::PIXEL_DATA).unwrap().value()
            else {
                panic!("{ts}: encapsulated pixel data");
            };
            // an entry a frame: where its item starts, from the first
            let mut at = 0;
            let expected: Vec<u32> = sequence
                .fragments()
                .iter()
                .map(|fragment| {
                    let offset = at;
                    at += 8 + fragment.len() as u32;
                    offset
                })
                .collect();
            assert_eq!(sequence.offset_table(), expected.as_slice(), "{ts}");
            assert_eq!(expected.len(), RealSeries::FRAMES as usize);

            // so the layout takes two reads, the table's head and the table, however many
            // frames there are, and each frame by it is the frame of the whole file
            let PixelDataSearch::Found(found) = pixel_data_at(&file) else {
                panic!("{ts}: pixel data");
            };
            let header = Instance::from_bytes(&file[..found.element]).unwrap();
            let mut plan = FrameLayout::plan(&header, found, file.len() as u64).unwrap();
            let mut reads = 0;
            let layout = loop {
                match plan {
                    FramePlan::Ready(layout) => break layout,
                    FramePlan::Read(search) => {
                        reads += 1;
                        let wanted = search.wants();
                        let part = &file[wanted.start as usize..wanted.end as usize];
                        plan = search.give(part).unwrap();
                    }
                }
            };
            assert_eq!(reads, 2, "{ts}");
            for number in 1..=RealSeries::FRAMES {
                let range = layout.range(number).unwrap();
                let part = &file[range.start as usize..range.end as usize];
                assert_eq!(
                    layout.frame(number, part),
                    whole.frame(number).ok(),
                    "{ts}: frame {number}"
                );
            }
        }

        // one frame is the file: no table to find it by
        let single = Instance::from_bytes(&series().slice_bytes(0)).unwrap();
        let file = single.transcoded(JPEG_LS_LOSSLESS, true).unwrap();
        let whole = Instance::from_bytes(&file).unwrap();
        let DicomValue::PixelSequence(sequence) =
            whole.object.element(tags::PIXEL_DATA).unwrap().value()
        else {
            panic!("encapsulated pixel data");
        };
        assert!(sequence.offset_table().is_empty());
    }

    #[test]
    fn offsets_past_32_bits_leave_the_table_empty() {
        assert_eq!(basic_offsets([].into_iter()), Some(vec![]));
        assert_eq!(
            basic_offsets([10, 20, 4].into_iter()),
            Some(vec![0, 18, 46])
        );
        // the last frame may start at the last offset there is, and end past it
        let most = u64::from(u32::MAX) - 8;
        assert_eq!(
            basic_offsets([most, 1 << 40].into_iter()),
            Some(vec![0, u32::MAX])
        );
        assert_eq!(basic_offsets([most + 1, 2].into_iter()), None);
        assert_eq!(basic_offsets([2, most, 2].into_iter()), None);
        assert_eq!(basic_offsets([u64::MAX, u64::MAX, 2].into_iter()), None);
    }

    #[test]
    fn decoders_take_the_padding_byte() {
        let shape = Shape::new(5, 7, 16, 12);
        for ts in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS] {
            let odd = (0..200u16).find(|seed| {
                let native: Vec<u8> = (0..35u16)
                    .flat_map(|i| ((i * 37 + seed * 101) % 4096).to_le_bytes())
                    .collect();
                let encoded = codec::encode(ts, shape, &native).unwrap();
                let padded = pad_even(encoded.clone());
                assert_eq!(codec::decode(ts, shape, &padded).unwrap(), native, "{ts}");
                encoded.len() % 2 == 1
            });
            assert!(odd.is_some(), "{ts}: no codestream of odd length to check");
        }
    }

    #[test]
    fn refuses_colour_and_unknown_syntaxes() {
        // a real image in lossy JPEG, which pacs-rs has no codec for
        let bytes = series().jpeg_lossy_bytes(0);
        let instance = Instance::from_bytes(&bytes).unwrap();
        assert!(matches!(
            instance.transcoded(JPEG_LS_LOSSLESS, true),
            Err(TranscodeError::Unsupported(_))
        ));
        let native = Instance::from_bytes(&series().slice_bytes(0)).unwrap();
        assert!(matches!(
            native.transcoded(uids::JPEG_BASELINE8_BIT, true),
            Err(TranscodeError::Unsupported(_))
        ));
    }
}
