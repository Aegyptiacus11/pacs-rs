//! The Query SCP (PS3.4 C.4.1.3): a C-FIND on the Patient Root or the Study Root
//! Query/Retrieve Information Model (C.6.1, C.6.2), answered by the hierarchical search of
//! C.4.1.3.1.1 with one Pending response per match, and ended by a C-CANCEL-FIND-RQ with
//! status Cancel.
//!
//! - **What is read**: the Request Identifier's Query/Retrieve Level, the Unique Keys of
//!   the levels above it, and the keys of its own level that pacs-rs keeps ([`Level`]).
//!   Any other key is left out of the responses, which then say so with status FF01
//!   (C.2.2.1.3, Table C.4-1).
//! - **Matching** is the index's (`pacs_index::matching`, PS3.4 C.2.2.2): single value,
//!   list of UID, universal, wild card, range and sequence matching. Relational queries,
//!   empty value and multiple value matching are not negotiated.
//! - **Studies** come from a cursor over the index, so their number costs no memory; a
//!   study's series and a series' instances are read whole, then matched.
//! - **A patient** is the studies with its Patient ID that the caller sees: found by any
//!   of them, and described by the first one found. A study without a Patient ID belongs
//!   to no patient of the Patient Root model, and is found by the Study Root model.

use dicom_core::header::Header;
use dicom_core::value::{DataSetSequence, Value as DicomValue};
use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_encoding::text::SpecificCharacterSet;
use dicom_encoding::transfer_syntax::TransferSyntaxIndex;
use dicom_object::InMemDicomObject;
use dicom_transfer_syntax_registry::TransferSyntaxRegistry;
use dicom_ul::association::AsyncAssociation;
use pacs_index::matching::ItemKey;
use pacs_index::{Index, Key, Kind, Match, StudyCursor, StudySearch};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncRead, AsyncWrite};

use super::Request;
use super::operation::{After, Interruption, Refusal, interruption, refusal};
use crate::command::{self, Command, DATA_SET_PRESENT, tag};
use crate::message::{self, DataSet, MessageError};
use crate::query::{PATIENT_ROOT_FIND, STUDY_ROOT_FIND};
use crate::status::Status;

/// An attribute a level is searched on and answers with.
#[derive(Debug)]
struct Attribute {
    tag: Tag,
    /// The tag as DICOM JSON and the index write it.
    key: &'static str,
    vr: VR,
    kind: Kind,
    /// For a sequence, the attributes of its items that are keys.
    items: &'static [Attribute],
}

const fn attribute(tag: Tag, key: &'static str, vr: VR, kind: Kind) -> Attribute {
    Attribute {
        tag,
        key,
        vr,
        kind,
        items: &[],
    }
}

/// The keys of the Patient level of the Patient Root model (PS3.4 Table C.6-1) that pacs-rs
/// keeps.
const PATIENT: &[Attribute] = &[
    attribute(tags::PATIENT_NAME, "00100010", VR::PN, Kind::Name),
    attribute(tags::PATIENT_ID, "00100020", VR::LO, Kind::Text),
    attribute(tags::PATIENT_BIRTH_DATE, "00100030", VR::DA, Kind::Date),
    attribute(tags::PATIENT_SEX, "00100040", VR::CS, Kind::Text),
    attribute(
        tags::NUMBER_OF_PATIENT_RELATED_STUDIES,
        NUMBER_OF_PATIENT_RELATED_STUDIES,
        VR::IS,
        Kind::Number,
    ),
    attribute(
        tags::NUMBER_OF_PATIENT_RELATED_SERIES,
        NUMBER_OF_PATIENT_RELATED_SERIES,
        VR::IS,
        Kind::Number,
    ),
    attribute(
        tags::NUMBER_OF_PATIENT_RELATED_INSTANCES,
        NUMBER_OF_PATIENT_RELATED_INSTANCES,
        VR::IS,
        Kind::Number,
    ),
];

const PATIENT_ID: &str = "00100020";
const NUMBER_OF_PATIENT_RELATED_STUDIES: &str = "00201200";
const NUMBER_OF_PATIENT_RELATED_SERIES: &str = "00201202";
const NUMBER_OF_PATIENT_RELATED_INSTANCES: &str = "00201204";

/// What is counted of a patient, not kept with any of its studies.
const COUNTED_OF_A_PATIENT: [&str; 3] = [
    NUMBER_OF_PATIENT_RELATED_STUDIES,
    NUMBER_OF_PATIENT_RELATED_SERIES,
    NUMBER_OF_PATIENT_RELATED_INSTANCES,
];

/// The keys of the Study level of the Patient Root model (PS3.4 Table C.6-2) that pacs-rs
/// keeps: those of the Study Root model's, less what is the patient's.
const STUDY_OF_A_PATIENT: &[Attribute] = &[
    attribute(tags::STUDY_DATE, "00080020", VR::DA, Kind::Date),
    attribute(tags::STUDY_TIME, "00080030", VR::TM, Kind::Time),
    attribute(tags::ACCESSION_NUMBER, "00080050", VR::SH, Kind::Text),
    attribute(tags::MODALITIES_IN_STUDY, "00080061", VR::CS, Kind::Text),
    attribute(
        tags::REFERRING_PHYSICIAN_NAME,
        "00080090",
        VR::PN,
        Kind::Name,
    ),
    attribute(tags::STUDY_DESCRIPTION, "00081030", VR::LO, Kind::Text),
    attribute(tags::STUDY_INSTANCE_UID, "0020000D", VR::UI, Kind::Uid),
    attribute(tags::STUDY_ID, "00200010", VR::SH, Kind::Text),
    attribute(
        tags::NUMBER_OF_STUDY_RELATED_SERIES,
        "00201206",
        VR::IS,
        Kind::Number,
    ),
    attribute(
        tags::NUMBER_OF_STUDY_RELATED_INSTANCES,
        "00201208",
        VR::IS,
        Kind::Number,
    ),
];

/// The keys of the Study level of the Study Root model (PS3.4 Table C.6-5) that pacs-rs
/// keeps: the Unique and the Required ones, and of the Optional ones those the index has
/// or counts.
const STUDY: &[Attribute] = &[
    attribute(tags::STUDY_DATE, "00080020", VR::DA, Kind::Date),
    attribute(tags::STUDY_TIME, "00080030", VR::TM, Kind::Time),
    attribute(tags::ACCESSION_NUMBER, "00080050", VR::SH, Kind::Text),
    attribute(tags::MODALITIES_IN_STUDY, "00080061", VR::CS, Kind::Text),
    attribute(
        tags::REFERRING_PHYSICIAN_NAME,
        "00080090",
        VR::PN,
        Kind::Name,
    ),
    attribute(tags::STUDY_DESCRIPTION, "00081030", VR::LO, Kind::Text),
    attribute(tags::PATIENT_NAME, "00100010", VR::PN, Kind::Name),
    attribute(tags::PATIENT_ID, "00100020", VR::LO, Kind::Text),
    attribute(tags::PATIENT_BIRTH_DATE, "00100030", VR::DA, Kind::Date),
    attribute(tags::PATIENT_SEX, "00100040", VR::CS, Kind::Text),
    attribute(tags::STUDY_INSTANCE_UID, "0020000D", VR::UI, Kind::Uid),
    attribute(tags::STUDY_ID, "00200010", VR::SH, Kind::Text),
    attribute(
        tags::NUMBER_OF_STUDY_RELATED_SERIES,
        "00201206",
        VR::IS,
        Kind::Number,
    ),
    attribute(
        tags::NUMBER_OF_STUDY_RELATED_INSTANCES,
        "00201208",
        VR::IS,
        Kind::Number,
    ),
];

/// The keys of the Series level (PS3.4 Table C.6-3) that pacs-rs keeps, in both models.
const SERIES: &[Attribute] = &[
    attribute(tags::MODALITY, "00080060", VR::CS, Kind::Text),
    attribute(tags::SERIES_DESCRIPTION, "0008103E", VR::LO, Kind::Text),
    attribute(tags::SERIES_INSTANCE_UID, "0020000E", VR::UI, Kind::Uid),
    attribute(tags::SERIES_NUMBER, "00200011", VR::IS, Kind::Number),
    attribute(
        tags::NUMBER_OF_SERIES_RELATED_INSTANCES,
        "00201209",
        VR::IS,
        Kind::Number,
    ),
    attribute(
        tags::PERFORMED_PROCEDURE_STEP_START_DATE,
        "00400244",
        VR::DA,
        Kind::Date,
    ),
    attribute(
        tags::PERFORMED_PROCEDURE_STEP_START_TIME,
        "00400245",
        VR::TM,
        Kind::Time,
    ),
    Attribute {
        tag: tags::REQUEST_ATTRIBUTES_SEQUENCE,
        key: "00400275",
        vr: VR::SQ,
        kind: Kind::Sequence,
        items: &[
            attribute(
                tags::SCHEDULED_PROCEDURE_STEP_ID,
                "00400009",
                VR::SH,
                Kind::Text,
            ),
            attribute(tags::REQUESTED_PROCEDURE_ID, "00401001", VR::SH, Kind::Text),
        ],
    },
];

/// The keys of the Composite Object Instance level (PS3.4 Table C.6-4) that pacs-rs keeps,
/// in both models.
const IMAGE: &[Attribute] = &[
    attribute(tags::SOP_CLASS_UID, "00080016", VR::UI, Kind::Uid),
    attribute(tags::SOP_INSTANCE_UID, "00080018", VR::UI, Kind::Uid),
    attribute(tags::INSTANCE_NUMBER, "00200013", VR::IS, Kind::Number),
    attribute(tags::NUMBER_OF_FRAMES, "00280008", VR::IS, Kind::Number),
    attribute(tags::ROWS, "00280010", VR::US, Kind::Number),
    attribute(tags::COLUMNS, "00280011", VR::US, Kind::Number),
    attribute(tags::BITS_ALLOCATED, "00280100", VR::US, Kind::Number),
];

/// A key as the Conformance Statement lists it: its tag and VR, and for a sequence the keys
/// of its item.
pub(crate) type ListedKey = (Tag, VR, Vec<Tag>);

/// The keys pacs-rs matches and returns, by model and level: for the Conformance Statement.
pub(crate) fn keys() -> Vec<(&'static str, &'static str, Vec<ListedKey>)> {
    let list = |attributes: &[Attribute]| {
        attributes
            .iter()
            .map(|a| (a.tag, a.vr, a.items.iter().map(|i| i.tag).collect()))
            .collect()
    };
    vec![
        ("Patient Root", "PATIENT", list(PATIENT)),
        ("Patient Root", "STUDY", list(STUDY_OF_A_PATIENT)),
        ("Study Root", "STUDY", list(STUDY)),
        ("both", "SERIES", list(SERIES)),
        ("both", "IMAGE", list(IMAGE)),
    ]
}

/// A Query/Retrieve Information Model (PS3.4 C.6): what the hierarchy starts with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Model {
    /// Patient, study, series, instance (C.6.1).
    PatientRoot,
    /// Study, series, instance, the patient's attributes being the study's (C.6.2).
    StudyRoot,
}

impl Model {
    /// The model a FIND SOP class is of.
    fn of_find(sop_class: &str) -> Option<Model> {
        match sop_class {
            PATIENT_ROOT_FIND => Some(Model::PatientRoot),
            STUDY_ROOT_FIND => Some(Model::StudyRoot),
            _ => None,
        }
    }

    /// What Query/Retrieve Level may be in this model, for an error.
    pub(super) fn levels(self) -> &'static str {
        match self {
            Model::PatientRoot => {
                "Query/Retrieve Level is PATIENT, STUDY, SERIES or IMAGE in the Patient Root model"
            }
            Model::StudyRoot => {
                "Query/Retrieve Level is STUDY, SERIES or IMAGE in the Study Root model"
            }
        }
    }
}

/// A level of a Query/Retrieve Information Model (PS3.4 C.6.1.1, C.6.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Level {
    Patient,
    Study,
    Series,
    Image,
}

impl Level {
    /// By the value of Query/Retrieve Level (0008,0052) in `model`: the Study Root model
    /// has no PATIENT level.
    pub(super) fn named(model: Model, name: &str) -> Option<Level> {
        match (name, model) {
            ("PATIENT", Model::PatientRoot) => Some(Level::Patient),
            ("STUDY", _) => Some(Level::Study),
            ("SERIES", _) => Some(Level::Series),
            ("IMAGE", _) => Some(Level::Image),
            _ => None,
        }
    }

    pub(super) fn name(self) -> &'static str {
        match self {
            Level::Patient => "PATIENT",
            Level::Study => "STUDY",
            Level::Series => "SERIES",
            Level::Image => "IMAGE",
        }
    }

    fn attributes(self, model: Model) -> &'static [Attribute] {
        match (self, model) {
            (Level::Patient, _) => PATIENT,
            (Level::Study, Model::PatientRoot) => STUDY_OF_A_PATIENT,
            (Level::Study, Model::StudyRoot) => STUDY,
            (Level::Series, _) => SERIES,
            (Level::Image, _) => IMAGE,
        }
    }
}

/// The Patient ID that is the Unique Key of the level above (PS3.4 C.4.1.2.1, C.4.2.1.4.1):
/// one value, so no list, no wild card and not nothing. `None` when it is not that.
pub(super) fn one_patient_id(value: &str) -> Option<&str> {
    (!value.is_empty() && !value.contains(['\\', '*', '?'])).then_some(value)
}

/// An attribute a response carries.
#[derive(Debug)]
struct Returned {
    attribute: &'static Attribute,
    /// For a sequence matched by an item's keys: the attributes of the items that were
    /// asked for, which the matching items are returned with (C.2.2.2.6). `None` returns
    /// the value whole.
    of_items: Option<Vec<&'static Attribute>>,
    /// And what the items must answer to be among them.
    item_keys: Vec<ItemKey>,
}

/// A Request Identifier, read (PS3.4 C.4.1.1.3.1).
#[derive(Debug)]
struct Query {
    level: Level,
    /// The Unique Keys of the levels above, each a single value (C.4.1.2.1).
    patient: Option<String>,
    study: Option<String>,
    series: Option<String>,
    keys: Vec<Key>,
    returned: Vec<Returned>,
    /// A key pacs-rs does not have at this level was given: the responses are FF01.
    unsupported: bool,
}

/// The value of an element as the keys are written: values joined by the backslash, without
/// padding. `None` for a value that is not text.
fn text(element: &dicom_object::mem::InMemElement) -> Option<String> {
    let value = element.to_str().ok()?;
    Some(value.trim_matches([' ', '\0']).to_owned())
}

/// What a key's value asks of `attribute`; `None` when it asks nothing.
fn asked(attribute: &Attribute, value: &str) -> Result<Option<Match>, Refusal> {
    let bad = |reason: String| {
        refusal(
            Status::UNABLE_TO_PROCESS,
            format!("{}: {reason}", attribute.tag),
            Some(attribute.tag),
        )
    };
    // only a list of UID has several values without a negotiation (C.2.2.2.8)
    if attribute.kind != Kind::Uid && value.contains('\\') {
        return Err(bad(
            "one value only, multiple value matching is not negotiated".into(),
        ));
    }
    // of a name, the alphabetic group is what is matched
    let value = match attribute.kind {
        Kind::Name => value.split('=').next().unwrap_or(value),
        _ => value,
    };
    Match::parse(attribute.kind, value).map_err(bad)
}

impl Query {
    fn parse(identifier: &InMemDicomObject, model: Model) -> Result<Query, Refusal> {
        let mismatch = |comment: &str, tag: Tag| {
            refusal(
                Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                comment,
                Some(tag),
            )
        };
        let level = identifier
            .get(tags::QUERY_RETRIEVE_LEVEL)
            .and_then(text)
            .ok_or_else(|| {
                mismatch(
                    "the Identifier has no Query/Retrieve Level",
                    tags::QUERY_RETRIEVE_LEVEL,
                )
            })?;
        let level = Level::named(model, &level)
            .ok_or_else(|| mismatch(model.levels(), tags::QUERY_RETRIEVE_LEVEL))?;
        // PS3.4 C.4.1.2.1: a single value in the Unique Key of each level above
        let unique = |tag: Tag, name: &str| -> Result<String, Refusal> {
            let value = identifier.get(tag).and_then(text).unwrap_or_default();
            if value.is_empty() || !pacs_core::is_valid_uid(&value) {
                return Err(mismatch(
                    &format!("{} level needs one {name}", level.name()),
                    tag,
                ));
            }
            Ok(value)
        };
        let patient = match (model, level) {
            (Model::StudyRoot, _) | (_, Level::Patient) => None,
            _ => {
                let value = identifier
                    .get(tags::PATIENT_ID)
                    .and_then(text)
                    .unwrap_or_default();
                let one = one_patient_id(&value).ok_or_else(|| {
                    mismatch(
                        &format!("{} level needs one Patient ID", level.name()),
                        tags::PATIENT_ID,
                    )
                })?;
                Some(one.to_owned())
            }
        };
        let study = match level {
            Level::Patient | Level::Study => None,
            _ => Some(unique(tags::STUDY_INSTANCE_UID, "Study Instance UID")?),
        };
        let series = match level {
            Level::Image => Some(unique(tags::SERIES_INSTANCE_UID, "Series Instance UID")?),
            _ => None,
        };
        let mut query = Query {
            level,
            patient,
            study,
            series,
            keys: Vec::new(),
            returned: Vec::new(),
            unsupported: false,
        };
        for element in identifier {
            let tag = element.tag();
            let above = (tag == tags::PATIENT_ID && query.patient.is_some())
                || (tag == tags::STUDY_INSTANCE_UID && query.study.is_some())
                || (tag == tags::SERIES_INSTANCE_UID && query.series.is_some());
            // not keys: how the Identifier is written and read (C.4.1.1.3.1), group
            // lengths, and private attributes, which may be ignored
            if above
                || tag == tags::QUERY_RETRIEVE_LEVEL
                || tag == tags::SPECIFIC_CHARACTER_SET
                || tag == tags::TIMEZONE_OFFSET_FROM_UTC
                || tag.element() == 0
                || tag.group() % 2 == 1
            {
                continue;
            }
            let Some(attribute) = level.attributes(model).iter().find(|a| a.tag == tag) else {
                query.unsupported = true;
                continue;
            };
            let mut returned = Returned {
                attribute,
                of_items: None,
                item_keys: Vec::new(),
            };
            if attribute.kind == Kind::Sequence {
                if let Some((of_items, item_keys)) = query.item(attribute, element)? {
                    if !item_keys.is_empty() {
                        query.keys.push(Key {
                            tag: attribute.key,
                            kind: Kind::Sequence,
                            asked: Match::Item(item_keys.clone()),
                        });
                    }
                    returned.of_items = Some(of_items);
                    returned.item_keys = item_keys;
                }
            } else {
                let value = text(element).ok_or_else(|| {
                    refusal(
                        Status::UNABLE_TO_PROCESS,
                        format!("{tag}: the key's value cannot be read"),
                        Some(tag),
                    )
                })?;
                if let Some(asked) = asked(attribute, &value)? {
                    query.keys.push(Key {
                        tag: attribute.key,
                        kind: attribute.kind,
                        asked,
                    });
                }
            }
            query.returned.push(returned);
        }
        if let Some(key) = query.keys.iter().find(|key| key.asked.reversed()) {
            return Err(refusal(
                Status::UNABLE_TO_PROCESS,
                format!("{}: the range of times ends before it starts", key.tag),
                None,
            ));
        }
        Ok(query)
    }

    /// The one item of a sequence key (C.2.2.2.6): the attributes it asks to have back and
    /// the keys among them. `None` for no item, or an empty one: the sequence is returned
    /// whole, whatever it holds.
    #[allow(clippy::type_complexity)]
    fn item(
        &mut self,
        attribute: &'static Attribute,
        element: &dicom_object::mem::InMemElement,
    ) -> Result<Option<(Vec<&'static Attribute>, Vec<ItemKey>)>, Refusal> {
        let items = element.items().unwrap_or_default();
        if items.len() > 1 {
            return Err(refusal(
                Status::UNABLE_TO_PROCESS,
                format!("{}: a sequence key has a single Item", attribute.tag),
                Some(attribute.tag),
            ));
        }
        let Some(item) = items.first().filter(|item| item.iter().next().is_some()) else {
            return Ok(None);
        };
        let (mut of_items, mut keys) = (Vec::new(), Vec::new());
        for element in item {
            let Some(inner) = attribute.items.iter().find(|a| a.tag == element.tag()) else {
                self.unsupported = true;
                continue;
            };
            let value = text(element).unwrap_or_default();
            if let Some(asked) = asked(inner, &value)? {
                keys.push(ItemKey {
                    tag: inner.key.to_owned(),
                    kind: inner.kind,
                    asked,
                });
            }
            of_items.push(inner);
        }
        Ok(Some((of_items, keys)))
    }

    /// Whether a series or an instance, by its attributes, answers every key.
    fn answered_by(&self, entity: &Value) -> bool {
        self.keys
            .iter()
            .all(|key| key.asked.answered_by(key.kind, entity, key.tag))
    }

    /// The Response Identifier of one match (PS3.4 C.4.1.1.3.2): the keys asked for that
    /// pacs-rs has at this level, empty where the entity has no value; the Unique Keys of
    /// the levels above; Query/Retrieve Level; and Retrieve AE Title, the AE title that
    /// was called, since what is found here is retrieved from here.
    fn response(&self, entity: &Value, called: &str) -> InMemDicomObject {
        let mut identifier = InMemDicomObject::new_empty();
        let put = |identifier: &mut InMemDicomObject, tag: Tag, vr: VR, value: &str| {
            identifier.put(DataElement::new(tag, vr, PrimitiveValue::from(value)));
        };
        put(
            &mut identifier,
            tags::QUERY_RETRIEVE_LEVEL,
            VR::CS,
            self.level.name(),
        );
        put(&mut identifier, tags::RETRIEVE_AE_TITLE, VR::AE, called);
        if let Some(patient) = &self.patient {
            put(&mut identifier, tags::PATIENT_ID, VR::LO, patient);
        }
        if let Some(study) = &self.study {
            put(&mut identifier, tags::STUDY_INSTANCE_UID, VR::UI, study);
        }
        if let Some(series) = &self.series {
            put(&mut identifier, tags::SERIES_INSTANCE_UID, VR::UI, series);
        }
        for returned in &self.returned {
            identifier.put(returned.element(entity));
        }
        if !is_ascii(&identifier) {
            // the index keeps text as Unicode: UTF-8 whatever the files were written in
            put(
                &mut identifier,
                tags::SPECIFIC_CHARACTER_SET,
                VR::CS,
                "ISO_IR 192",
            );
        }
        identifier
    }
}

impl Returned {
    fn element(&self, entity: &Value) -> dicom_object::mem::InMemElement {
        let attribute = self.attribute;
        let stored = entity.get(attribute.key);
        if attribute.vr == VR::SQ {
            let items: Vec<InMemDicomObject> = stored
                .and_then(|stored| stored.get("Value"))
                .and_then(Value::as_array)
                .map(|items| self.items(items))
                .unwrap_or_default();
            return DataElement::new(
                attribute.tag,
                VR::SQ,
                DicomValue::Sequence(DataSetSequence::from(items)),
            );
        }
        let values = stored.map(values).unwrap_or_default();
        let value = match attribute.vr {
            VR::US => PrimitiveValue::U16(
                values
                    .iter()
                    .filter_map(|value| value.parse().ok())
                    .collect(),
            ),
            _ if values.is_empty() => PrimitiveValue::Empty,
            _ => PrimitiveValue::Strs(values.into_iter().collect()),
        };
        DataElement::new(attribute.tag, attribute.vr, value)
    }

    /// The items of a stored sequence to answer with: all of them, whole, for a key that
    /// asked nothing; else those that answer the item's keys, with the attributes asked for.
    fn items(&self, stored: &[Value]) -> Vec<InMemDicomObject> {
        let whole = |item: &Value| dicom_json::from_value::<InMemDicomObject>(item.clone()).ok();
        let Some(of_items) = &self.of_items else {
            return stored.iter().filter_map(whole).collect();
        };
        stored
            .iter()
            .filter(|item| {
                self.item_keys
                    .iter()
                    .all(|key| key.asked.answered_by(key.kind, item, &key.tag))
            })
            .map(|item| {
                let mut answer = InMemDicomObject::new_empty();
                for attribute in of_items {
                    let returned = Returned {
                        attribute,
                        of_items: None,
                        item_keys: Vec::new(),
                    };
                    answer.put(returned.element(item));
                }
                answer
            })
            .collect()
    }
}

/// Every value of an element in DICOM JSON, as the Identifier writes it: a person name
/// with all its component groups.
fn values(element: &Value) -> Vec<String> {
    let Some(values) = element.get("Value").and_then(Value::as_array) else {
        return Vec::new();
    };
    values
        .iter()
        .map(|value| match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Object(name) => {
                let group = |key| name.get(key).and_then(Value::as_str).unwrap_or("");
                let groups = [group("Alphabetic"), group("Ideographic"), group("Phonetic")];
                groups.join("=").trim_end_matches('=').to_owned()
            }
            // a missing value among several
            _ => String::new(),
        })
        .collect()
}

fn is_ascii(identifier: &InMemDicomObject) -> bool {
    identifier.iter().all(|element| match element.value() {
        DicomValue::Sequence(items) => items.items().iter().all(is_ascii),
        _ => element.to_str().map_or(true, |text| text.is_ascii()),
    })
}

/// A study as its level answers: what is stored, and what is counted.
fn study_entity(study: pacs_index::Study) -> Value {
    let mut entity = study.attributes;
    if let Some(map) = entity.as_object_mut() {
        if !study.modalities.is_empty() {
            map.insert(
                "00080061".into(),
                json!({"vr": "CS", "Value": study.modalities}),
            );
        }
        map.insert(
            "00201206".into(),
            json!({"vr": "IS", "Value": [study.series_count]}),
        );
        map.insert(
            "00201208".into(),
            json!({"vr": "IS", "Value": [study.instance_count]}),
        );
    }
    entity
}

/// The patients of a query, one at a time: a patient is found by any of its studies, and
/// given once.
struct Patients {
    studies: Box<StudyCursor>,
    index: Index,
    /// The Patient IDs already seen, given or not.
    seen: std::collections::HashSet<String>,
    /// Whether what is counted of a patient was asked for, to match or to have back.
    counted: bool,
    /// The keys on what is counted, which no study answers.
    of_counts: Vec<Key>,
}

impl Patients {
    async fn next(&mut self) -> Result<Option<Value>, pacs_index::Error> {
        while let Some(study) = self.studies.next().await? {
            let Some(patient_id) = pacs_index::first_value(&study.attributes, PATIENT_ID)
                .filter(|id| !id.trim().is_empty())
            else {
                // no Patient ID, no patient of this model
                continue;
            };
            if !self.seen.insert(patient_id.clone()) {
                continue;
            }
            let mut entity = match study.attributes {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            if self.counted {
                // every study of the patient that the caller sees
                let theirs = self
                    .index
                    .search_studies(&StudySearch {
                        keys: vec![Key {
                            tag: PATIENT_ID,
                            kind: Kind::Text,
                            asked: Match::Is(patient_id),
                        }],
                        ..StudySearch::default()
                    })
                    .await?;
                let counts = [
                    theirs.len() as i64,
                    theirs.iter().map(|study| study.series_count).sum(),
                    theirs.iter().map(|study| study.instance_count).sum(),
                ];
                for (tag, count) in COUNTED_OF_A_PATIENT.into_iter().zip(counts) {
                    entity.insert(tag.into(), json!({"vr": "IS", "Value": [count]}));
                }
            }
            let entity = Value::Object(entity);
            if self
                .of_counts
                .iter()
                .all(|key| key.asked.answered_by(key.kind, &entity, key.tag))
            {
                return Ok(Some(entity));
            }
        }
        Ok(None)
    }
}

/// The matches of a query, one at a time.
enum Matches {
    Patients(Patients),
    Studies(Box<StudyCursor>),
    Listed(std::vec::IntoIter<Value>),
}

impl Matches {
    /// Start the hierarchical search (C.4.1.3.1.1): the entities of the query's level,
    /// under the ones its Unique Keys name. A study the index does not show, one that is
    /// not the patient's, or a series that is not in the study, has none.
    async fn of(query: &Query, index: &Index) -> Result<Matches, pacs_index::Error> {
        let object = |attributes: Value| match attributes {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        // a range of dates and one of times are read apart without a negotiation
        // (PS3.4 C.2.2.2.5.4)
        let studies = |keys: Vec<Key>| {
            Box::new(index.study_cursor(StudySearch {
                keys,
                combined_date_time: false,
                ..StudySearch::default()
            }))
        };
        // of the patient named above, when one is: exactly that Patient ID
        let of_the_patient = query.patient.as_ref().map(|patient| Key {
            tag: PATIENT_ID,
            kind: Kind::Text,
            asked: Match::Is(patient.clone()),
        });
        if let (Some(study), Some(patient)) = (&query.study, &query.patient) {
            // a study under another patient than the one named is not under these keys
            let theirs = index.study(study).await?.is_some_and(|found| {
                pacs_index::first_value(&found.attributes, PATIENT_ID).as_deref()
                    == Some(patient.as_str())
            });
            if !theirs {
                return Ok(Matches::Listed(Vec::new().into_iter()));
            }
        }
        Ok(match (query.level, &query.study, &query.series) {
            (Level::Patient, ..) => {
                let (of_counts, of_studies): (Vec<Key>, Vec<Key>) = query
                    .keys
                    .iter()
                    .cloned()
                    .partition(|key| COUNTED_OF_A_PATIENT.contains(&key.tag));
                let counted = query
                    .returned
                    .iter()
                    .any(|returned| COUNTED_OF_A_PATIENT.contains(&returned.attribute.key));
                Matches::Patients(Patients {
                    studies: studies(of_studies),
                    index: index.clone(),
                    seen: std::collections::HashSet::new(),
                    counted,
                    of_counts,
                })
            }
            (Level::Series, Some(study), _) => {
                let series = index.series(study, None).await?;
                let entities = series.into_iter().map(|series| {
                    let mut entity = object(series.attributes);
                    entity.insert(
                        "00201209".into(),
                        json!({"vr": "IS", "Value": [series.instance_count]}),
                    );
                    Value::Object(entity)
                });
                Matches::listed(query, entities)
            }
            (Level::Image, Some(study), Some(series)) => {
                let instances = index.instances(study, series, None).await?;
                let entities = instances.into_iter().map(|instance| instance.attributes);
                Matches::listed(query, entities)
            }
            _ => {
                let mut keys = query.keys.clone();
                keys.extend(of_the_patient);
                Matches::Studies(studies(keys))
            }
        })
    }

    fn listed(query: &Query, entities: impl Iterator<Item = Value>) -> Matches {
        let matching: Vec<Value> = entities
            .filter(|entity| query.answered_by(entity))
            .collect();
        Matches::Listed(matching.into_iter())
    }

    async fn next(&mut self) -> Result<Option<Value>, pacs_index::Error> {
        match self {
            Matches::Patients(patients) => patients.next().await,
            Matches::Studies(cursor) => Ok(cursor.next().await?.map(study_entity)),
            Matches::Listed(entities) => Ok(entities.next()),
        }
    }
}

/// What a C-FIND came to, for whoever accounts for it.
#[derive(Debug)]
pub(super) struct Found {
    /// The final status.
    pub status: Status,
    /// Pending responses sent.
    pub matches: usize,
    /// The Request Identifier as it was received, when there was one.
    pub identifier: Vec<u8>,
}

/// Answer a C-FIND-RQ on `association`: every Pending response and the final one.
///
/// `authorized` is whether the caller may query at all; `index` reads what it may see. An
/// error is the association's: it cannot go on.
pub(super) async fn find<A, S>(
    association: &mut A,
    request: &Request<'_>,
    command: &Command,
    data: DataSet,
    index: &Index,
    authorized: bool,
) -> Result<After<Found>, MessageError>
where
    A: AsyncAssociation<S> + Send,
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let message_id = command.message_id()?;
    let mut found = Found {
        status: Status::SUCCESS,
        matches: 0,
        identifier: Vec::new(),
    };
    let prepared = prepare(request, data, index, authorized, &mut found).await;
    let (query, mut matches, ts) = match prepared {
        Ok(prepared) => prepared,
        Err(refusal) => {
            found.status = refusal.status;
            let mut response = command::response(command, refusal.status, Some(&refusal.comment));
            if let Some(tag) = refusal.offending {
                response = response.with_tags(tag::OFFENDING_ELEMENT, &[tag]);
            }
            message::send(association, request.context_id, &response, None).await?;
            return Ok(After::Open(found));
        }
    };
    let pending = if query.unsupported {
        Status::PENDING_OPTIONAL_KEYS
    } else {
        Status::PENDING
    };
    loop {
        match interruption(association, message_id, "C-FIND").await? {
            Interruption::Nothing => {}
            Interruption::Cancel => {
                found.status = Status::CANCEL;
                break;
            }
            Interruption::Released => {
                found.status = Status::CANCEL;
                return Ok(After::Released(found));
            }
            Interruption::Aborted => {
                found.status = Status::CANCEL;
                return Ok(After::Aborted(found));
            }
        }
        let entity = match matches.next().await {
            Ok(Some(entity)) => entity,
            Ok(None) => break,
            Err(e) => {
                tracing::error!(error = %e, "C-FIND: the index failed");
                found.status = Status::UNABLE_TO_PROCESS_INDEX;
                let response = command::response(
                    command,
                    found.status,
                    Some("the index could not be searched"),
                );
                message::send(association, request.context_id, &response, None).await?;
                return Ok(After::Open(found));
            }
        };
        let mut identifier = Vec::new();
        query
            .response(&entity, request.called)
            .write_dataset_with_ts_cs(&mut identifier, ts, SpecificCharacterSet::ISO_IR_192)
            .map_err(|e| {
                MessageError::Protocol(format!("a Response Identifier cannot be written: {e}"))
            })?;
        let response = command::response(command, pending, None)
            .with_us(tag::COMMAND_DATA_SET_TYPE, DATA_SET_PRESENT);
        message::send(
            association,
            request.context_id,
            &response,
            Some(&identifier),
        )
        .await?;
        found.matches += 1;
    }
    // Success: a response has been sent for each match; or Cancel. Neither has an
    // Identifier (PS3.4 C.4.1.1.3.2).
    let response = command::response(command, found.status, None);
    message::send(association, request.context_id, &response, None).await?;
    Ok(After::Open(found))
}

/// Everything a C-FIND needs before its first match, or why it has none.
async fn prepare(
    request: &Request<'_>,
    data: DataSet,
    index: &Index,
    authorized: bool,
    found: &mut Found,
) -> Result<(Query, Matches, &'static dicom_encoding::TransferSyntax), Refusal> {
    let bytes = match data {
        DataSet::Received(bytes) => bytes,
        DataSet::None => {
            return Err(refusal(
                Status::IDENTIFIER_DOES_NOT_MATCH_SOP_CLASS,
                "C-FIND-RQ without an Identifier",
                None,
            ));
        }
        // only a C-STORE's data set is spooled; an Identifier that large is too large
        DataSet::Spooled(spooled) => {
            return Err(refusal(
                Status::FIND_OUT_OF_RESOURCES,
                format!("an Identifier of {} bytes is over the limit", spooled.size),
                None,
            ));
        }
        DataSet::Dropped { size } => {
            return Err(refusal(
                Status::FIND_OUT_OF_RESOURCES,
                format!("an Identifier of {size} bytes is over the limit"),
                None,
            ));
        }
    };
    found.identifier = bytes;
    if !authorized {
        // PS3.7 9.1.2.1.6: the peer was not authorized to invoke the operation
        return Err(refusal(
            Status::NOT_AUTHORIZED,
            format!(
                "{} is not an AE this server answers queries of",
                request.calling
            ),
            None,
        ));
    }
    let unreadable = |reason: String| {
        refusal(
            Status::UNABLE_TO_PROCESS,
            format!("the Identifier cannot be read: {reason}"),
            None,
        )
    };
    let ts = TransferSyntaxRegistry
        .get(request.transfer_syntax)
        .ok_or_else(|| unreadable(format!("transfer syntax {}", request.transfer_syntax)))?;
    let identifier = crate::scu::read_identifier(&found.identifier, ts).map_err(unreadable)?;
    let model = Model::of_find(request.abstract_syntax).ok_or_else(|| {
        refusal(
            Status::SOP_CLASS_NOT_SUPPORTED,
            format!("SOP Class {} has no C-FIND", request.abstract_syntax),
            None,
        )
    })?;
    let query = Query::parse(&identifier, model)?;
    let matches = Matches::of(&query, index).await.map_err(|e| {
        tracing::error!(error = %e, "C-FIND: the index failed");
        refusal(
            Status::UNABLE_TO_PROCESS_INDEX,
            "the index could not be searched",
            None,
        )
    })?;
    Ok((query, matches, ts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_attribute_is_written_as_its_tag() {
        fn check(attributes: &[Attribute]) {
            for attribute in attributes {
                let written = format!(
                    "{:04X}{:04X}",
                    attribute.tag.group(),
                    attribute.tag.element()
                );
                assert_eq!(attribute.key, written);
                assert_eq!(attribute.vr == VR::SQ, attribute.kind == Kind::Sequence);
                assert_eq!(attribute.items.is_empty(), attribute.vr != VR::SQ);
                check(attribute.items);
            }
        }
        for model in [Model::PatientRoot, Model::StudyRoot] {
            for level in [Level::Study, Level::Series, Level::Image] {
                check(level.attributes(model));
                assert_eq!(Level::named(model, level.name()), Some(level));
            }
        }
        check(Level::Patient.attributes(Model::PatientRoot));
        // the patient is a level of one model, and its attributes the study's in the other
        assert_eq!(
            Level::named(Model::PatientRoot, "PATIENT"),
            Some(Level::Patient)
        );
        assert_eq!(Level::named(Model::StudyRoot, "PATIENT"), None);
        let has = |model, tag: Tag| {
            Level::Study
                .attributes(model)
                .iter()
                .any(|attribute| attribute.tag == tag)
        };
        assert!(has(Model::StudyRoot, tags::PATIENT_NAME));
        assert!(!has(Model::PatientRoot, tags::PATIENT_NAME));
        assert!(!has(Model::PatientRoot, tags::PATIENT_ID));
        // one Patient ID, as the Unique Key of a level above
        assert_eq!(one_patient_id("P-1"), Some("P-1"));
        for not_one in ["", "P-1\\P-2", "P-*", "P-?"] {
            assert_eq!(one_patient_id(not_one), None, "{not_one}");
        }
    }
}
