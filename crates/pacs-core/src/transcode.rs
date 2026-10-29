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
    /// one sample per pixel, monochrome, 8 or 16 bits allocated.
    pub fn shape(&self) -> Result<Shape, TranscodeError> {
        let unsupported = |what: String| TranscodeError::Unsupported(what);
        let get = |tag, name: &str| {
            self.int(tag)
                .and_then(|n| u16::try_from(n).ok())
                .ok_or_else(|| unsupported(format!("{name} missing")))
        };
        let samples = self.int(tags::SAMPLES_PER_PIXEL).unwrap_or(1);
        if samples != 1 {
            return Err(unsupported(format!("{samples} samples per pixel")));
        }
        let photometric = self
            .string(tags::PHOTOMETRIC_INTERPRETATION)
            .unwrap_or_default();
        if !matches!(photometric.as_str(), "MONOCHROME1" | "MONOCHROME2") {
            return Err(unsupported(format!(
                "photometric interpretation {photometric:?}"
            )));
        }
        let shape = Shape::new(
            get(tags::ROWS, "Rows")?,
            get(tags::COLUMNS, "Columns")?,
            get(tags::BITS_ALLOCATED, "Bits Allocated")?,
            get(tags::BITS_STORED, "Bits Stored")?,
        );
        if !matches!(shape.bits_allocated, 8 | 16) {
            return Err(unsupported(format!(
                "{} bits allocated",
                shape.bits_allocated
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
                if verify && codec::decode(target, shape, &encoded)? != native {
                    return Err(TranscodeError::NotLossless(number));
                }
                fragments.push(pad_even(encoded));
            }
            // One fragment per frame, so the empty Basic Offset Table is enough to find
            // each (PS3.5 A.4).
            object.put(DataElement::new(
                tags::PIXEL_DATA,
                VR::OB,
                PixelFragmentSequence::new(Vec::<u32>::new(), fragments),
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
    use dicom_dictionary_std::uids;

    use super::*;
    use crate::codec::{EXPLICIT_VR_LE, JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS};
    use crate::synthetic::SyntheticSeries;

    fn series() -> SyntheticSeries {
        let mut series = SyntheticSeries::new(7, 1);
        series.rows = 32;
        series.columns = 24;
        series
    }

    #[test]
    fn every_way_round_and_back() {
        let original = Instance::from_bytes(&series().multiframe_bytes(0, 3)).unwrap();
        let shape = original.shape().unwrap();
        let reference: Vec<Vec<u8>> = (1..=3)
            .map(|n| original.native_frame(n, shape).unwrap())
            .collect();
        for first in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS, EXPLICIT_VR_LE] {
            let once = Instance::from_bytes(&original.transcoded(first, true).unwrap()).unwrap();
            assert_eq!(once.key.transfer_syntax_uid, first);
            assert_eq!(once.key.sop_instance_uid, original.key.sop_instance_uid);
            for second in [JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS, EXPLICIT_VR_LE] {
                let twice = Instance::from_bytes(&once.transcoded(second, true).unwrap()).unwrap();
                assert_eq!(twice.key.transfer_syntax_uid, second);
                for (n, expected) in (1..=3).zip(&reference) {
                    assert_eq!(&twice.native_frame(n, shape).unwrap(), expected);
                }
            }
        }
    }

    #[test]
    fn frames_come_out_one_per_fragment() {
        let original = Instance::from_bytes(&series().multiframe_bytes(0, 2)).unwrap();
        let jls =
            Instance::from_bytes(&original.transcoded(JPEG_LS_LOSSLESS, true).unwrap()).unwrap();
        let frame = jls.frame(2).unwrap();
        assert_eq!(frame.media_type, "image/jls");
        assert_eq!(frame.transfer_syntax_uid, JPEG_LS_LOSSLESS);
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
        let bytes = series().encapsulated_bytes(
            0,
            uids::JPEG_BASELINE8_BIT,
            1,
            vec![],
            vec![vec![0xFF, 0xD8]],
        );
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
