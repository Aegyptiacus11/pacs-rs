//! Main tags: the attributes the REST API shows for each resource, as raw DICOM strings.
//!
//! The lists are those of the reference server's default configuration, read from its
//! `GET /system` (`MainDicomTags`) for version 26.6.1, so that pacs-rs answers with the
//! same attributes.

use dicom_core::Tag;
use dicom_core::dictionary::{DataDictionary, DataDictionaryEntry};
use dicom_dictionary_std::StandardDataDictionary;
use serde_json::{Map, Value};

use crate::Instance;

impl Instance {
    /// Those of `tags` the data set has, by keyword, as their raw string values (several
    /// values joined by `\\`), padding trimmed. Sequences are left out.
    pub fn main_tags(&self, tags: &[Tag]) -> Map<String, Value> {
        let mut map = Map::new();
        for &tag in tags {
            let Some(element) = self.object.get(tag) else {
                continue;
            };
            let Ok(value) = element.to_str() else {
                continue;
            };
            let name = StandardDataDictionary
                .by_tag(tag)
                .map(|e| e.alias().to_owned())
                .unwrap_or_else(|| format!("{:04x},{:04x}", tag.group(), tag.element()));
            map.insert(
                name,
                Value::String(value.trim_matches([' ', '\0']).to_owned()),
            );
        }
        map
    }
}

pub const PATIENT: &[Tag] = &[
    Tag(0x0010, 0x0010),
    Tag(0x0010, 0x0020),
    Tag(0x0010, 0x0030),
    Tag(0x0010, 0x0040),
    Tag(0x0010, 0x1000),
];

pub const STUDY: &[Tag] = &[
    Tag(0x0008, 0x0020),
    Tag(0x0008, 0x0030),
    Tag(0x0008, 0x0050),
    Tag(0x0008, 0x0080),
    Tag(0x0008, 0x0090),
    Tag(0x0008, 0x0201),
    Tag(0x0008, 0x1030),
    Tag(0x0020, 0x000D),
    Tag(0x0020, 0x0010),
    Tag(0x0032, 0x1032),
    Tag(0x0032, 0x1060),
];

pub const SERIES: &[Tag] = &[
    Tag(0x0008, 0x0021),
    Tag(0x0008, 0x0031),
    Tag(0x0008, 0x0060),
    Tag(0x0008, 0x0070),
    Tag(0x0008, 0x0201),
    Tag(0x0008, 0x1010),
    Tag(0x0008, 0x103E),
    Tag(0x0008, 0x1070),
    Tag(0x0018, 0x0010),
    Tag(0x0018, 0x0015),
    Tag(0x0018, 0x0024),
    Tag(0x0018, 0x1030),
    Tag(0x0018, 0x1090),
    Tag(0x0018, 0x1400),
    Tag(0x0020, 0x000E),
    Tag(0x0020, 0x0011),
    Tag(0x0020, 0x0037),
    Tag(0x0020, 0x0105),
    Tag(0x0020, 0x1002),
    Tag(0x0040, 0x0244),
    Tag(0x0040, 0x0245),
    Tag(0x0040, 0x0254),
    Tag(0x0040, 0x0275),
    Tag(0x0054, 0x0081),
    Tag(0x0054, 0x0101),
    Tag(0x0054, 0x1000),
];

pub const INSTANCE: &[Tag] = &[
    Tag(0x0008, 0x0012),
    Tag(0x0008, 0x0013),
    Tag(0x0008, 0x0018),
    Tag(0x0020, 0x0012),
    Tag(0x0020, 0x0013),
    Tag(0x0020, 0x0032),
    Tag(0x0020, 0x0037),
    Tag(0x0020, 0x0100),
    Tag(0x0020, 0x4000),
    Tag(0x0028, 0x0008),
    Tag(0x0054, 0x1330),
];
