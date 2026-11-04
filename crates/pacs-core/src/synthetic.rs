//! Synthetic CT series for tests: small, deterministic, no patient data.

use dicom_core::value::PixelFragmentSequence;
use dicom_core::{DataElement, PrimitiveValue, VR, dicom_value};
use dicom_dictionary_std::{tags, uids};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

/// A synthetic CT series: `slices` instances of `rows` x `columns` 16-bit pixels.
#[derive(Debug, Clone)]
pub struct SyntheticSeries {
    /// Distinguishes series made in the same test; goes into the series and instance UIDs.
    pub seed: u32,
    /// Goes into the study UID and patient ID; series with the same one share a study.
    pub study_seed: u32,
    /// Goes into the patient ID instead of `study_seed`, so studies can share a patient.
    pub patient_seed: Option<u32>,
    pub series_number: u32,
    pub slices: u32,
    pub rows: u16,
    pub columns: u16,
    pub patient_name: String,
    /// Specific Character Set, when the patient name needs one.
    pub character_set: Option<&'static str>,
}

impl SyntheticSeries {
    pub fn new(seed: u32, slices: u32) -> Self {
        SyntheticSeries {
            seed,
            study_seed: seed,
            patient_seed: None,
            series_number: 1,
            slices,
            rows: 4,
            columns: 4,
            patient_name: "SYNTHETIC^SERIES".into(),
            character_set: None,
        }
    }

    /// The same series, but in the study of `study_seed`, as series number `number`.
    pub fn in_study(mut self, study_seed: u32, number: u32) -> Self {
        self.study_seed = study_seed;
        self.series_number = number;
        self
    }

    /// The same series, but for the patient of `patient_seed`.
    pub fn for_patient(mut self, patient_seed: u32) -> Self {
        self.patient_seed = Some(patient_seed);
        self
    }

    pub fn patient_id(&self) -> String {
        format!("SYN{}", self.patient_seed.unwrap_or(self.study_seed))
    }

    /// The same series for a patient named `name`, in `character_set` (e.g. `ISO_IR 100`).
    pub fn with_patient(mut self, name: &str, character_set: Option<&'static str>) -> Self {
        self.patient_name = name.into();
        self.character_set = character_set;
        self
    }

    // 2.25 UIDs (PS3.5 B.2) from fixed numbers, so tests are reproducible: the seed, one
    // digit for the level, five for the slice. The fixed width keeps seeds from colliding;
    // seeds start at 1, as a UID component may not start with 0.
    pub fn study_uid(&self) -> String {
        format!("2.25.{}100000", self.study_seed)
    }

    pub fn series_uid(&self) -> String {
        format!("2.25.{}200000", self.seed)
    }

    pub fn instance_uid(&self, slice: u32) -> String {
        format!("2.25.{}3{:05}", self.seed, slice + 1)
    }

    /// The pixel values of one frame of one slice: distinct per slice and frame.
    pub fn pixels(&self, slice: u32, frame: u32) -> Vec<u16> {
        (0..u32::from(self.rows) * u32::from(self.columns))
            .map(|i| (slice * 1000 + frame * 100 + i) as u16)
            .collect()
    }

    /// One slice as a DICOM Part 10 file in Explicit VR Little Endian, preamble included.
    pub fn slice_bytes(&self, slice: u32) -> Vec<u8> {
        self.multiframe_bytes(slice, 1)
    }

    /// One instance of `frames` native frames, Number of Frames set when more than one.
    pub fn multiframe_bytes(&self, slice: u32, frames: u32) -> Vec<u8> {
        let mut object = self.dataset(slice);
        if frames > 1 {
            object.put(DataElement::new(
                tags::NUMBER_OF_FRAMES,
                VR::IS,
                frames.to_string(),
            ));
        }
        let pixels: Vec<u16> = (0..frames).flat_map(|f| self.pixels(slice, f)).collect();
        object.put(DataElement::new(
            tags::PIXEL_DATA,
            VR::OW,
            PrimitiveValue::U16(pixels.into()),
        ));
        to_file(object, uids::EXPLICIT_VR_LITTLE_ENDIAN)
    }

    /// One instance with encapsulated pixel data in transfer syntax `ts`: the fragments
    /// are opaque bytes (nothing decodes them), with the given basic offset table.
    pub fn encapsulated_bytes(
        &self,
        slice: u32,
        ts: &str,
        frames: u32,
        offset_table: Vec<u32>,
        fragments: Vec<Vec<u8>>,
    ) -> Vec<u8> {
        let mut object = self.dataset(slice);
        object.put(DataElement::new(
            tags::NUMBER_OF_FRAMES,
            VR::IS,
            frames.to_string(),
        ));
        object.put(DataElement::new(
            tags::PIXEL_DATA,
            VR::OB,
            PixelFragmentSequence::new(offset_table, fragments),
        ));
        to_file(object, ts)
    }

    fn dataset(&self, slice: u32) -> InMemDicomObject {
        let z = slice as f64 * 2.5;
        let mut object = InMemDicomObject::from_element_iter([
            DataElement::new(tags::SOP_CLASS_UID, VR::UI, uids::CT_IMAGE_STORAGE),
            DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, self.instance_uid(slice)),
            DataElement::new(tags::MODALITY, VR::CS, "CT"),
            DataElement::new(tags::PATIENT_NAME, VR::PN, self.patient_name.as_str()),
            DataElement::new(tags::PATIENT_ID, VR::LO, self.patient_id()),
            DataElement::new(tags::PATIENT_BIRTH_DATE, VR::DA, "19700101"),
            DataElement::new(tags::PATIENT_SEX, VR::CS, "O"),
            DataElement::new(tags::STUDY_DATE, VR::DA, "20260101"),
            DataElement::new(tags::STUDY_TIME, VR::TM, "120000"),
            DataElement::new(
                tags::ACCESSION_NUMBER,
                VR::SH,
                format!("ACC{}", self.study_seed),
            ),
            DataElement::new(
                tags::REFERRING_PHYSICIAN_NAME,
                VR::PN,
                "REFERRING^PHYSICIAN",
            ),
            DataElement::new(tags::STUDY_ID, VR::SH, format!("S{}", self.study_seed)),
            DataElement::new(tags::STUDY_DESCRIPTION, VR::LO, "Synthetic study"),
            DataElement::new(
                tags::SERIES_DESCRIPTION,
                VR::LO,
                format!("Synthetic series {}", self.series_number),
            ),
            DataElement::new(tags::SLICE_THICKNESS, VR::DS, "2.5"),
            DataElement::new(
                tags::PIXEL_SPACING,
                VR::DS,
                dicom_value!(Strs, ["0.5".to_owned(), "0.5".to_owned()]),
            ),
            DataElement::new(tags::STUDY_INSTANCE_UID, VR::UI, self.study_uid()),
            DataElement::new(tags::SERIES_INSTANCE_UID, VR::UI, self.series_uid()),
            DataElement::new(tags::SERIES_NUMBER, VR::IS, self.series_number.to_string()),
            DataElement::new(tags::INSTANCE_NUMBER, VR::IS, (slice + 1).to_string()),
            DataElement::new(
                tags::IMAGE_POSITION_PATIENT,
                VR::DS,
                dicom_value!(Strs, ["0".to_owned(), "0".to_owned(), z.to_string()]),
            ),
            DataElement::new(tags::SAMPLES_PER_PIXEL, VR::US, PrimitiveValue::from(1_u16)),
            DataElement::new(tags::PHOTOMETRIC_INTERPRETATION, VR::CS, "MONOCHROME2"),
            DataElement::new(tags::ROWS, VR::US, PrimitiveValue::from(self.rows)),
            DataElement::new(tags::COLUMNS, VR::US, PrimitiveValue::from(self.columns)),
            DataElement::new(tags::BITS_ALLOCATED, VR::US, PrimitiveValue::from(16_u16)),
            DataElement::new(tags::BITS_STORED, VR::US, PrimitiveValue::from(12_u16)),
            DataElement::new(tags::HIGH_BIT, VR::US, PrimitiveValue::from(11_u16)),
            DataElement::new(
                tags::PIXEL_REPRESENTATION,
                VR::US,
                PrimitiveValue::from(0_u16),
            ),
        ]);
        if let Some(character_set) = self.character_set {
            object.put(DataElement::new(
                tags::SPECIFIC_CHARACTER_SET,
                VR::CS,
                character_set,
            ));
        }
        object
    }

    /// Every slice, in instance-number order.
    pub fn all_bytes(&self) -> Vec<Vec<u8>> {
        (0..self.slices).map(|s| self.slice_bytes(s)).collect()
    }
}

fn to_file(object: InMemDicomObject, ts: &str) -> Vec<u8> {
    let file = object
        .with_meta(FileMetaTableBuilder::new().transfer_syntax(ts))
        .expect("the synthetic data set has the SOP class and instance UIDs");
    let mut bytes = Vec::new();
    file.write_all(&mut bytes)
        .expect("the transfer syntax is known and writing to a Vec does not fail");
    bytes
}

/// A Part 10 file written byte by byte, for what an encoder would not write: elements in UN, as
/// GDCM leaves them when it re-encodes an Implicit VR file, and wrong lengths.
#[derive(Debug, Clone)]
pub struct RawFile {
    transfer_syntax: String,
    big: bool,
    data_set: Vec<u8>,
}

/// Undefined length.
pub const UNDEFINED: u32 = u32::MAX;
/// The head of an item, little endian; its length follows.
pub const ITEM: [u8; 4] = [0xFE, 0xFF, 0x00, 0xE0];
/// The delimiters with their zero lengths, little endian.
pub const ITEM_END: [u8; 8] = [0xFE, 0xFF, 0x0D, 0xE0, 0, 0, 0, 0];
pub const SEQUENCE_END: [u8; 8] = [0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0];

impl RawFile {
    /// An empty data set in this transfer syntax, written little endian.
    pub fn new(transfer_syntax: &str) -> Self {
        RawFile {
            transfer_syntax: transfer_syntax.into(),
            big: false,
            data_set: Vec::new(),
        }
    }

    /// An empty data set in Explicit VR Big Endian.
    pub fn big_endian() -> Self {
        RawFile {
            transfer_syntax: "1.2.840.10008.1.2.2".into(),
            big: true,
            data_set: Vec::new(),
        }
    }

    fn head(&mut self, tag: (u16, u16), vr: &[u8; 2]) {
        let (group, element) = if self.big {
            (tag.0.to_be_bytes(), tag.1.to_be_bytes())
        } else {
            (tag.0.to_le_bytes(), tag.1.to_le_bytes())
        };
        self.data_set.extend(group);
        self.data_set.extend(element);
        self.data_set.extend(vr);
    }

    /// An element with a 16-bit length (PS3.5 Table 7.1-2).
    pub fn short(mut self, tag: (u16, u16), vr: &[u8; 2], value: &[u8]) -> Self {
        self.head(tag, vr);
        let len = value.len() as u16;
        self.data_set.extend(if self.big {
            len.to_be_bytes()
        } else {
            len.to_le_bytes()
        });
        self.data_set.extend(value);
        self
    }

    /// An element with a 32-bit length (PS3.5 Table 7.1-1), which `len` may contradict.
    pub fn long_declared(mut self, tag: (u16, u16), vr: &[u8; 2], len: u32, value: &[u8]) -> Self {
        self.head(tag, vr);
        self.data_set.extend([0, 0]);
        self.data_set.extend(if self.big {
            len.to_be_bytes()
        } else {
            len.to_le_bytes()
        });
        self.data_set.extend(value);
        self
    }

    /// An element with a 32-bit length.
    pub fn long(self, tag: (u16, u16), vr: &[u8; 2], value: &[u8]) -> Self {
        self.long_declared(tag, vr, value.len() as u32, value)
    }

    /// Bytes as they are: elements of an implicit VR data set, anything else.
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        self.data_set.extend_from_slice(bytes);
        self
    }

    /// The UIDs an instance must have, those of `seed`, with `vr` written for Study Instance UID
    /// and in a 32-bit length if that is UN; ascending tags up to Series Instance UID.
    pub fn identifiers(self, seed: u32, study_vr: &[u8; 2]) -> Self {
        let study = even(&format!("2.25.{seed}1"));
        let with_sop = self
            .short((0x0008, 0x0016), b"UI", &even(uids::CT_IMAGE_STORAGE))
            .short((0x0008, 0x0018), b"UI", &even(&format!("2.25.{seed}3")));
        let with_study = if study_vr == b"UN" {
            with_sop.long((0x0020, 0x000D), b"UN", &study)
        } else {
            with_sop.short((0x0020, 0x000D), study_vr, &study)
        };
        with_study.short((0x0020, 0x000E), b"UI", &even(&format!("2.25.{seed}2")))
    }

    /// Encapsulated pixel data: an empty offset table and one fragment.
    pub fn one_fragment(self) -> Self {
        let mut value = ITEM.to_vec();
        value.extend(0u32.to_le_bytes());
        value.extend(ITEM);
        value.extend(4u32.to_le_bytes());
        value.extend([0xFF, 0xD8, 0xFF, 0xD9]);
        value.extend(SEQUENCE_END);
        self.long_declared((0x7FE0, 0x0010), b"OB", UNDEFINED, &value)
    }

    /// The whole file: preamble, File Meta group, data set.
    pub fn bytes(&self) -> Vec<u8> {
        let meta = |element: u8, vr: &[u8; 2], value: &[u8]| {
            let mut bytes = vec![0x02, 0x00, element, 0x00];
            bytes.extend(vr);
            bytes.extend((value.len() as u16).to_le_bytes());
            bytes.extend(value);
            bytes
        };
        let mut group = meta(0x02, b"UI", &even(uids::CT_IMAGE_STORAGE));
        group.extend(meta(0x03, b"UI", &even("2.25.1")));
        group.extend(meta(0x10, b"UI", &even(&self.transfer_syntax)));
        group.extend(meta(0x12, b"UI", &even("2.25.2")));
        let mut file = vec![0u8; 128];
        file.extend(b"DICM");
        file.extend(meta(0x00, b"UL", &(group.len() as u32).to_le_bytes()));
        file.extend(group);
        file.extend(&self.data_set);
        file
    }
}

/// `text` padded with NUL to an even length, as a UID is on the wire.
pub fn even(text: &str) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

/// An element in implicit VR, little endian, as a sequence in UN holds them.
pub fn implicit(tag: (u16, u16), value: &[u8]) -> Vec<u8> {
    let mut element = tag.0.to_le_bytes().to_vec();
    element.extend(tag.1.to_le_bytes());
    element.extend((value.len() as u32).to_le_bytes());
    element.extend(value);
    element
}

/// An item of undefined length around `content`, delimiter included, little endian.
pub fn item(content: &[u8]) -> Vec<u8> {
    let mut bytes = ITEM.to_vec();
    bytes.extend(UNDEFINED.to_le_bytes());
    bytes.extend(content);
    bytes.extend(ITEM_END);
    bytes
}

/// A sequence in implicit VR, with its delimiter, of one item that references an instance.
pub fn implicit_reference_sequence(instance: &str) -> Vec<u8> {
    let mut content = implicit((0x0008, 0x1150), &even(uids::CT_IMAGE_STORAGE));
    content.extend(implicit((0x0008, 0x1155), &even(instance)));
    let mut bytes = item(&content);
    bytes.extend(SEQUENCE_END);
    bytes
}

/// A JPEG Lossless SV1 file as GDCM leaves one it re-encoded from Implicit VR: Referenced Study
/// Sequence in UN with undefined length and implicit content.
pub fn un_sequence_file(seed: u32) -> Vec<u8> {
    RawFile::new(uids::JPEG_LOSSLESS_SV1)
        .short((0x0008, 0x0016), b"UI", &even(uids::CT_IMAGE_STORAGE))
        .short((0x0008, 0x0018), b"UI", &even(&format!("2.25.{seed}3")))
        .long_declared(
            (0x0008, 0x1110),
            b"UN",
            UNDEFINED,
            &implicit_reference_sequence(&format!("2.25.{seed}4")),
        )
        .short((0x0020, 0x000D), b"UI", &even(&format!("2.25.{seed}1")))
        .short((0x0020, 0x000E), b"UI", &even(&format!("2.25.{seed}2")))
        .one_fragment()
        .bytes()
}

/// The same with Study Instance UID, a standard element, in UN with a defined length.
pub fn un_study_uid_file(seed: u32) -> Vec<u8> {
    RawFile::new(uids::JPEG_LOSSLESS_SV1)
        .identifiers(seed, b"UN")
        .one_fragment()
        .bytes()
}
