//! The sample study as test data: the three slices of a real CT in `docs/sample` (LiTS
//! volume 0, see `docs/sample-study.md`), under other identifiers, for the tests that need
//! several patients, studies and series.
//!
//! What a [`SampleSeries`] writes is the real file but for what tells one entity from
//! another: the UIDs, the Patient ID, the Accession Number, the Study ID, the series and
//! instance numbers, and what a test sets by name (a patient's name, a study's date and
//! time, a modality). The pixel data and every other attribute are the file's own.
//!
//! The files are read from the checkout, so this is for tests run from it, not for a
//! published crate. They are shared under CC BY-NC-SA 4.0, not under pacs-rs's licences.

use std::sync::OnceLock;

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

/// How many slices the sample study has.
pub const SLICES: u32 = 3;

/// The data sets of the sample's slices, in instance-number order, read once.
fn slices() -> &'static [InMemDicomObject] {
    static READ: OnceLock<Vec<InMemDicomObject>> = OnceLock::new();
    READ.get_or_init(|| {
        (1..=SLICES)
            .map(|number| {
                let path = format!(
                    "{}/../../docs/sample/{number:05}.dcm",
                    env!("CARGO_MANIFEST_DIR")
                );
                dicom_object::open_file(&path)
                    .unwrap_or_else(|e| panic!("the sample study's {path}: {e}"))
                    .into_inner()
            })
            .collect()
    })
}

/// The same data sets without their pixel data: what an index keeps of an instance.
fn headers() -> &'static [InMemDicomObject] {
    static READ: OnceLock<Vec<InMemDicomObject>> = OnceLock::new();
    READ.get_or_init(|| {
        slices()
            .iter()
            .map(|slice| {
                let mut header = slice.clone();
                header.remove_element(tags::PIXEL_DATA);
                header
            })
            .collect()
    })
}

/// A series made of the sample study's slices: `slices` of them, from the first.
#[derive(Debug, Clone)]
pub struct SampleSeries {
    /// Distinguishes series made in the same test; goes into the series and instance UIDs.
    pub seed: u32,
    /// Goes into the study UID and the Patient ID; series with the same one share a study.
    pub study_seed: u32,
    /// Goes into the Patient ID instead of `study_seed`, so studies can share a patient.
    pub patient_seed: Option<u32>,
    pub series_number: u32,
    pub slices: u32,
    /// Patient's Name, and the Specific Character Set it needs; the file's own unless set.
    patient: Option<(String, Option<&'static str>)>,
    /// Study Date and Study Time; the file's own unless set.
    at: Option<(String, String)>,
    /// The file's own (CT) unless set.
    modality: Option<String>,
}

impl SampleSeries {
    /// `slices` is 3 at most: what the sample study has.
    pub fn new(seed: u32, slices: u32) -> Self {
        assert!(
            (1..=SLICES).contains(&slices),
            "the sample study has {SLICES} slices, not {slices}"
        );
        SampleSeries {
            seed,
            study_seed: seed,
            patient_seed: None,
            series_number: 1,
            slices,
            patient: None,
            at: None,
            modality: None,
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
        format!("LITS-{}", self.patient_seed.unwrap_or(self.study_seed))
    }

    /// The same series, of a study at `date` (YYYYMMDD) and `time` (HHMMSS).
    pub fn at(mut self, date: &str, time: &str) -> Self {
        self.at = Some((date.into(), time.into()));
        self
    }

    /// The same series, said to be of another modality than CT.
    pub fn of_modality(mut self, modality: &str) -> Self {
        self.modality = Some(modality.into());
        self
    }

    /// The same series for a patient named `name`, in `character_set` (e.g. `ISO_IR 100`).
    pub fn with_patient(mut self, name: &str, character_set: Option<&'static str>) -> Self {
        self.patient = Some((name.into(), character_set));
        self
    }

    // 2.25 UIDs (PS3.5 B.2) from fixed numbers, as the synthetic series has them: the seed,
    // one digit for the level, five for the slice.
    pub fn study_uid(&self) -> String {
        format!("2.25.{}100000", self.study_seed)
    }

    pub fn series_uid(&self) -> String {
        format!("2.25.{}200000", self.seed)
    }

    pub fn instance_uid(&self, slice: u32) -> String {
        format!("2.25.{}3{:05}", self.seed, slice + 1)
    }

    /// The pixel data of one slice as it is in the file: 512 x 512 values of 16 bits. The
    /// sample study has one frame per slice, and `frame` is not read.
    pub fn pixels(&self, slice: u32, _frame: u32) -> Vec<u16> {
        slices()[slice as usize]
            .element(tags::PIXEL_DATA)
            .expect("the sample study has pixel data")
            .to_bytes()
            .expect("the sample study's pixel data is native")
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

    /// One slice as a data set: the sample's, under this series' identifiers.
    pub fn slice_object(&self, slice: u32) -> InMemDicomObject {
        self.renamed(slice, slices()[slice as usize].clone())
    }

    /// `object`, a slice of the sample, under this series' identifiers.
    fn renamed(&self, slice: u32, mut object: InMemDicomObject) -> InMemDicomObject {
        let mut put = |tag, vr, value: String| {
            object.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
        };
        put(tags::SOP_INSTANCE_UID, VR::UI, self.instance_uid(slice));
        put(tags::STUDY_INSTANCE_UID, VR::UI, self.study_uid());
        put(tags::SERIES_INSTANCE_UID, VR::UI, self.series_uid());
        put(tags::PATIENT_ID, VR::LO, self.patient_id());
        put(
            tags::ACCESSION_NUMBER,
            VR::SH,
            format!("ACC{}", self.study_seed),
        );
        put(tags::STUDY_ID, VR::SH, format!("S{}", self.study_seed));
        put(tags::SERIES_NUMBER, VR::IS, self.series_number.to_string());
        put(tags::INSTANCE_NUMBER, VR::IS, (slice + 1).to_string());
        if let Some((name, character_set)) = &self.patient {
            put(tags::PATIENT_NAME, VR::PN, name.clone());
            if let Some(character_set) = character_set {
                put(
                    tags::SPECIFIC_CHARACTER_SET,
                    VR::CS,
                    (*character_set).to_owned(),
                );
            }
        }
        if let Some((date, time)) = &self.at {
            put(tags::STUDY_DATE, VR::DA, date.clone());
            put(tags::STUDY_TIME, VR::TM, time.clone());
        }
        if let Some(modality) = &self.modality {
            put(tags::MODALITY, VR::CS, modality.clone());
        }
        object
    }

    /// One slice as a DICOM Part 10 file in Explicit VR Little Endian, as the sample is.
    pub fn slice_bytes(&self, slice: u32) -> Vec<u8> {
        to_file(self.slice_object(slice))
    }

    /// One slice's file without its pixel data: enough for an index that is filled
    /// without a store, when a test searches thousands of studies and retrieves none.
    pub fn header_bytes(&self, slice: u32) -> Vec<u8> {
        to_file(self.renamed(slice, headers()[slice as usize].clone()))
    }

    /// Every slice, in instance-number order.
    pub fn all_bytes(&self) -> Vec<Vec<u8>> {
        (0..self.slices).map(|s| self.slice_bytes(s)).collect()
    }
}

fn to_file(object: InMemDicomObject) -> Vec<u8> {
    let file = object
        .with_meta(FileMetaTableBuilder::new().transfer_syntax("1.2.840.10008.1.2.1"))
        .expect("the sample study has the SOP class and instance UIDs");
    let mut bytes = Vec::new();
    file.write_all(&mut bytes)
        .expect("writing to a Vec does not fail");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Instance;

    #[test]
    fn a_slice_is_the_real_one_under_other_identifiers() {
        let series = SampleSeries::new(7, 3)
            .in_study(5, 2)
            .for_patient(4)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000");
        let original = slices();
        for slice in 0..3 {
            let instance = Instance::from_bytes(&series.slice_bytes(slice)).unwrap();
            assert_eq!(instance.key.sop_instance_uid, series.instance_uid(slice));
            assert_eq!(instance.key.study_instance_uid, "2.25.5100000");
            assert_eq!(instance.key.series_instance_uid, "2.25.7200000");
            assert_eq!(instance.string(tags::PATIENT_ID).unwrap(), "LITS-4");
            assert_eq!(instance.string(tags::PATIENT_NAME).unwrap(), "Doe^Jane");
            assert_eq!(instance.string(tags::STUDY_DATE).unwrap(), "20260105");
            assert_eq!(instance.int(tags::SERIES_NUMBER), Some(2));
            assert_eq!(
                instance.int(tags::INSTANCE_NUMBER),
                Some(i64::from(slice) + 1)
            );
            // what was not named is the file's own: a real CT of 512 by 512
            assert_eq!(instance.int(tags::ROWS), Some(512));
            assert_eq!(instance.string(tags::MODALITY).unwrap(), "CT");
            assert_eq!(
                instance.string(tags::STUDY_DESCRIPTION),
                original[slice as usize]
                    .element(tags::STUDY_DESCRIPTION)
                    .unwrap()
                    .to_str()
                    .ok()
                    .map(|s| s.into_owned())
            );
            let shape = instance.shape().unwrap();
            let native = instance.native_frame(1, shape).unwrap();
            let pixels: Vec<u16> = native
                .chunks(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            assert_eq!(pixels.len(), 512 * 512);
            assert_eq!(pixels, series.pixels(slice, 0));
        }
        // the three slices are three images, not one thrice
        assert_ne!(series.pixels(0, 0), series.pixels(1, 0));
        // the header alone names the same instance, and has no pixels
        let header = Instance::from_bytes(&series.header_bytes(1)).unwrap();
        assert_eq!(header.key.sop_instance_uid, series.instance_uid(1));
        assert_eq!(header.string(tags::PATIENT_NAME).unwrap(), "Doe^Jane");
        assert_eq!(header.int(tags::ROWS), Some(512));
        assert!(series.header_bytes(1).len() < 2000);
        // left as the file has them
        let plain = Instance::from_bytes(&SampleSeries::new(9, 1).slice_bytes(0)).unwrap();
        assert_eq!(plain.string(tags::PATIENT_NAME).unwrap(), "VOLUME 0^LITS");
        assert_eq!(plain.string(tags::PATIENT_ID).unwrap(), "LITS-9");
        assert_eq!(plain.string(tags::STUDY_DATE).unwrap(), "20261117");
    }

    #[test]
    #[should_panic(expected = "the sample study has 3 slices")]
    fn a_series_has_no_more_slices_than_the_sample() {
        SampleSeries::new(1, 4);
    }
}
