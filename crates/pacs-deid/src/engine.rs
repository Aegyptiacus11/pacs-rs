//! The de-identifier of PS3.15 E.1.1: every attribute of Table E.1-1 is protected or kept
//! as the Profile and the project's Options and overrides say, at the top level and inside
//! sequences; private attributes and group 0004 are removed; Patient Identity Removed and
//! the De-identification Method are added.

use std::collections::HashMap;
use std::sync::OnceLock;

use dicom_core::value::{DataSetSequence, PrimitiveValue, Value};
use dicom_core::{DataElement, Length, Tag, VR};
use dicom_dictionary_std::tags;
use dicom_object::InMemDicomObject;
use dicom_object::mem::InMemElement;

use crate::action::Action;
use crate::clean;
use crate::dates;
use crate::keys::Keys;
use crate::profile::{Opt, Profile};
use crate::quarantine;
use crate::table::{ROWS, Row, Tags};

/// What became of an instance.
#[derive(Debug)]
pub enum Outcome {
    Deidentified(Box<InMemDicomObject>),
    /// Kept out of the archive, for the reason given.
    Quarantined(String),
}

/// Table E.1-1 by tag, and its repeating-group rows apart.
type Index = (HashMap<(u16, u16), &'static Row>, Vec<&'static Row>);

fn table() -> &'static Index {
    static TABLE: OnceLock<Index> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut one = HashMap::new();
        let mut repeating = Vec::new();
        for row in ROWS {
            match row.tags {
                Tags::One(g, e) => {
                    one.insert((g, e), row);
                }
                Tags::Repeating(..) => repeating.push(row),
            }
        }
        (one, repeating)
    })
}

fn row(group: u16, element: u16) -> Option<&'static Row> {
    let (one, repeating) = table();
    one.get(&(group, element)).copied().or_else(|| {
        repeating.iter().copied().find(|row| match row.tags {
            // the repeating groups are the even ones, xx00 to xx1E (PS3.5 7.6)
            Tags::Repeating(high, e) => {
                (group >> 8) as u8 == high
                    && group & 0xFF <= 0x1E
                    && group.is_multiple_of(2)
                    && e.is_none_or(|e| e == element)
            }
            Tags::One(..) => false,
        })
    })
}

/// The attribute's name in Table E.1-1 and the action `profile` takes on it, overrides
/// included; `None` for an attribute the table does not list.
pub fn explain(profile: &Profile, group: u16, element: u16) -> Option<(&'static str, Action)> {
    let row = row(group, element)?;
    let ctx = Context {
        profile,
        keys: &Keys::new([0u8; 32]).expect("32 bytes"),
        shift: profile
            .has(Opt::RetainLongitudinalModifiedDates)
            .then_some(-1),
        pseudonym: String::new(),
        identifiers: Vec::new(),
    };
    Some((row.name, action(Tag(group, element), VR::UN, &ctx)))
}

struct Context<'a> {
    profile: &'a Profile,
    keys: &'a Keys,
    /// The patient's date shift, with the Modified Dates Option.
    shift: Option<i32>,
    pseudonym: String,
    identifiers: Vec<String>,
}

/// De-identify `original` by `profile`, with the replacement values `keys` give; or say why
/// it goes to quarantine. The File Meta Information is the caller's to write anew (E.1.1).
pub fn deidentify(original: &InMemDicomObject, profile: &Profile, keys: &Keys) -> Outcome {
    if let Some(reason) = quarantine::reason(original, &profile.quarantine) {
        return Outcome::Quarantined(reason);
    }
    let first = |tag| {
        original
            .element(tag)
            .ok()
            .and_then(|e| e.to_str().ok())
            .map(|s| s.trim_matches([' ', '\0']).to_owned())
            .filter(|s| !s.is_empty())
    };
    // one patient, one pseudonym and one date shift: Patient ID with its issuer, or the study
    // when there is no Patient ID
    let patient = match first(tags::PATIENT_ID) {
        Some(id) => format!(
            "{id}\\{}",
            first(tags::ISSUER_OF_PATIENT_ID).unwrap_or_default()
        ),
        None => format!(
            "study:{}",
            first(tags::STUDY_INSTANCE_UID).unwrap_or_default()
        ),
    };
    let ctx = Context {
        profile,
        keys,
        shift: profile
            .has(Opt::RetainLongitudinalModifiedDates)
            .then(|| keys.date_shift(&patient, profile.date_shift_days)),
        pseudonym: keys.pseudonym(&patient, &profile.pseudonym_prefix),
        identifiers: clean::identifiers(original, &profile.extra_identifiers),
    };
    let mut object = walk(original, &ctx);
    add_method(&mut object, profile);
    if profile.has(Opt::RetainPatientCharacteristics) && object.element(tags::PATIENT_AGE).is_err()
    {
        // E.1.1, note: Patient's Age may replace Patient's Birth Date
        let on = first(tags::STUDY_DATE).or_else(|| first(tags::ACQUISITION_DATE));
        if let (Some(birth), Some(on)) = (first(tags::PATIENT_BIRTH_DATE), on)
            && let Some(age) = dates::age_years(&birth, &on).filter(|a| *a <= 999)
        {
            object.put(DataElement::new(
                tags::PATIENT_AGE,
                VR::AS,
                PrimitiveValue::from(format!("{age:03}Y")),
            ));
        }
    }
    Outcome::Deidentified(Box::new(object))
}

/// The attributes this de-identifier writes itself, replaced wherever they were.
const WRITTEN: [Tag; 4] = [
    tags::PATIENT_IDENTITY_REMOVED,
    tags::DEIDENTIFICATION_METHOD,
    tags::DEIDENTIFICATION_METHOD_CODE_SEQUENCE,
    tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED,
];

fn walk(object: &InMemDicomObject, ctx: &Context) -> InMemDicomObject {
    let mut out = InMemDicomObject::new_empty();
    for element in object.iter() {
        let tag = element.header().tag;
        if tag.group() == 0x0002 || tag.group() == 0x0004 || WRITTEN.contains(&tag) {
            continue;
        }
        if let Some(element) = apply(element, action(tag, element.vr(), ctx), ctx) {
            out.put(element);
        }
    }
    out
}

/// The action for an attribute: the first override that matches; else, for a private
/// attribute, X (Retain Safe Private is not implemented); else Table E.1-1, an Option's
/// action over the Profile's (K over C when two Options differ); else, with the Modified
/// Dates Option, C for every other DA and DT (E.1.1: dates may be handled by their VR); else
/// K.
fn action(tag: Tag, vr: VR, ctx: &Context) -> Action {
    let remove = ctx.profile.remove_when_allowed;
    if let Some(o) = ctx
        .profile
        .overrides
        .iter()
        .find(|o| o.tag.matches(tag.group(), tag.element()))
    {
        return o.action.resolve(remove);
    }
    if tag.group() % 2 == 1 {
        return Action::X;
    }
    if let Some(row) = row(tag.group(), tag.element()) {
        let chosen: Vec<Action> = ctx
            .profile
            .options
            .iter()
            .filter_map(|opt| row.options[opt.column()])
            .collect();
        let action = if chosen.contains(&Action::K) {
            Action::K
        } else if chosen.contains(&Action::C) {
            Action::C
        } else {
            row.basic
        };
        return action.resolve(remove);
    }
    if ctx.shift.is_some() && matches!(vr, VR::DA | VR::DT) {
        return Action::C;
    }
    Action::K
}

fn strings(element: &InMemElement) -> Vec<String> {
    element
        .to_multi_str()
        .map(|v| v.to_vec())
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim_matches([' ', '\0']).to_owned())
        .collect()
}

fn with_strings(tag: Tag, vr: VR, values: Vec<String>) -> InMemElement {
    DataElement::new(tag, vr, PrimitiveValue::Strs(values.into()))
}

fn empty(tag: Tag, vr: VR) -> InMemElement {
    if vr == VR::SQ {
        DataElement::new(
            tag,
            vr,
            Value::Sequence(DataSetSequence::new(
                Vec::<InMemDicomObject>::new(),
                Length::UNDEFINED,
            )),
        )
    } else {
        DataElement::new(tag, vr, PrimitiveValue::Empty)
    }
}

/// The sequence with the profile applied to each item.
fn items(element: &InMemElement, ctx: &Context) -> Option<InMemElement> {
    let items = element.items()?;
    let items: Vec<InMemDicomObject> = items.iter().map(|item| walk(item, ctx)).collect();
    Some(DataElement::new(
        element.header().tag,
        VR::SQ,
        Value::Sequence(DataSetSequence::new(items, Length::UNDEFINED)),
    ))
}

fn apply(element: &InMemElement, action: Action, ctx: &Context) -> Option<InMemElement> {
    let (tag, vr) = (element.header().tag, element.vr());
    let is_sequence = element.items().is_some();
    match action {
        Action::X => None,
        Action::Z if tag == tags::PATIENT_NAME || tag == tags::PATIENT_ID => {
            Some(with_strings(tag, vr, vec![ctx.pseudonym.clone()]))
        }
        Action::Z => Some(empty(tag, vr)),
        Action::K if is_sequence => items(element, ctx),
        Action::K => Some(element.clone()),
        // a sequence's dummy is its items with the profile applied
        Action::D | Action::U | Action::C if is_sequence => items(element, ctx),
        Action::U if vr == VR::UI => Some(with_strings(
            tag,
            vr,
            strings(element)
                .iter()
                .filter(|u| !u.is_empty())
                .map(|u| ctx.keys.uid(u))
                .collect(),
        )),
        Action::D | Action::U => {
            if tag == tags::PATIENT_NAME || tag == tags::PATIENT_ID {
                return Some(with_strings(tag, vr, vec![ctx.pseudonym.clone()]));
            }
            Some(dummy(element, ctx))
        }
        Action::C => clean(element, ctx),
        Action::ZD | Action::XZ | Action::XD | Action::XZD | Action::XZU => {
            unreachable!("compound actions are resolved first")
        }
    }
}

/// A non-zero length value consistent with the VR (Table E.1-1a, D).
fn dummy(element: &InMemElement, ctx: &Context) -> InMemElement {
    let (tag, vr) = (element.header().tag, element.vr());
    let text = |s: &str| with_strings(tag, vr, vec![s.to_owned()]);
    match vr {
        VR::PN | VR::LO | VR::SH | VR::LT | VR::ST | VR::UT | VR::UC | VR::AE | VR::CS => {
            text("ANONYMIZED")
        }
        VR::DA => text("19000101"),
        VR::TM => text("000000"),
        VR::DT => text("19000101000000"),
        VR::AS => text("000Y"),
        VR::IS | VR::DS => text("0"),
        VR::UI => text(&ctx.keys.uid("dummy")),
        VR::US => DataElement::new(tag, vr, PrimitiveValue::from(0_u16)),
        VR::SS => DataElement::new(tag, vr, PrimitiveValue::from(0_i16)),
        VR::UL => DataElement::new(tag, vr, PrimitiveValue::from(0_u32)),
        VR::SL => DataElement::new(tag, vr, PrimitiveValue::from(0_i32)),
        VR::FL => DataElement::new(tag, vr, PrimitiveValue::from(0_f32)),
        VR::FD => DataElement::new(tag, vr, PrimitiveValue::from(0_f64)),
        _ => empty(tag, vr),
    }
}

/// Table E.1-1a, C: dates moved by the patient's shift (Modified Dates); times kept, as a
/// whole-day shift does not move them; text kept unless it holds an identifier (Clean
/// Descriptors and the cleaned retentions); anything else kept as is.
fn clean(element: &InMemElement, ctx: &Context) -> Option<InMemElement> {
    let (tag, vr) = (element.header().tag, element.vr());
    match vr {
        VR::DA | VR::DT => {
            let Some(shift) = ctx.shift else {
                return Some(empty(tag, vr));
            };
            let shifted: Option<Vec<String>> = strings(element)
                .iter()
                .filter(|v| !v.is_empty())
                .map(|v| {
                    if vr == VR::DA {
                        dates::shift_da(v, shift)
                    } else {
                        dates::shift_dt(v, shift)
                    }
                })
                .collect();
            Some(match shifted {
                Some(values) => with_strings(tag, vr, values),
                None => empty(tag, vr),
            })
        }
        VR::PN | VR::LO | VR::SH | VR::LT | VR::ST | VR::UT | VR::UC | VR::AE => {
            let kept: Option<Vec<String>> = strings(element)
                .iter()
                .map(|v| clean::clean_text(v, &ctx.identifiers))
                .collect();
            Some(match kept {
                Some(values) => with_strings(tag, vr, values),
                None => empty(tag, vr),
            })
        }
        _ => Some(element.clone()),
    }
}

/// Patient Identity Removed YES, the De-identification Method Code Sequence with the Basic
/// Profile and each Option (CID 7050), the De-identification Method naming the profile and
/// every override, and Longitudinal Temporal Information Modified (E.1.1, E.3.6).
fn add_method(object: &mut InMemDicomObject, profile: &Profile) {
    let cs = |tag, value: &str| DataElement::new(tag, VR::CS, PrimitiveValue::from(value));
    object.put(cs(tags::PATIENT_IDENTITY_REMOVED, "YES"));
    let code = |value: &str, meaning: &str| {
        InMemDicomObject::from_element_iter([
            DataElement::new(tags::CODE_VALUE, VR::SH, PrimitiveValue::from(value)),
            DataElement::new(
                tags::CODING_SCHEME_DESIGNATOR,
                VR::SH,
                PrimitiveValue::from("DCM"),
            ),
            DataElement::new(tags::CODE_MEANING, VR::LO, PrimitiveValue::from(meaning)),
        ])
    };
    let mut codes = vec![code("113100", "Basic Application Confidentiality Profile")];
    codes.extend(profile.options.iter().map(|o| {
        let (value, meaning) = o.code();
        code(value, meaning)
    }));
    object.put(DataElement::new(
        tags::DEIDENTIFICATION_METHOD_CODE_SEQUENCE,
        VR::SQ,
        Value::Sequence(DataSetSequence::new(codes, Length::UNDEFINED)),
    ));
    // LO: 64 characters a value
    let mut method = vec![format!("pacs-rs profile {}", profile.name)];
    for o in &profile.overrides {
        let mut text = format!("override {} {:?}", o.tag, o.action);
        if let Some(reason) = &o.reason {
            text.push_str(": ");
            text.push_str(reason);
        }
        method.push(text);
    }
    let method: Vec<String> = method
        .into_iter()
        .map(|m| m.chars().filter(|c| *c != '\\').take(64).collect())
        .collect();
    object.put(with_strings(tags::DEIDENTIFICATION_METHOD, VR::LO, method));
    let temporal = if profile.has(Opt::RetainLongitudinalModifiedDates) {
        "MODIFIED"
    } else if profile.has(Opt::RetainLongitudinalFullDates) {
        "UNMODIFIED"
    } else {
        "REMOVED"
    };
    object.put(cs(
        tags::LONGITUDINAL_TEMPORAL_INFORMATION_MODIFIED,
        temporal,
    ));
}
