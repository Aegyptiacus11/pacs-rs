//! Real DICOM as test data: files from pydicom's test data (`tests/data/pydicom`, with
//! where each comes from and under what terms), written under other identifiers for the
//! tests that need several patients, studies and series.
//!
//! What a [`RealSeries`] writes is the file but for what tells one entity from another:
//! the UIDs, the Patient ID, the Accession Number, the Study ID, the series and instance
//! numbers, and what a test sets by name (a patient's name, a study's date and time, a
//! modality). The pixel data and every other attribute are the file's own. There is one
//! image of each kind, so the slices of a series are that image under as many SOP Instance
//! UIDs.
//!
//! Beside the CT and the MR of one frame there are an MR of ten frames, the same MRs in
//! JPEG-LS and a nuclear medicine image in lossy JPEG, for the tests of frames and of
//! transfer syntaxes. [`RealSeries::refragmented`] is the one alteration of pixel data:
//! the real codestreams, cut into other fragments. [`AlteredFile`] is the CT file with
//! elements written wrongly in place of its own, for the tests of what a reader must
//! survive.
//!
//! The files are read from the checkout, so this is for tests run from it, not for a
//! published crate.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use dicom_core::value::{PixelFragmentSequence, Value as DicomValue};
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::{FileMetaTableBuilder, InMemDicomObject};
use dicom_transfer_syntax_registry::entries::{
    EXPLICIT_VR_BIG_ENDIAN, EXPLICIT_VR_LITTLE_ENDIAN, IMPLICIT_VR_LITTLE_ENDIAN,
};

/// The files, by what a test asks of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scanned {
    /// `CT_small.dcm`: CT Image Storage, 128 x 128.
    Ct,
    /// `MR_small.dcm`: MR Image Storage, 64 x 64.
    Mr,
    /// `MR_small_jpeg_ls_lossless.dcm`: the MR in JPEG-LS Lossless, one fragment.
    MrJpegLs,
    /// `emri_small.dcm`: Enhanced MR Image Storage, ten frames of 64 x 64.
    Frames,
    /// `emri_small_jpeg_ls_lossless.dcm`: the same in JPEG-LS Lossless, a fragment a frame.
    FramesJpegLs,
    /// `JPEG-lossy.dcm`: a nuclear medicine image in JPEG Extended (Process 2 and 4).
    JpegLossy,
}

const NAMES: [&str; 6] = [
    "CT_small.dcm",
    "MR_small.dcm",
    "MR_small_jpeg_ls_lossless.dcm",
    "emri_small.dcm",
    "emri_small_jpeg_ls_lossless.dcm",
    "JPEG-lossy.dcm",
];

/// Where a file of `tests/data/pydicom` is in the checkout.
pub fn path(name: &str) -> String {
    format!("{}/tests/data/pydicom/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// A file of `tests/data/pydicom` as it is, byte for byte.
pub fn file_bytes(name: &str) -> Vec<u8> {
    let path = path(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("the test file {path}: {e}"))
}

/// The data sets of the files, read once, each with its transfer syntax.
fn files() -> &'static [(InMemDicomObject, String); 6] {
    static READ: OnceLock<[(InMemDicomObject, String); 6]> = OnceLock::new();
    READ.get_or_init(|| {
        NAMES.map(|name| {
            let path = path(name);
            let file = dicom_object::open_file(&path)
                .unwrap_or_else(|e| panic!("the test file {path}: {e}"));
            let syntax = file.meta().transfer_syntax().to_owned();
            (file.into_inner(), syntax)
        })
    })
}

/// The CT's and the MR's data sets without their pixel data: what an index keeps of an
/// instance.
fn headers() -> &'static [InMemDicomObject; 2] {
    static READ: OnceLock<[InMemDicomObject; 2]> = OnceLock::new();
    READ.get_or_init(|| {
        [Scanned::Ct, Scanned::Mr].map(|scanned| {
            let mut header = files()[scanned as usize].0.clone();
            header.remove_element(tags::PIXEL_DATA);
            header
        })
    })
}

/// The values of 16 bits of native pixel data.
fn words(object: &InMemDicomObject) -> Vec<u16> {
    object
        .element(tags::PIXEL_DATA)
        .expect("the file has pixel data")
        .to_bytes()
        .expect("the file's pixel data is native")
        .chunks(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect()
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
    /// Patient's Birth Date; the file's own (the CT's and the MR's are empty) unless set.
    born: Option<String>,
    /// Referring Physician's Name; the file's own (empty in both) unless set.
    referrer: Option<String>,
    /// Series Description; the file's own (neither has one) unless set.
    description: Option<String>,
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
            born: None,
            referrer: None,
            description: None,
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

    /// The same series, of a patient born on `date` (YYYYMMDD).
    pub fn born(mut self, date: &str) -> Self {
        self.born = Some(date.into());
        self
    }

    /// The same series, described as `text` (Series Description).
    pub fn described(mut self, text: &str) -> Self {
        self.description = Some(text.into());
        self
    }

    /// The same series, of a study asked for by `name`.
    pub fn referred_by(mut self, name: &str) -> Self {
        self.referrer = Some(name.into());
        self
    }

    /// The same series for a patient named `name`, in `character_set` (e.g. `ISO_IR 100`).
    pub fn with_patient(mut self, name: &str, character_set: Option<&'static str>) -> Self {
        self.patient = Some((name.into(), character_set));
        self
    }

    // 2.25 UIDs (PS3.5 B.2) from fixed numbers, so tests are reproducible: the seed,
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
        words(&files()[self.scanned as usize].0)
    }

    /// One slice as a data set: the file's, under this series' identifiers.
    pub fn slice_object(&self, slice: u32) -> InMemDicomObject {
        self.renamed(slice, files()[self.scanned as usize].0.clone())
    }

    /// The frames of [`RealSeries::multiframe_bytes`].
    pub const FRAMES: u32 = 10;

    /// The file of ten frames (an Enhanced MR, native pixel data) under this series'
    /// identifiers, as instance `slice`.
    pub fn multiframe_bytes(&self, slice: u32) -> Vec<u8> {
        self.file_of(Scanned::Frames, slice)
    }

    /// One frame (from 0) of [`RealSeries::multiframe_bytes`], values of 16 bits.
    pub fn frame_pixels(frame: u32) -> Vec<u16> {
        let all = words(&files()[Scanned::Frames as usize].0);
        let size = all.len() / Self::FRAMES as usize;
        all[frame as usize * size..(frame as usize + 1) * size].to_vec()
    }

    /// The MR in JPEG-LS Lossless as the file has it (one frame, one fragment), under this
    /// series' identifiers.
    pub fn jpeg_ls_bytes(&self, slice: u32) -> Vec<u8> {
        self.file_of(Scanned::MrJpegLs, slice)
    }

    /// The file of ten frames in JPEG-LS Lossless as it is (a fragment a frame, an empty
    /// offset table), under this series' identifiers.
    pub fn jpeg_ls_multiframe_bytes(&self, slice: u32) -> Vec<u8> {
        self.file_of(Scanned::FramesJpegLs, slice)
    }

    /// The nuclear medicine image in lossy JPEG, a transfer syntax pacs-rs has no codec
    /// for (`1.2.840.10008.1.2.4.51`), under this series' identifiers.
    pub fn jpeg_lossy_bytes(&self, slice: u32) -> Vec<u8> {
        self.file_of(Scanned::JpegLossy, slice)
    }

    /// The JPEG-LS codestreams of the file of ten frames, one a frame, as its fragments
    /// hold them.
    pub fn jpeg_ls_frames() -> Vec<Vec<u8>> {
        match files()[Scanned::FramesJpegLs as usize]
            .0
            .element(tags::PIXEL_DATA)
            .expect("the file has pixel data")
            .value()
        {
            DicomValue::PixelSequence(sequence) => sequence.fragments().to_vec(),
            _ => panic!("the file's pixel data is encapsulated"),
        }
    }

    /// The file of ten frames in JPEG-LS with its pixel data cut otherwise: said to have
    /// `frames` frames, in these fragments, behind this basic offset table. For what no
    /// file at hand has: a frame in several fragments, an offset table that is not empty.
    pub fn refragmented(
        &self,
        slice: u32,
        frames: u32,
        offset_table: Vec<u32>,
        fragments: Vec<Vec<u8>>,
    ) -> Vec<u8> {
        let (object, syntax) = &files()[Scanned::FramesJpegLs as usize];
        let mut object = self.renamed(slice, object.clone());
        object.put(DataElement::new(
            tags::NUMBER_OF_FRAMES,
            VR::IS,
            PrimitiveValue::from(frames.to_string()),
        ));
        object.put(DataElement::new(
            tags::PIXEL_DATA,
            VR::OB,
            PixelFragmentSequence::new(offset_table, fragments),
        ));
        to_file(object, syntax)
    }

    /// One of the files under this series' identifiers, in its own transfer syntax.
    fn file_of(&self, scanned: Scanned, slice: u32) -> Vec<u8> {
        let (object, syntax) = &files()[scanned as usize];
        to_file(self.renamed(slice, object.clone()), syntax)
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
        if let Some(born) = &self.born {
            put(tags::PATIENT_BIRTH_DATE, VR::DA, born.clone());
        }
        if let Some(description) = &self.description {
            put(tags::SERIES_DESCRIPTION, VR::LO, description.clone());
        }
        if let Some(referrer) = &self.referrer {
            put(tags::REFERRING_PHYSICIAN_NAME, VR::PN, referrer.clone());
        }
        object
    }

    /// One slice as a DICOM Part 10 file in Explicit VR Little Endian, as the file is.
    pub fn slice_bytes(&self, slice: u32) -> Vec<u8> {
        self.file_of(self.scanned, slice)
    }

    /// One slice's file without its pixel data: enough for an index that is filled
    /// without a store, when a test searches thousands of studies and retrieves none.
    pub fn header_bytes(&self, slice: u32) -> Vec<u8> {
        to_file(
            self.renamed(slice, headers()[self.scanned as usize].clone()),
            "1.2.840.10008.1.2.1",
        )
    }

    /// Every slice, in instance-number order.
    pub fn all_bytes(&self) -> Vec<Vec<u8>> {
        (0..self.slices).map(|s| self.slice_bytes(s)).collect()
    }
}

fn to_file(object: InMemDicomObject, transfer_syntax: &str) -> Vec<u8> {
    let file = object
        .with_meta(FileMetaTableBuilder::new().transfer_syntax(transfer_syntax))
        .expect("the file has the SOP class and instance UIDs");
    let mut bytes = Vec::new();
    file.write_all(&mut bytes)
        .expect("writing to a Vec does not fail");
    bytes
}

/// The CT file altered into what no encoder writes: elements in UN, as GDCM leaves them when
/// it re-encodes an Implicit VR file, and lengths that lie. The data set is the scanner's;
/// what a test names is written in its place byte by byte, each element where its tag
/// belongs.
///
/// The file starts without the four UIDs that identify an instance, which
/// [`AlteredFile::identifiers`] writes, so a test also has the file that lacks them.
#[derive(Debug, Clone)]
pub struct AlteredFile {
    transfer_syntax: String,
    big: bool,
    /// What is written in place of the file's own, by tag.
    put: BTreeMap<(u16, u16), Vec<u8>>,
}

/// Undefined length.
pub const UNDEFINED: u32 = u32::MAX;
/// The head of an item, little endian; its length follows.
pub const ITEM: [u8; 4] = [0xFE, 0xFF, 0x00, 0xE0];
/// The delimiters with their zero lengths, little endian.
pub const ITEM_END: [u8; 8] = [0xFE, 0xFF, 0x0D, 0xE0, 0, 0, 0, 0];
pub const SEQUENCE_END: [u8; 8] = [0xFE, 0xFF, 0xDD, 0xE0, 0, 0, 0, 0];

const IMPLICIT_VR_LE: &str = "1.2.840.10008.1.2";
const EXPLICIT_VR_BE: &str = "1.2.840.10008.1.2.2";
const IDENTIFYING: [Tag; 4] = [
    tags::SOP_CLASS_UID,
    tags::SOP_INSTANCE_UID,
    tags::STUDY_INSTANCE_UID,
    tags::SERIES_INSTANCE_UID,
];

impl AlteredFile {
    /// The CT file said to be in this transfer syntax, its data set written little endian
    /// (in implicit VR for Implicit VR Little Endian, in explicit VR otherwise).
    pub fn new(transfer_syntax: &str) -> Self {
        AlteredFile {
            transfer_syntax: transfer_syntax.into(),
            big: false,
            put: BTreeMap::new(),
        }
    }

    /// The CT file in Explicit VR Big Endian.
    pub fn big_endian() -> Self {
        AlteredFile {
            transfer_syntax: EXPLICIT_VR_BE.into(),
            big: true,
            put: BTreeMap::new(),
        }
    }

    fn head(&self, tag: (u16, u16), vr: &[u8; 2]) -> Vec<u8> {
        let (group, element) = if self.big {
            (tag.0.to_be_bytes(), tag.1.to_be_bytes())
        } else {
            (tag.0.to_le_bytes(), tag.1.to_le_bytes())
        };
        let mut bytes = group.to_vec();
        bytes.extend(element);
        bytes.extend(vr);
        bytes
    }

    /// An element with a 16-bit length (PS3.5 Table 7.1-2), in place of the file's.
    pub fn short(mut self, tag: (u16, u16), vr: &[u8; 2], value: &[u8]) -> Self {
        let mut bytes = self.head(tag, vr);
        let len = value.len() as u16;
        bytes.extend(if self.big {
            len.to_be_bytes()
        } else {
            len.to_le_bytes()
        });
        bytes.extend(value);
        self.put.insert(tag, bytes);
        self
    }

    /// An element with a 32-bit length (PS3.5 Table 7.1-1), which `len` may contradict.
    pub fn long_declared(mut self, tag: (u16, u16), vr: &[u8; 2], len: u32, value: &[u8]) -> Self {
        let mut bytes = self.head(tag, vr);
        bytes.extend([0, 0]);
        bytes.extend(if self.big {
            len.to_be_bytes()
        } else {
            len.to_le_bytes()
        });
        bytes.extend(value);
        self.put.insert(tag, bytes);
        self
    }

    /// An element with a 32-bit length.
    pub fn long(self, tag: (u16, u16), vr: &[u8; 2], value: &[u8]) -> Self {
        self.long_declared(tag, vr, value.len() as u32, value)
    }

    /// Bytes as they are, where the tag they start with belongs: elements of an implicit
    /// VR data set, anything else.
    pub fn raw(mut self, bytes: &[u8]) -> Self {
        let pair = |at: usize| {
            let two = [bytes[at], bytes[at + 1]];
            if self.big {
                u16::from_be_bytes(two)
            } else {
                u16::from_le_bytes(two)
            }
        };
        let tag = (pair(0), pair(2));
        self.put.entry(tag).or_default().extend_from_slice(bytes);
        self
    }

    /// The UIDs an instance must have, those of `seed`, with `vr` written for Study Instance UID
    /// and in a 32-bit length if that is UN.
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

    /// Encapsulated pixel data in place of the file's: an empty offset table and one
    /// fragment.
    pub fn one_fragment(self) -> Self {
        let mut value = ITEM.to_vec();
        value.extend(0u32.to_le_bytes());
        value.extend(ITEM);
        value.extend(4u32.to_le_bytes());
        value.extend([0xFF, 0xD8, 0xFF, 0xD9]);
        value.extend(SEQUENCE_END);
        self.long_declared((0x7FE0, 0x0010), b"OB", UNDEFINED, &value)
    }

    /// The scanner's data set with what was put in place, in the file's encoding.
    fn data_set(&self) -> Vec<u8> {
        let syntax = if self.big {
            EXPLICIT_VR_BIG_ENDIAN.erased()
        } else if self.transfer_syntax == IMPLICIT_VR_LE {
            IMPLICIT_VR_LITTLE_ENDIAN.erased()
        } else {
            EXPLICIT_VR_LITTLE_ENDIAN.erased()
        };
        let mut bytes = Vec::new();
        let mut own = InMemDicomObject::new_empty();
        let flush = |own: &mut InMemDicomObject, bytes: &mut Vec<u8>| {
            std::mem::replace(own, InMemDicomObject::new_empty())
                .write_dataset_with_ts(&mut *bytes, &syntax)
                .expect("writing the scanner's elements to a Vec does not fail");
        };
        let mut put = self.put.iter().peekable();
        for element in files()[Scanned::Ct as usize].0.clone() {
            let tag = element.header().tag;
            while let Some((_, raw)) = put.next_if(|(at, _)| Tag(at.0, at.1) < tag) {
                flush(&mut own, &mut bytes);
                bytes.extend(raw);
            }
            if let Some((_, raw)) = put.next_if(|(at, _)| Tag(at.0, at.1) == tag) {
                flush(&mut own, &mut bytes);
                bytes.extend(raw);
            } else if !IDENTIFYING.contains(&tag) {
                own.put(element);
            }
        }
        flush(&mut own, &mut bytes);
        for (_, raw) in put {
            bytes.extend(raw);
        }
        bytes
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
        file.extend(self.data_set());
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

/// The CT file as GDCM leaves one it re-encoded from Implicit VR into JPEG Lossless SV1:
/// Referenced Study Sequence in UN with undefined length and implicit content.
pub fn un_sequence_file(seed: u32) -> Vec<u8> {
    AlteredFile::new(uids::JPEG_LOSSLESS_SV1)
        .identifiers(seed, b"UI")
        .long_declared(
            (0x0008, 0x1110),
            b"UN",
            UNDEFINED,
            &implicit_reference_sequence(&format!("2.25.{seed}4")),
        )
        .one_fragment()
        .bytes()
}

/// The same with Study Instance UID, a standard element, in UN with a defined length.
pub fn un_study_uid_file(seed: u32) -> Vec<u8> {
    AlteredFile::new(uids::JPEG_LOSSLESS_SV1)
        .identifiers(seed, b"UN")
        .one_fragment()
        .bytes()
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
        for (name, size) in [
            ("CT_small.dcm", 39206),
            ("MR_small.dcm", 9830),
            ("MR_small_jpeg_ls_lossless.dcm", 6124),
            ("emri_small.dcm", 84256),
            ("emri_small_jpeg_ls_lossless.dcm", 42834),
            ("JPEG-lossy.dcm", 9844),
            ("UN_sequence.dcm", 674),
        ] {
            assert_eq!(std::fs::metadata(path(name)).unwrap().len(), size, "{name}");
        }
    }

    #[test]
    fn the_file_of_ten_frames_and_its_jpeg_ls() {
        let series = RealSeries::new(11, 1);
        let native = Instance::from_bytes(&series.multiframe_bytes(0)).unwrap();
        assert_eq!(native.key.sop_instance_uid, series.instance_uid(0));
        assert_eq!(native.number_of_frames(), RealSeries::FRAMES);
        assert_eq!(native.key.transfer_syntax_uid, "1.2.840.10008.1.2.1");
        assert_eq!(RealSeries::frame_pixels(0).len(), 64 * 64);
        assert_ne!(RealSeries::frame_pixels(0), RealSeries::frame_pixels(9));
        let jls = Instance::from_bytes(&series.jpeg_ls_multiframe_bytes(0)).unwrap();
        assert_eq!(jls.key.transfer_syntax_uid, "1.2.840.10008.1.2.4.80");
        assert_eq!(RealSeries::jpeg_ls_frames().len(), 10);
        let one = Instance::from_bytes(&series.jpeg_ls_bytes(0)).unwrap();
        assert_eq!(one.key.transfer_syntax_uid, "1.2.840.10008.1.2.4.80");
        assert_eq!(one.string(tags::MANUFACTURER).unwrap(), "TOSHIBA_MEC");
        let lossy = Instance::from_bytes(&series.jpeg_lossy_bytes(0)).unwrap();
        assert_eq!(lossy.key.transfer_syntax_uid, "1.2.840.10008.1.2.4.51");
    }
}
