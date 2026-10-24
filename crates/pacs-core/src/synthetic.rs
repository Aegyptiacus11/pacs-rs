//! Synthetic CT series for tests: small, deterministic, no patient data.

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
    pub series_number: u32,
    pub slices: u32,
    pub rows: u16,
    pub columns: u16,
}

impl SyntheticSeries {
    pub fn new(seed: u32, slices: u32) -> Self {
        SyntheticSeries {
            seed,
            study_seed: seed,
            series_number: 1,
            slices,
            rows: 4,
            columns: 4,
        }
    }

    /// The same series, but in the study of `study_seed`, as series number `number`.
    pub fn in_study(mut self, study_seed: u32, number: u32) -> Self {
        self.study_seed = study_seed;
        self.series_number = number;
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

    /// One slice as a DICOM Part 10 file in Explicit VR Little Endian, preamble included.
    pub fn slice_bytes(&self, slice: u32) -> Vec<u8> {
        let pixels: Vec<u16> = (0..u32::from(self.rows) * u32::from(self.columns))
            .map(|i| (slice * 100 + i) as u16)
            .collect();
        let z = slice as f64 * 2.5;
        let object = InMemDicomObject::from_element_iter([
            DataElement::new(tags::SOP_CLASS_UID, VR::UI, uids::CT_IMAGE_STORAGE),
            DataElement::new(tags::SOP_INSTANCE_UID, VR::UI, self.instance_uid(slice)),
            DataElement::new(tags::MODALITY, VR::CS, "CT"),
            DataElement::new(tags::PATIENT_NAME, VR::PN, "SYNTHETIC^SERIES"),
            DataElement::new(tags::PATIENT_ID, VR::LO, format!("SYN{}", self.study_seed)),
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
            DataElement::new(tags::PIXEL_DATA, VR::OW, PrimitiveValue::U16(pixels.into())),
        ]);
        let file = object
            .with_meta(FileMetaTableBuilder::new().transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN))
            .expect("the synthetic data set has the SOP class and instance UIDs");
        let mut bytes = Vec::new();
        file.write_all(&mut bytes)
            .expect("writing to a Vec does not fail");
        bytes
    }

    /// Every slice, in instance-number order.
    pub fn all_bytes(&self) -> Vec<Vec<u8>> {
        (0..self.slices).map(|s| self.slice_bytes(s)).collect()
    }
}
