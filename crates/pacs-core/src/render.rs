//! A frame made a picture for eyes that do not read DICOM (PS3.18 8.3.5.1, rendered
//! resources): its stored values through the Modality LUT and a VOI window to 8-bit
//! grayscale (PS3.3 C.11.1, C.11.2), or its colour made RGB; cropped and scaled to a
//! viewport; written as JPEG or PNG.

use dicom_dictionary_std::tags;

use crate::Instance;
use crate::codec::{self, Shape};
use crate::transcode::TranscodeError;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Not something rendered here, or a parameter that cannot be met.
    #[error("cannot render: {0}")]
    Unsupported(String),
    #[error(transparent)]
    Pixels(#[from] TranscodeError),
    #[error("writing the picture: {0}")]
    Encode(String),
}

fn unsupported(what: impl Into<String>) -> RenderError {
    RenderError::Unsupported(what.into())
}

/// The VOI LUT Function (0028,1056) a window is applied by (PS3.3 C.11.2.1.2-3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Function {
    Linear,
    LinearExact,
    Sigmoid,
}

impl Function {
    fn parse(name: &str) -> Option<Function> {
        match name.trim().to_ascii_lowercase().as_str() {
            "linear" => Some(Function::Linear),
            "linear_exact" | "linear-exact" => Some(Function::LinearExact),
            "sigmoid" => Some(Function::Sigmoid),
            _ => None,
        }
    }
}

/// A VOI window: the values from `center - width/2` to `center + width/2` spread over
/// black to white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub center: f64,
    pub width: f64,
    pub function: Function,
}

impl Window {
    /// The `window` query parameter: `center,width,function` (PS3.18 8.3.5.1.4).
    pub fn parse(text: &str) -> Result<Window, String> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        let [center, width, function] = parts[..] else {
            return Err(format!("window is center,width,function, not {text:?}"));
        };
        let number = |s: &str| {
            s.parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("window: {s:?} is not a number"))
        };
        let window = Window {
            center: number(center)?,
            width: number(width)?,
            function: Function::parse(function).ok_or_else(|| {
                format!("window: {function:?} is not linear, linear-exact or sigmoid")
            })?,
        };
        window.check()?;
        Ok(window)
    }

    fn check(&self) -> Result<(), String> {
        let least = if self.function == Function::Linear {
            1.0
        } else {
            f64::MIN_POSITIVE
        };
        if self.width < least {
            return Err(format!(
                "window width {} is too small for its function",
                self.width
            ));
        }
        Ok(())
    }

    /// An output value of 0 to 255 for value `x` (PS3.3 C.11.2.1.2.1, C.11.2.1.3.1-2).
    fn apply(&self, x: f64) -> f64 {
        let (c, w) = (self.center, self.width);
        let unit = match self.function {
            Function::Linear => {
                if x <= c - 0.5 - (w - 1.0) / 2.0 {
                    0.0
                } else if x > c - 0.5 + (w - 1.0) / 2.0 {
                    1.0
                } else {
                    (x - (c - 0.5)) / (w - 1.0) + 0.5
                }
            }
            Function::LinearExact => {
                if x <= c - w / 2.0 {
                    0.0
                } else if x > c + w / 2.0 {
                    1.0
                } else {
                    (x - c) / w + 0.5
                }
            }
            Function::Sigmoid => 1.0 / (1.0 + (-4.0 * (x - c) / w).exp()),
        };
        unit.clamp(0.0, 1.0) * 255.0
    }
}

/// The `viewport` query parameter: the picture fits `width` by `height`, its proportions
/// kept; with a source region, only that part of the frame (PS3.18 8.3.5.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    /// x, y, width and height of the region of the frame, in its pixels.
    pub region: Option<(u32, u32, u32, u32)>,
}

impl Viewport {
    /// `vw,vh` or `vw,vh,sx,sy,sw,sh`.
    pub fn parse(text: &str) -> Result<Viewport, String> {
        let numbers: Vec<u32> = text
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<u32>()
                    .map_err(|_| format!("viewport: {s:?} is not a whole number"))
            })
            .collect::<Result<_, _>>()?;
        let viewport = match numbers[..] {
            [width, height] => Viewport {
                width,
                height,
                region: None,
            },
            [width, height, x, y, w, h] => Viewport {
                width,
                height,
                region: Some((x, y, w, h)),
            },
            _ => {
                return Err(format!(
                    "viewport is vw,vh or vw,vh,sx,sy,sw,sh, not {text:?}"
                ));
            }
        };
        if viewport.width == 0
            || viewport.height == 0
            || viewport.width > MAX_SIDE
            || viewport.height > MAX_SIDE
        {
            return Err(format!(
                "viewport: a side is from 1 to {MAX_SIDE} pixels, not {text:?}"
            ));
        }
        if let Some((_, _, w, h)) = viewport.region
            && (w == 0 || h == 0)
        {
            return Err("viewport: the region is empty".into());
        }
        Ok(viewport)
    }
}

/// The largest side a viewport may ask for.
pub const MAX_SIDE: u32 = 4096;

/// How to render: all optional.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rendering {
    pub window: Option<Window>,
    pub viewport: Option<Viewport>,
    /// JPEG quality, 1 to 100.
    pub quality: Option<u8>,
}

/// A picture of 8-bit samples, grayscale (1 a pixel) or RGB (3), row after row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub pixels: Vec<u8>,
}

/// Frame `number` (from 1) of `instance` as a picture.
pub fn render(
    instance: &Instance,
    number: u32,
    rendering: &Rendering,
) -> Result<Picture, RenderError> {
    let shape = instance.shape()?;
    let native = instance.native_frame(number, shape)?;
    let by_pixel = codec::by_pixel(shape, &native);
    let photometric = instance
        .string(tags::PHOTOMETRIC_INTERPRETATION)
        .unwrap_or_default();
    let picture = match photometric.as_str() {
        "MONOCHROME1" | "MONOCHROME2" => grayscale(
            instance,
            shape,
            &by_pixel,
            rendering,
            photometric == "MONOCHROME1",
        )?,
        "RGB" | "YBR_FULL" => colour(shape, &by_pixel, photometric == "YBR_FULL"),
        "PALETTE COLOR" => palette(instance, shape, &by_pixel)?,
        other => return Err(unsupported(format!("photometric interpretation {other:?}"))),
    };
    match &rendering.viewport {
        Some(viewport) => fit(&picture, viewport),
        None => Ok(picture),
    }
}

/// The stored values of a monochrome frame, as numbers: Bits Stored of them, signed when
/// Pixel Representation is 1.
fn stored_values(instance: &Instance, shape: Shape, native: &[u8]) -> Vec<f64> {
    let signed = instance.int(tags::PIXEL_REPRESENTATION) == Some(1);
    let bits = u32::from(shape.bits_stored);
    let mask: u32 = if bits >= 32 {
        u32::MAX
    } else {
        (1 << bits) - 1
    };
    let value = |raw: u32| {
        let raw = raw & mask;
        if signed && bits > 0 && raw & (1 << (bits - 1)) != 0 {
            (i64::from(raw) - (1i64 << bits)) as f64
        } else {
            f64::from(raw)
        }
    };
    match shape.bits_allocated {
        8 => native.iter().map(|&b| value(u32::from(b))).collect(),
        16 => native
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| value(u32::from(u16::from_le_bytes(*c))))
            .collect(),
        _ => native
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| value(u32::from_le_bytes(*c)))
            .collect(),
    }
}

/// The first value of a decimal string attribute.
fn first_number(instance: &Instance, tag: dicom_core::Tag) -> Option<f64> {
    instance
        .object
        .get(tag)?
        .to_str()
        .ok()?
        .split('\\')
        .next()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
}

fn grayscale(
    instance: &Instance,
    shape: Shape,
    native: &[u8],
    rendering: &Rendering,
    inverted: bool,
) -> Result<Picture, RenderError> {
    // Modality LUT by rescale (C.11.1); a Modality LUT Sequence is not applied here
    let slope = first_number(instance, tags::RESCALE_SLOPE).unwrap_or(1.0);
    let intercept = first_number(instance, tags::RESCALE_INTERCEPT).unwrap_or(0.0);
    let values: Vec<f64> = stored_values(instance, shape, native)
        .into_iter()
        .map(|v| v * slope + intercept)
        .collect();
    // the window asked for, else the file's first, else the frame's whole range
    let file_window = || {
        let window = Window {
            center: first_number(instance, tags::WINDOW_CENTER)?,
            width: first_number(instance, tags::WINDOW_WIDTH)?,
            function: instance
                .string(tags::VOILUT_FUNCTION)
                .and_then(|f| Function::parse(&f))
                .unwrap_or(Function::Linear),
        };
        window.check().ok().map(|()| window)
    };
    let window = match rendering.window.or_else(file_window) {
        Some(window) => window,
        None => {
            let (low, high) = values
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), &v| (l.min(v), h.max(v)));
            Window {
                center: (low + high) / 2.0,
                width: (high - low).max(1.0),
                function: Function::LinearExact,
            }
        }
    };
    let pixels = values
        .iter()
        .map(|&v| {
            let y = window.apply(v).round() as u8;
            if inverted { 255 - y } else { y }
        })
        .collect();
    Ok(Picture {
        width: u32::from(shape.columns),
        height: u32::from(shape.rows),
        channels: 1,
        pixels,
    })
}

/// RGB, or YBR_FULL made RGB (PS3.3 C.7.6.3.1.2), its samples brought to 8 bits.
fn colour(shape: Shape, native: &[u8], ybr: bool) -> Picture {
    let eight: Vec<u8> = match shape.bits_allocated {
        8 => native.to_vec(),
        _ => {
            let shift = shape.bits_stored.saturating_sub(8);
            native
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| (u16::from_le_bytes(*c) >> shift).min(255) as u8)
                .collect()
        }
    };
    let pixels = if ybr {
        eight
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[y, cb, cr]| {
                let (y, cb, cr) = (f64::from(y), f64::from(cb) - 128.0, f64::from(cr) - 128.0);
                let byte = |v: f64| v.round().clamp(0.0, 255.0) as u8;
                [
                    byte(y + 1.402 * cr),
                    byte(y - 0.344_136 * cb - 0.714_136 * cr),
                    byte(y + 1.772 * cb),
                ]
            })
            .collect()
    } else {
        eight
    };
    Picture {
        width: u32::from(shape.columns),
        height: u32::from(shape.rows),
        channels: 3,
        pixels,
    }
}

/// One of the Palette Color Lookup Tables (PS3.3 C.7.6.3.1.5-6): its first value mapped,
/// and its entries brought to 8 bits.
fn lookup_table(
    instance: &Instance,
    descriptor: dicom_core::Tag,
    data: dicom_core::Tag,
) -> Result<(i64, Vec<u8>), RenderError> {
    let descriptor: Vec<i64> = instance
        .object
        .get(descriptor)
        .and_then(|e| e.to_multi_int::<i64>().ok())
        .filter(|d| d.len() == 3)
        .ok_or_else(|| unsupported("a palette colour lookup table without its descriptor"))?;
    let entries = if descriptor[0] == 0 {
        65536
    } else {
        descriptor[0] as usize
    };
    let (first, bits) = (descriptor[1], descriptor[2]);
    let bytes = instance
        .object
        .get(data)
        .and_then(|e| e.to_bytes().ok())
        .ok_or_else(|| unsupported("a segmented or missing palette colour lookup table"))?;
    let table: Vec<u8> = if bits == 8 && bytes.len() == entries {
        bytes.to_vec()
    } else {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                let word = u16::from_le_bytes(*c);
                if bits == 8 {
                    (word & 0xFF) as u8
                } else {
                    (word >> 8) as u8
                }
            })
            .collect()
    };
    if table.len() < entries {
        return Err(unsupported(
            "a palette colour lookup table shorter than its descriptor",
        ));
    }
    Ok((first, table[..entries].to_vec()))
}

fn palette(instance: &Instance, shape: Shape, native: &[u8]) -> Result<Picture, RenderError> {
    let tables = [
        lookup_table(
            instance,
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        )?,
        lookup_table(
            instance,
            tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        )?,
        lookup_table(
            instance,
            tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DESCRIPTOR,
            tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        )?,
    ];
    let values: Vec<i64> = match shape.bits_allocated {
        8 => native.iter().map(|&b| i64::from(b)).collect(),
        _ => native
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i64::from(u16::from_le_bytes(*c)))
            .collect(),
    };
    let pixels = values
        .iter()
        .flat_map(|&v| {
            tables.each_ref().map(|(first, table)| {
                // values before the first mapped take its first entry, past the last its last
                let at = (v - first).clamp(0, table.len() as i64 - 1) as usize;
                table[at]
            })
        })
        .collect();
    Ok(Picture {
        width: u32::from(shape.columns),
        height: u32::from(shape.rows),
        channels: 3,
        pixels,
    })
}

/// `picture` cropped to the viewport's region and scaled to fit it, proportions kept,
/// each pixel interpolated between its four neighbours.
fn fit(picture: &Picture, viewport: &Viewport) -> Result<Picture, RenderError> {
    let (x0, y0, w, h) = viewport
        .region
        .unwrap_or((0, 0, picture.width, picture.height));
    if x0.checked_add(w).is_none_or(|end| end > picture.width)
        || y0.checked_add(h).is_none_or(|end| end > picture.height)
    {
        return Err(unsupported(format!(
            "the viewport's region goes past the frame of {} x {}",
            picture.width, picture.height
        )));
    }
    let scale =
        (f64::from(viewport.width) / f64::from(w)).min(f64::from(viewport.height) / f64::from(h));
    let out_w = ((f64::from(w) * scale).round() as u32).max(1);
    let out_h = ((f64::from(h) * scale).round() as u32).max(1);
    let channels = usize::from(picture.channels);
    let at = |x: u32, y: u32, c: usize| {
        f64::from(picture.pixels[(y as usize * picture.width as usize + x as usize) * channels + c])
    };
    let mut pixels = Vec::with_capacity(out_w as usize * out_h as usize * channels);
    for oy in 0..out_h {
        // the centre of the output pixel, in the region's pixels
        let sy = ((f64::from(oy) + 0.5) / scale - 0.5).clamp(0.0, f64::from(h - 1));
        let (ya, yb) = (sy.floor() as u32, (sy.floor() as u32 + 1).min(h - 1));
        let fy = sy - sy.floor();
        for ox in 0..out_w {
            let sx = ((f64::from(ox) + 0.5) / scale - 0.5).clamp(0.0, f64::from(w - 1));
            let (xa, xb) = (sx.floor() as u32, (sx.floor() as u32 + 1).min(w - 1));
            let fx = sx - sx.floor();
            for c in 0..channels {
                let top = at(x0 + xa, y0 + ya, c) * (1.0 - fx) + at(x0 + xb, y0 + ya, c) * fx;
                let bottom = at(x0 + xa, y0 + yb, c) * (1.0 - fx) + at(x0 + xb, y0 + yb, c) * fx;
                pixels.push((top * (1.0 - fy) + bottom * fy).round().clamp(0.0, 255.0) as u8);
            }
        }
    }
    Ok(Picture {
        width: out_w,
        height: out_h,
        channels: picture.channels,
        pixels,
    })
}

/// The picture as a JPEG file, of `quality` (1 to 100).
pub fn jpeg(picture: &Picture, quality: u8) -> Result<Vec<u8>, RenderError> {
    let side = |n: u32| {
        u16::try_from(n)
            .map_err(|_| RenderError::Encode(format!("{n} pixels is too wide for JPEG")))
    };
    let color = if picture.channels == 1 {
        jpeg_encoder::ColorType::Luma
    } else {
        jpeg_encoder::ColorType::Rgb
    };
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, quality.clamp(1, 100))
        .encode(
            &picture.pixels,
            side(picture.width)?,
            side(picture.height)?,
            color,
        )
        .map_err(|e| RenderError::Encode(e.to_string()))?;
    Ok(out)
}

/// The picture as a PNG file.
pub fn png(picture: &Picture) -> Result<Vec<u8>, RenderError> {
    let error = |e: png::EncodingError| RenderError::Encode(e.to_string());
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, picture.width, picture.height);
        encoder.set_color(if picture.channels == 1 {
            png::ColorType::Grayscale
        } else {
            png::ColorType::Rgb
        });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(error)?;
        writer.write_image_data(&picture.pixels).map_err(error)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_linear_window_spreads_its_range_over_black_to_white() {
        let window = Window::parse("40,400,linear").unwrap();
        assert_eq!(window.apply(-1000.0), 0.0);
        assert_eq!(window.apply(1000.0), 255.0);
        assert!((window.apply(40.0) - 127.8).abs() < 0.5);
        let exact = Window::parse("0,100,linear-exact").unwrap();
        assert_eq!(exact.apply(-50.0), 0.0);
        assert!((exact.apply(0.0) - 127.5).abs() < 1e-9);
        let sigmoid = Window::parse("0,100,sigmoid").unwrap();
        assert!((sigmoid.apply(0.0) - 127.5).abs() < 1e-9);
    }

    #[test]
    fn parameters_that_cannot_be_met_say_why() {
        for (bad, said) in [
            ("40,400", "center,width,function"),
            ("a,400,linear", "is not a number"),
            ("40,400,cubic", "is not linear"),
            ("40,0.5,linear", "too small"),
        ] {
            let error = Window::parse(bad).unwrap_err();
            assert!(error.contains(said), "{bad}: {error}");
        }
        for (bad, said) in [
            ("512", "vw,vh"),
            ("0,512", "from 1 to"),
            ("512,x", "not a whole number"),
            ("512,512,0,0,0,10", "region is empty"),
            ("9000,10", "from 1 to"),
        ] {
            let error = Viewport::parse(bad).unwrap_err();
            assert!(error.contains(said), "{bad}: {error}");
        }
    }

    #[test]
    fn a_viewport_keeps_the_proportions_and_crops_its_region() {
        let picture = Picture {
            width: 4,
            height: 2,
            channels: 1,
            pixels: vec![0, 0, 255, 255, 0, 0, 255, 255],
        };
        let half = fit(&picture, &Viewport::parse("2,100").unwrap()).unwrap();
        assert_eq!((half.width, half.height), (2, 1));
        assert_eq!(half.pixels, [0, 255]);
        let right = fit(&picture, &Viewport::parse("4,4,2,0,2,2").unwrap()).unwrap();
        assert_eq!((right.width, right.height), (4, 4));
        assert!(right.pixels.iter().all(|&p| p == 255));
        assert!(fit(&picture, &Viewport::parse("4,4,3,0,2,2").unwrap()).is_err());
    }
}
