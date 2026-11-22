//! Colour images in JPEG-LS Lossless, as monochrome ones are: RGB and palette colour, by
//! pixel and by plane (PS3.5 8.2.3: compressed colour is by pixel, Planar Configuration 0),
//! on real files: an ultrasound of the NEMA WG04 set, a Philips palette colour image.

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use pacs_core::Instance;
use pacs_core::real::file_bytes;

const JPEG_LS: &str = "1.2.840.10008.1.2.4.80";
const EXPLICIT: &str = "1.2.840.10008.1.2.1";

fn pixels(instance: &Instance) -> Vec<u8> {
    let shape = instance.shape().unwrap();
    instance.native_frame(1, shape).unwrap()
}

/// Through JPEG-LS and back: the same pixels, by pixel; Planar Configuration 0 both ways.
fn round_trip(file: &[u8]) -> (Instance, Instance) {
    let original = Instance::from_bytes(file).unwrap();
    let compressed = Instance::from_bytes(&original.transcoded(JPEG_LS, true).unwrap()).unwrap();
    assert_eq!(compressed.key.transfer_syntax_uid, JPEG_LS);
    let back = Instance::from_bytes(&compressed.transcoded(EXPLICIT, false).unwrap()).unwrap();
    for one in [&compressed, &back] {
        if one.int(tags::SAMPLES_PER_PIXEL) == Some(3) {
            assert_eq!(one.int(tags::PLANAR_CONFIGURATION), Some(0));
        }
    }
    (original, back)
}

#[test]
fn rgb_by_pixel_is_stored_losslessly() {
    let file = file_bytes("examples_rgb_color.dcm");
    let (original, back) = round_trip(&file);
    assert_eq!(original.shape().unwrap().samples, 3);
    assert_eq!(pixels(&back), pixels(&original));
}

/// The same ultrasound with its samples by plane (Planar Configuration 1): compressed by
/// pixel, it comes back as the by-pixel original.
#[test]
fn rgb_by_plane_is_interleaved_and_stored_losslessly() {
    let file = file_bytes("examples_rgb_color.dcm");
    let by_pixel = Instance::from_bytes(&file).unwrap();
    let interleaved = pixels(&by_pixel);
    // the planes: every red, then every green, then every blue
    let planes: Vec<u8> = (0..3)
        .flat_map(|s| {
            interleaved
                .iter()
                .skip(s)
                .step_by(3)
                .copied()
                .collect::<Vec<_>>()
        })
        .collect();
    let mut object = dicom_object::from_reader(&*file).unwrap();
    object.put(DataElement::new(
        tags::PLANAR_CONFIGURATION,
        VR::US,
        PrimitiveValue::from(1u16),
    ));
    object.put(DataElement::new(
        tags::PIXEL_DATA,
        VR::OB,
        PrimitiveValue::U8(planes.clone().into()),
    ));
    let mut planar = Vec::new();
    object.write_all(&mut planar).unwrap();
    let planar_instance = Instance::from_bytes(&planar).unwrap();
    assert!(planar_instance.shape().unwrap().planar);
    assert_eq!(pixels(&planar_instance), planes);
    let (_, back) = round_trip(&planar);
    assert_eq!(pixels(&back), interleaved);
}

#[test]
fn palette_colour_is_stored_losslessly_with_its_tables() {
    let file = file_bytes("examples_palette.dcm");
    let (original, back) = round_trip(&file);
    assert_eq!(pixels(&back), pixels(&original));
    // the lookup tables go along untouched
    for tag in [
        tags::RED_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::GREEN_PALETTE_COLOR_LOOKUP_TABLE_DATA,
        tags::BLUE_PALETTE_COLOR_LOOKUP_TABLE_DATA,
    ] {
        let data = |instance: &Instance| {
            instance
                .object
                .element(tag)
                .unwrap()
                .to_bytes()
                .unwrap()
                .to_vec()
        };
        assert_eq!(data(&back), data(&original), "{tag}");
    }
}

/// What is not one native frame of full planes stays as it is: YBR_FULL_422.
#[test]
fn subsampled_colour_is_left_as_it_is() {
    let file = file_bytes("examples_rgb_color.dcm");
    let mut object = dicom_object::from_reader(&*file).unwrap();
    object.put(DataElement::new(
        tags::PHOTOMETRIC_INTERPRETATION,
        VR::CS,
        PrimitiveValue::from("YBR_FULL_422"),
    ));
    let mut bytes = Vec::new();
    object.write_all(&mut bytes).unwrap();
    let instance = Instance::from_bytes(&bytes).unwrap();
    let error = instance.transcoded(JPEG_LS, true).unwrap_err().to_string();
    assert!(error.contains("YBR_FULL_422"), "{error}");
}
