//! Real DICOM as test data: two files made by scanners, a CT and an MR, from pydicom's test
//! data (`tests/data/pydicom`, with where they come from and under what terms), written
//! under other identifiers for the tests that need several patients, studies and series.
//!
//! What a [`RealSeries`] writes is the scanner's file but for what tells one entity from
//! another: the UIDs, the Patient ID, the Accession Number, the Study ID, the series and
//! instance numbers, and what a test sets by name (a patient's name, a study's date and
//! time, a modality). The pixel data and every other attribute are the file's own. There
//! is one image of each kind, so the slices of a series are that image under as many SOP
//! Instance UIDs.
//!
//! The files are read from the checkout, so this is for tests run from it, not for a
//! published crate.

use std::sync::OnceLock;

use dicom_core::{DataElement, PrimitiveValue, VR};
use dicom_dictionary_std::tags;
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};

/// Which of the two files a series is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scanned {
    /// `CT_small.dcm`: CT Image Storage, 128 x 128.
    Ct,
    /// `MR_small.dcm`: MR Image Storage, 64 x 64.
    Mr,
}

/// The data sets of the two files, read once: the CT, then the MR.
fn files() -> &'static [InMemDicomObject; 2] {
    static READ: OnceLock<[InMemDicomObject; 2]> = OnceLock::new();
    READ.get_or_init(|| {
        ["CT_small.dcm", "MR_small.dcm"].map(|name| {
            let path = format!("{}/tests/data/pydicom/{name}", env!("CARGO_MANIFEST_DIR"));
            dicom_object::open_file(&path)
                .unwrap_or_else(|e| panic!("the test file {path}: {e}"))
                .into_inner()
        })
    })
}

/// The same data sets without their pixel data: what an index keeps of an instance.
fn headers() -> &'static [InMemDicomObject; 2] {
    static READ: OnceLock<[InMemDicomObject; 2]> = OnceLock::new();
    READ.get_or_init(|| {
        files().clone().map(|mut header| {
            header.remove_element(tags::PIXEL_DATA);
            header
        })
    })
}

/// A series of a scanner's image: `slices` instances of it.
#[derive(Debug, Clone)]
pub struct RealSeries {
    /// Distinguishes series made in the same test; goes into the series and instance UIDs.
    pub seed: u32,
    /// Goes into the study UID and the Patient ID; series with the same one share a study.
    pub study_seed: u32,
    /// Goes into the Patient ID instead of `study_seed`, so studies can share a patient.
    pub patient_seed: Option<u32>,
    pub series_number: u32,
    pub slices: u32,
    scanned: Scanned,
    /// Patient's Name, and the Specific Character Set it needs; the file's own unless set.
    patient: Option<(String, Option<&'static str>)>,
    /// Study Date and Study Time; the file's own unless set.
    at: Option<(String, String)>,
    /// The file's own unless set.
    modality: Option<String>,
}

impl RealSeries {
    /// A series of the CT image.
    pub fn new(seed: u32, slices: u32) -> Self {
        RealSeries {
            seed,
            study_seed: seed,
            patient_seed: None,
            series_number: 1,
            slices,
            scanned: Scanned::Ct,
            patient: None,
            at: None,
            modality: None,
        }
    }

    /// A series of the MR image: another SOP class, MR Image Storage.
    pub fn mr(seed: u32, slices: u32) -> Self {
        RealSeries {
            scanned: Scanned::Mr,
            ..RealSeries::new(seed, slices)
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
        format!("NEMA-{}", self.patient_seed.unwrap_or(self.study_seed))
    }

    /// The same series, of a study at `date` (YYYYMMDD) and `time` (HHMMSS).
    pub fn at(mut self, date: &str, time: &str) -> Self {
        self.at = Some((date.into(), time.into()));
        self
    }

    /// The same series, with Modality said to be `modality`: the image and its SOP class
    /// stay what they are. For a real MR, see [`RealSeries::mr`].
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

    /// The pixel data as it is in the file, values of 16 bits: the same for every slice,
    /// the file having one image of one frame.
    pub fn pixels(&self, _slice: u32, _frame: u32) -> Vec<u16> {
        files()[self.scanned as usize]
            .element(tags::PIXEL_DATA)
            .expect("the file has pixel data")
            .to_bytes()
            .expect("the file's pixel data is native")
            .chunks(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect()
    }

    /// One slice as a data set: the file's, under this series' identifiers.
    pub fn slice_object(&self, slice: u32) -> InMemDicomObject {
        self.renamed(slice, files()[self.scanned as usize].clone())
    }

    /// `object`, the file's data set or its header, under this series' identifiers.
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

    /// One slice as a DICOM Part 10 file in Explicit VR Little Endian, as the file is.
    pub fn slice_bytes(&self, slice: u32) -> Vec<u8> {
        to_file(self.slice_object(slice))
    }

    /// One slice's file without its pixel data: enough for an index that is filled
    /// without a store, when a test searches thousands of studies and retrieves none.
    pub fn header_bytes(&self, slice: u32) -> Vec<u8> {
        to_file(self.renamed(slice, headers()[self.scanned as usize].clone()))
    }

    /// Every slice, in instance-number order.
    pub fn all_bytes(&self) -> Vec<Vec<u8>> {
        (0..self.slices).map(|s| self.slice_bytes(s)).collect()
    }
}

fn to_file(object: InMemDicomObject) -> Vec<u8> {
    let file = object
        .with_meta(FileMetaTableBuilder::new().transfer_syntax("1.2.840.10008.1.2.1"))
        .expect("the file has the SOP class and instance UIDs");
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
    fn a_slice_is_the_scanner_s_file_under_other_identifiers() {
        let series = RealSeries::new(7, 3)
            .in_study(5, 2)
            .for_patient(4)
            .with_patient("Doe^Jane", None)
            .at("20260105", "100000");
        for slice in 0..3 {
            let instance = Instance::from_bytes(&series.slice_bytes(slice)).unwrap();
            assert_eq!(instance.key.sop_instance_uid, series.instance_uid(slice));
            assert_eq!(instance.key.study_instance_uid, "2.25.5100000");
            assert_eq!(instance.key.series_instance_uid, "2.25.7200000");
            assert_eq!(instance.string(tags::PATIENT_ID).unwrap(), "NEMA-4");
            assert_eq!(instance.string(tags::PATIENT_NAME).unwrap(), "Doe^Jane");
            assert_eq!(instance.string(tags::STUDY_DATE).unwrap(), "20260105");
            assert_eq!(instance.int(tags::SERIES_NUMBER), Some(2));
            assert_eq!(
                instance.int(tags::INSTANCE_NUMBER),
                Some(i64::from(slice) + 1)
            );
            // what was not named is the file's own: a CT of 128 by 128 from a GE scanner
            assert_eq!(instance.int(tags::ROWS), Some(128));
            assert_eq!(instance.string(tags::MODALITY).unwrap(), "CT");
            assert_eq!(
                instance.string(tags::MANUFACTURER).unwrap(),
                "GE MEDICAL SYSTEMS"
            );
            let shape = instance.shape().unwrap();
            let native = instance.native_frame(1, shape).unwrap();
            let pixels: Vec<u16> = native
                .chunks(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            assert_eq!(pixels.len(), 128 * 128);
            assert_eq!(pixels, series.pixels(slice, 0));
        }
        // the header alone names the same instance, and has no pixels
        let header = Instance::from_bytes(&series.header_bytes(1)).unwrap();
        assert_eq!(header.key.sop_instance_uid, series.instance_uid(1));
        assert_eq!(header.string(tags::PATIENT_NAME).unwrap(), "Doe^Jane");
        assert_eq!(header.int(tags::ROWS), Some(128));
        assert!(series.header_bytes(1).len() < series.slice_bytes(1).len() / 4);
        // left as the file has them
        let plain = Instance::from_bytes(&RealSeries::new(9, 1).slice_bytes(0)).unwrap();
        assert_eq!(
            plain.string(tags::PATIENT_NAME).unwrap(),
            "CompressedSamples^CT1"
        );
        assert_eq!(plain.string(tags::PATIENT_ID).unwrap(), "NEMA-9");
        assert_eq!(plain.string(tags::STUDY_DATE).unwrap(), "20040119");
    }

    #[test]
    fn the_mr_is_another_image_of_another_sop_class() {
        let series = RealSeries::mr(8, 1);
        let instance = Instance::from_bytes(&series.slice_bytes(0)).unwrap();
        assert_eq!(instance.key.sop_class_uid, "1.2.840.10008.5.1.4.1.1.4");
        assert_eq!(instance.string(tags::MODALITY).unwrap(), "MR");
        assert_eq!(instance.int(tags::ROWS), Some(64));
        assert_eq!(series.pixels(0, 0).len(), 64 * 64);
        assert_eq!(
            Instance::from_bytes(&RealSeries::new(8, 1).slice_bytes(0))
                .unwrap()
                .key
                .sop_class_uid,
            "1.2.840.10008.5.1.4.1.1.2"
        );
    }

    /// The files are the ones pydicom ships: `tests/data/pydicom/README.md` has their
    /// origin and these sizes' files' hashes.
    #[test]
    fn the_files_are_the_ones_the_readme_names() {
        for (name, size) in [("CT_small.dcm", 39206), ("MR_small.dcm", 9830)] {
            let path = format!("{}/tests/data/pydicom/{name}", env!("CARGO_MANIFEST_DIR"));
            assert_eq!(std::fs::metadata(&path).unwrap().len(), size, "{name}");
        }
    }
}
