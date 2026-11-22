//! Lossless pixel codecs: what pacs-rs compresses on the way in and transcodes on the way
//! out. Grayscale only, 8 or 16 bits allocated, one sample per pixel - what CT, MR and
//! most radiography are.
//!
//! - JPEG-LS lossless (`1.2.840.10008.1.2.4.80`), codecsls: how pacs-rs stores native pixel
//!   data. On 172 slices of a public CT it took 86 MB to 25 MB at about 2.5 ms a slice, and
//!   decodes about as fast; its streams are byte for byte those of CharLS, the reference.
//! - JPEG XL (`1.2.840.10008.1.2.4.110`, and `.112` read only): decoded with jxl-rs, the
//!   libjxl project's Rust decoder; encoded with zune-jpegxl, the one pure-Rust lossless
//!   encoder. zune compresses CT about 17% less than JPEG-LS, so JPEG XL is something
//!   pacs-rs accepts, keeps and serves on request, never what it chooses to store.
//!
//! Signed pixels are coded as their raw bit pattern, as DCMTK, GDCM and CharLS's own
//! DICOM users do: a decoder reinterprets them by Pixel Representation.

use std::borrow::Cow;

use jxl::api::{
    Endianness, Event, JxlColorType, JxlDataFormat, JxlDecoder, JxlDecoderOptions, JxlOutputBuffer,
    JxlPixelFormat,
};
use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace;
use zune_core::options::EncoderOptions;
use zune_jpegxl::JxlSimpleEncoder;

pub const EXPLICIT_VR_LE: &str = "1.2.840.10008.1.2.1";
pub const IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";
pub const JPEG_LS_LOSSLESS: &str = "1.2.840.10008.1.2.4.80";
pub const JPEG_LS_NEAR_LOSSLESS: &str = "1.2.840.10008.1.2.4.81";
pub const JPEG_XL_LOSSLESS: &str = "1.2.840.10008.1.2.4.110";
pub const JPEG_XL: &str = "1.2.840.10008.1.2.4.112";
/// Deflated Image Frame Compression (PS3.5 10.20): each frame deflated (RFC 1951) in a
/// fragment of its own, lossless whatever the pixels are; what is stored of more than 16
/// bits, which JPEG-LS does not code.
pub const DEFLATED_FRAMES: &str = "1.2.840.10008.1.2.8.1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("{0}")]
    Unsupported(String),
    #[error("JPEG-LS: {0}")]
    JpegLs(String),
    #[error("JPEG XL: {0}")]
    JpegXl(String),
}

/// The layout of one frame of pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    pub rows: u16,
    pub columns: u16,
    pub bits_allocated: u16,
    pub bits_stored: u16,
    /// Samples per pixel: 1, or 3 for colour.
    pub samples: u16,
    /// Of a native colour frame, whether its samples are by plane (Planar Configuration 1:
    /// all the reds, then the greens, then the blues) rather than by pixel (0: RGBRGB...).
    /// A compressed frame decodes by pixel, whatever its data set says (PS3.5 8.2.3).
    pub planar: bool,
}

impl Shape {
    /// A monochrome frame.
    pub fn new(rows: u16, columns: u16, bits_allocated: u16, bits_stored: u16) -> Self {
        Shape {
            rows,
            columns,
            bits_allocated,
            bits_stored,
            samples: 1,
            planar: false,
        }
    }

    /// The same frame, of `samples` per pixel, by plane when `planar`.
    pub fn with_samples(mut self, samples: u16, planar: bool) -> Self {
        self.samples = samples;
        self.planar = planar && samples > 1;
        self
    }

    /// Bytes of one native frame.
    pub fn frame_len(&self) -> usize {
        usize::from(self.rows)
            * usize::from(self.columns)
            * usize::from(self.samples)
            * usize::from(self.bits_allocated / 8)
    }

    fn check(&self) -> Result<(), CodecError> {
        if !matches!(self.samples, 1 | 3) {
            return Err(CodecError::Unsupported(format!(
                "{} samples per pixel",
                self.samples
            )));
        }
        if !matches!(self.bits_allocated, 8 | 16 | 32) {
            return Err(CodecError::Unsupported(format!(
                "{} bits allocated",
                self.bits_allocated
            )));
        }
        if self.bits_stored == 0 || self.bits_stored > self.bits_allocated {
            return Err(CodecError::Unsupported(format!(
                "{} bits stored of {} allocated",
                self.bits_stored, self.bits_allocated
            )));
        }
        if self.rows == 0 || self.columns == 0 {
            return Err(CodecError::Unsupported("an empty frame".into()));
        }
        Ok(())
    }
}

/// Whether frames in transfer syntax `ts` can be decoded here.
pub fn can_decode(ts: &str) -> bool {
    matches!(
        ts,
        EXPLICIT_VR_LE
            | IMPLICIT_VR_LE
            | JPEG_LS_LOSSLESS
            | JPEG_LS_NEAR_LOSSLESS
            | JPEG_XL_LOSSLESS
            | JPEG_XL
            | DEFLATED_FRAMES
    )
}

/// Whether frames can be encoded in transfer syntax `ts` here.
pub fn can_encode(ts: &str) -> bool {
    matches!(
        ts,
        EXPLICIT_VR_LE | JPEG_LS_LOSSLESS | JPEG_XL_LOSSLESS | DEFLATED_FRAMES
    )
}

/// Whether `ts` keeps pixel data as plain bytes rather than in fragments.
pub fn is_native(ts: &str) -> bool {
    matches!(ts, EXPLICIT_VR_LE | IMPLICIT_VR_LE)
}

/// One frame in `ts`, decoded to native little-endian pixels.
pub fn decode(ts: &str, shape: Shape, bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    shape.check()?;
    let native = match ts {
        EXPLICIT_VR_LE | IMPLICIT_VR_LE => bytes.to_vec(),
        JPEG_LS_LOSSLESS | JPEG_LS_NEAR_LOSSLESS => decode_jpeg_ls(bytes)?,
        JPEG_XL_LOSSLESS | JPEG_XL => decode_jpeg_xl(shape, bytes)?,
        DEFLATED_FRAMES => inflate(bytes)?,
        other => {
            return Err(CodecError::Unsupported(format!(
                "no decoder for transfer syntax {other}"
            )));
        }
    };
    if native.len() != shape.frame_len() {
        return Err(CodecError::Unsupported(format!(
            "decoded {} bytes, a {}x{} frame of {} bits is {}",
            native.len(),
            shape.rows,
            shape.columns,
            shape.bits_allocated,
            shape.frame_len()
        )));
    }
    Ok(native)
}

/// One native little-endian frame, encoded in `ts`.
pub fn encode(ts: &str, shape: Shape, native: &[u8]) -> Result<Vec<u8>, CodecError> {
    shape.check()?;
    if native.len() != shape.frame_len() {
        return Err(CodecError::Unsupported(format!(
            "{} bytes for a frame of {}",
            native.len(),
            shape.frame_len()
        )));
    }
    match ts {
        EXPLICIT_VR_LE => Ok(native.to_vec()),
        JPEG_LS_LOSSLESS => encode_jpeg_ls(shape, native),
        JPEG_XL_LOSSLESS => encode_jpeg_xl(shape, native),
        DEFLATED_FRAMES => deflate(native),
        other => Err(CodecError::Unsupported(format!(
            "no encoder for transfer syntax {other}"
        ))),
    }
}

/// The bits per sample to code: Bits Stored when every value fits in it, else all the
/// bits allocated (signed values sign-extended past Bits Stored, or stray high bits).
fn coded_bits(shape: Shape, native: &[u8]) -> u16 {
    if shape.bits_stored == shape.bits_allocated {
        return shape.bits_allocated;
    }
    let fits = match shape.bits_allocated {
        16 => {
            let mask = !((1u16 << shape.bits_stored) - 1);
            native
                .as_chunks::<2>()
                .0
                .iter()
                .all(|c| u16::from_le_bytes(*c) & mask == 0)
        }
        _ => {
            let mask = !((1u8 << shape.bits_stored) - 1);
            native.iter().all(|&b| b & mask == 0)
        }
    };
    if fits {
        shape.bits_stored
    } else {
        shape.bits_allocated
    }
}

/// A frame deflated as PS3.5 10.20 says: RFC 1951, no zlib or gzip wrapping.
fn deflate(native: &[u8]) -> Result<Vec<u8>, CodecError> {
    use std::io::Write;
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(native)
        .and_then(|()| encoder.finish())
        .map_err(|e| CodecError::Unsupported(format!("deflating a frame: {e}")))
}

fn inflate(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    use std::io::Read;
    let mut native = Vec::new();
    flate2::read::DeflateDecoder::new(bytes)
        .read_to_end(&mut native)
        .map_err(|e| CodecError::Unsupported(format!("inflating a frame: {e}")))?;
    Ok(native)
}

fn encode_jpeg_ls(shape: Shape, native: &[u8]) -> Result<Vec<u8>, CodecError> {
    if shape.bits_allocated > 16 {
        return Err(CodecError::Unsupported(format!(
            "{} bits allocated: JPEG-LS codes 16 at most",
            shape.bits_allocated
        )));
    }
    let bits = u8::try_from(coded_bits(shape, native))
        .map_err(|_| CodecError::JpegLs("more than 16 bits per sample".into()))?;
    let frame = codecsls::jpegls::FrameInfo {
        width: u32::from(shape.columns),
        height: u32::from(shape.rows),
        bits_per_sample: bits,
        components: u8::try_from(shape.samples)
            .map_err(|_| CodecError::JpegLs("too many samples per pixel".into()))?,
    };
    // codecsls takes the samples of a pixel together, as JPEG-LS decodes them
    let by_pixel = by_pixel(shape, native);
    codecsls::jpegls::encode(frame, &by_pixel).map_err(|e| CodecError::JpegLs(e.to_string()))
}

/// A native frame with its samples by pixel (RGBRGB...): a planar one interleaved, any other
/// as it is.
pub fn by_pixel(shape: Shape, native: &[u8]) -> Cow<'_, [u8]> {
    if !shape.planar {
        return Cow::Borrowed(native);
    }
    let sample = usize::from(shape.bits_allocated / 8);
    let samples = usize::from(shape.samples);
    let plane = native.len() / samples;
    let pixels = plane / sample;
    let mut out = Vec::with_capacity(native.len());
    for pixel in 0..pixels {
        for s in 0..samples {
            let at = s * plane + pixel * sample;
            out.extend_from_slice(&native[at..at + sample]);
        }
    }
    Cow::Owned(out)
}

fn decode_jpeg_ls(bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    codecsls::jpegls::decode(bytes)
        .map(|(_, pixels)| pixels)
        .map_err(|e| CodecError::JpegLs(e.to_string()))
}

fn encode_jpeg_xl(shape: Shape, native: &[u8]) -> Result<Vec<u8>, CodecError> {
    if shape.samples != 1 || shape.bits_allocated > 16 {
        return Err(CodecError::Unsupported(
            "JPEG XL here is for monochrome frames of 16 bits at most".into(),
        ));
    }
    // zune takes 16-bit samples in native byte order.
    let data: Cow<[u8]> = if shape.bits_allocated == 16 && cfg!(target_endian = "big") {
        Cow::Owned(
            native
                .as_chunks::<2>()
                .0
                .iter()
                .flat_map(|c| [c[1], c[0]])
                .collect(),
        )
    } else {
        Cow::Borrowed(native)
    };
    let depth = if shape.bits_allocated == 16 {
        BitDepth::Sixteen
    } else {
        BitDepth::Eight
    };
    let options = EncoderOptions::new(
        usize::from(shape.columns),
        usize::from(shape.rows),
        ColorSpace::Luma,
        depth,
    );
    let mut out = Vec::new();
    JxlSimpleEncoder::new(&data, options)
        .encode(&mut out)
        .map_err(|e| CodecError::JpegXl(format!("{e:?}")))?;
    Ok(out)
}

fn decode_jpeg_xl(shape: Shape, bytes: &[u8]) -> Result<Vec<u8>, CodecError> {
    let error = |e: jxl::error::Error| CodecError::JpegXl(e.to_string());
    let mut decoder = JxlDecoder::new(JxlDecoderOptions::default());
    let mut input = bytes;
    loop {
        match decoder.process(&mut input, None, None).map_err(error)? {
            Event::BasicInfo => break,
            Event::NeedMoreInput { .. } if input.is_empty() => {
                return Err(CodecError::JpegXl("the codestream ends early".into()));
            }
            _ => {}
        }
    }
    let info = decoder.basic_info().expect("after BasicInfo");
    let (width, height) = info.size;
    if (width, height) != (usize::from(shape.columns), usize::from(shape.rows)) {
        return Err(CodecError::JpegXl(format!(
            "the codestream is {width}x{height}, the data set says {}x{}",
            shape.columns, shape.rows
        )));
    }
    let bits = info.bit_depth.bits_per_sample();
    let extra_channels = info.extra_channels.len();
    let (format, bytes_per_sample) = if bits <= 8 && shape.bits_allocated == 8 {
        (JxlDataFormat::U8 { bit_depth: 8 }, 1)
    } else if bits <= 16 && shape.bits_allocated == 16 {
        (
            JxlDataFormat::U16 {
                endianness: Endianness::LittleEndian,
                bit_depth: bits as u8,
            },
            2,
        )
    } else {
        return Err(CodecError::JpegXl(format!(
            "{bits} bits per sample in the codestream, {} allocated",
            shape.bits_allocated
        )));
    };
    decoder
        .set_pixel_format(JxlPixelFormat {
            color_type: JxlColorType::Grayscale,
            color_data_format: Some(format),
            extra_channel_format: vec![None; extra_channels],
        })
        .map_err(error)?;
    let row_bytes = width * bytes_per_sample;
    let mut pixels = vec![0u8; row_bytes * height];
    loop {
        let mut buffers = [JxlOutputBuffer::new(&mut pixels, height, row_bytes)];
        match decoder
            .process(&mut input, Some(&mut buffers), None)
            .map_err(error)?
        {
            Event::FrameComplete { .. } | Event::Complete => return Ok(pixels),
            Event::NeedMoreInput { .. } if input.is_empty() => {
                return Err(CodecError::JpegXl("the codestream ends early".into()));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A CT-like 16-bit frame: air, a body with structure, a few bright points.
    fn ct_frame(shape: Shape, signed: bool) -> Vec<u8> {
        let (r, c) = (usize::from(shape.rows), usize::from(shape.columns));
        let mut out = Vec::with_capacity(r * c * 2);
        for y in 0..r {
            for x in 0..c {
                let (dx, dy) = (x as f64 - c as f64 / 2.0, y as f64 - r as f64 / 2.0);
                let inside = dx * dx + dy * dy < (r as f64 * 0.4).powi(2);
                let hu: i32 = if inside {
                    40 + ((x * 7 + y * 13) % 50) as i32 - 25
                        + if (x * y) % 97 == 0 { 900 } else { 0 }
                } else {
                    -1000 - ((x + y) % 3) as i32
                };
                let value = if signed {
                    hu as i16 as u16
                } else {
                    (hu + 1024) as u16
                };
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        out
    }

    fn round_trip(ts: &str, shape: Shape, native: &[u8]) -> usize {
        let encoded = encode(ts, shape, native).unwrap();
        assert_eq!(decode(ts, shape, &encoded).unwrap(), native, "{ts}");
        encoded.len()
    }

    #[test]
    fn lossless_round_trips() {
        let shape = Shape::new(64, 80, 16, 12);
        for signed in [false, true] {
            let frame = ct_frame(shape, signed);
            for ts in [EXPLICIT_VR_LE, JPEG_LS_LOSSLESS, JPEG_XL_LOSSLESS] {
                let len = round_trip(ts, shape, &frame);
                if ts != EXPLICIT_VR_LE {
                    assert!(len < frame.len() / 2, "{ts} signed={signed}: {len}");
                }
            }
        }
    }

    #[test]
    fn eight_bit_and_full_sixteen() {
        let shape = Shape::new(17, 23, 8, 8);
        let frame: Vec<u8> = (0..17 * 23).map(|i| (i * 31 % 251) as u8).collect();
        round_trip(JPEG_LS_LOSSLESS, shape, &frame);
        round_trip(JPEG_XL_LOSSLESS, shape, &frame);

        let shape = Shape::new(9, 11, 16, 16);
        let frame: Vec<u8> = (0..99u32)
            .flat_map(|i| ((i.wrapping_mul(2654435761) >> 16) as u16).to_le_bytes())
            .collect();
        round_trip(JPEG_LS_LOSSLESS, shape, &frame);
        round_trip(JPEG_XL_LOSSLESS, shape, &frame);
    }

    #[test]
    fn refuses_what_it_does_not_handle() {
        let shape = Shape::new(4, 4, 32, 32);
        assert!(matches!(
            encode(JPEG_LS_LOSSLESS, shape, &[0; 64]),
            Err(CodecError::Unsupported(_))
        ));
        let shape = Shape::new(4, 4, 16, 12);
        assert!(matches!(
            encode(JPEG_LS_LOSSLESS, shape, &[0; 31]),
            Err(CodecError::Unsupported(_))
        ));
        assert!(matches!(
            decode("1.2.840.10008.1.2.4.50", shape, &[]),
            Err(CodecError::Unsupported(_))
        ));
        assert!(decode(JPEG_XL_LOSSLESS, shape, b"not a codestream").is_err());
    }
}
