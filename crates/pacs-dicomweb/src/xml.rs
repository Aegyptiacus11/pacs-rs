//! The Native DICOM Model (PS3.19 Annex A), the XML of `application/dicom+xml`, read as the
//! DICOM JSON of the same data set (PS3.18 Annex F): what STOW-RS of XML metadata stores
//! through the same path as JSON metadata.
//!
//! `DicomAttribute` elements with their `tag` and `vr`; values in `Value`, person names in
//! `PersonName` (its `Alphabetic`, `Ideographic` and `Phonetic` groups of name parts), items
//! of a sequence in `Item`, binary values in `InlineBinary` or named by a `BulkData` `uri`.

use quick_xml::events::Event;
use serde_json::{Map, Value};

/// An element of the XML, as read: its name, attributes, text and children.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attributes: Vec<(String, String)>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The children named `name`, by their `number` (from 1), as PS3.19 numbers values.
    fn numbered<'a>(&'a self, name: &'a str) -> Vec<&'a Node> {
        let mut nodes: Vec<&Node> = self.children_named(name).collect();
        nodes.sort_by_key(|n| {
            n.attribute("number")
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
        });
        nodes
    }
}

fn parse(xml: &[u8]) -> Result<Node, String> {
    // Text is kept as written: trimmed, the spaces around an entity (`a &amp; b`) would
    // go. The whitespace of the layout falls in elements whose text is not read.
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut buffer = Vec::new();
    let error = |e: &dyn std::fmt::Display| format!("not well-formed XML: {e}");
    loop {
        let event = reader.read_event_into(&mut buffer).map_err(|e| error(&e))?;
        match event {
            Event::Start(ref start) | Event::Empty(ref start) => {
                let mut node = Node {
                    name: start.local_name().as_ref().to_owned(),
                    ..Node::default()
                };
                for attribute in start.attributes() {
                    let attribute = attribute.map_err(|e| error(&e))?;
                    node.attributes.push((
                        attribute.key.local_name().as_ref().to_owned(),
                        attribute
                            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                            .map_err(|e| error(&e))?
                            .into_owned(),
                    ));
                }
                let empty = matches!(event, Event::Empty(_));
                stack.push(node);
                if empty {
                    let node = stack.pop().expect("pushed");
                    stack.last_mut().expect("root").children.push(node);
                }
            }
            Event::End(_) => {
                let node = stack.pop().expect("an end has a start");
                let parent = stack
                    .last_mut()
                    .ok_or_else(|| "an end tag without its start".to_owned())?;
                parent.children.push(node);
            }
            Event::Text(text) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&text.xml10_content());
                }
            }
            Event::CData(data) => {
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&data.xml10_content());
                }
            }
            Event::GeneralRef(reference) => {
                let resolved = if reference.is_char_ref() {
                    reference
                        .resolve_char_ref()
                        .map_err(|e| error(&e))?
                        .map(String::from)
                } else {
                    match reference.xml10_content().as_ref() {
                        "amp" => Some("&".into()),
                        "lt" => Some("<".into()),
                        "gt" => Some(">".into()),
                        "quot" => Some("\"".into()),
                        "apos" => Some("'".into()),
                        _ => None,
                    }
                };
                let resolved =
                    resolved.ok_or_else(|| "an entity XML does not define".to_owned())?;
                if let Some(node) = stack.last_mut() {
                    node.text.push_str(&resolved);
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let mut root = stack
        .pop()
        .filter(|_| stack.is_empty())
        .ok_or_else(|| "unclosed elements".to_owned())?;
    root.children
        .pop()
        .filter(|_| root.children.is_empty())
        .ok_or_else(|| "not one document element".to_owned())
}

/// The VRs whose values are numbers in DICOM JSON (PS3.18 F.2.3).
fn is_number(vr: &str) -> bool {
    matches!(
        vr,
        "IS" | "DS" | "SL" | "SS" | "SV" | "UL" | "US" | "UV" | "FL" | "FD"
    )
}

fn number(text: &str) -> Result<Value, String> {
    let text = text.trim();
    if let Ok(n) = text.parse::<i64>() {
        return Ok(Value::from(n));
    }
    text.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .ok_or_else(|| format!("{text:?} is not a number"))
}

/// A `PersonName` as the string of one group: its parts joined by `^`, empty trailing ones
/// left out (PS3.5 6.2, PN).
fn person_name_group(group: &Node) -> String {
    let parts = [
        "FamilyName",
        "GivenName",
        "MiddleName",
        "NamePrefix",
        "NameSuffix",
    ]
    .map(|part| {
        group
            .children_named(part)
            .next()
            .map(|n| n.text.clone())
            .unwrap_or_default()
    });
    let mut joined = parts.join("^");
    while joined.ends_with('^') {
        joined.pop();
    }
    joined
}

/// A `NativeDicomModel`, or an `Item`, as a DICOM JSON object.
fn data_set(node: &Node) -> Result<Value, String> {
    let mut object = Map::new();
    for attribute in node.children_named("DicomAttribute") {
        let tag = attribute
            .attribute("tag")
            .filter(|t| t.len() == 8 && t.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| "a DicomAttribute without a tag of eight hexadecimal digits".to_owned())?
            .to_ascii_uppercase();
        let vr = attribute
            .attribute("vr")
            .ok_or_else(|| format!("DicomAttribute {tag} without its vr"))?;
        let mut element = Map::new();
        element.insert("vr".into(), Value::String(vr.to_owned()));
        if let Some(bulk) = attribute.children_named("BulkData").next() {
            let uri = bulk
                .attribute("uri")
                .ok_or_else(|| format!("BulkData of {tag} without its uri"))?;
            element.insert("BulkDataURI".into(), Value::String(uri.to_owned()));
        } else if let Some(inline) = attribute.children_named("InlineBinary").next() {
            element.insert(
                "InlineBinary".into(),
                Value::String(inline.text.trim().to_owned()),
            );
        } else if vr == "SQ" {
            let items = attribute
                .numbered("Item")
                .into_iter()
                .map(data_set)
                .collect::<Result<Vec<_>, _>>()?;
            if !items.is_empty() {
                element.insert("Value".into(), Value::Array(items));
            }
        } else if vr == "PN" {
            let names: Vec<Value> = attribute
                .numbered("PersonName")
                .into_iter()
                .map(|name| {
                    let mut groups = Map::new();
                    for group in ["Alphabetic", "Ideographic", "Phonetic"] {
                        if let Some(node) = name.children_named(group).next() {
                            groups.insert(group.into(), Value::String(person_name_group(node)));
                        }
                    }
                    Value::Object(groups)
                })
                .collect();
            if !names.is_empty() {
                element.insert("Value".into(), Value::Array(names));
            }
        } else {
            let values = attribute
                .numbered("Value")
                .into_iter()
                .map(|value| {
                    if is_number(vr) {
                        number(&value.text)
                    } else {
                        Ok(Value::String(value.text.clone()))
                    }
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("DicomAttribute {tag}: {e}"))?;
            if !values.is_empty() {
                element.insert("Value".into(), Value::Array(values));
            }
        }
        object.insert(tag, Value::Object(element));
    }
    Ok(Value::Object(object))
}

/// One `NativeDicomModel` document as the DICOM JSON object of its data set.
pub(crate) fn native_to_json(xml: &[u8]) -> Result<Value, String> {
    let root = parse(xml)?;
    if root.name != "NativeDicomModel" {
        return Err(format!("a document of {}, not NativeDicomModel", root.name));
    }
    data_set(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_native_model_reads_as_dicom_json() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<NativeDicomModel xmlns="http://dicom.nema.org/PS3.19/models/NativeDICOM">
  <DicomAttribute tag="00100010" vr="PN" keyword="PatientName">
    <PersonName number="1"><Alphabetic><FamilyName>Doe</FamilyName><GivenName>Jane</GivenName></Alphabetic></PersonName>
  </DicomAttribute>
  <DicomAttribute tag="00200013" vr="IS"><Value number="1">7</Value></DicomAttribute>
  <DicomAttribute tag="00280030" vr="DS"><Value number="2">0.5</Value><Value number="1">0.7</Value></DicomAttribute>
  <DicomAttribute tag="00081030" vr="LO"><Value number="1">Liver &amp; spleen</Value></DicomAttribute>
  <DicomAttribute tag="00400275" vr="SQ"><Item number="1">
    <DicomAttribute tag="00400009" vr="SH"><Value number="1">STEP1</Value></DicomAttribute>
  </Item></DicomAttribute>
  <DicomAttribute tag="7fe00010" vr="OW"><BulkData uri="pixels"/></DicomAttribute>
  <DicomAttribute tag="00431029" vr="OB"><InlineBinary>AAEC</InlineBinary></DicomAttribute>
  <DicomAttribute tag="00080090" vr="PN"/>
</NativeDicomModel>"#;
        let json = native_to_json(xml).unwrap();
        assert_eq!(json["00100010"]["Value"][0]["Alphabetic"], "Doe^Jane");
        assert_eq!(json["00200013"]["Value"][0], 7);
        assert_eq!(json["00280030"]["Value"], serde_json::json!([0.7, 0.5]));
        assert_eq!(json["00081030"]["Value"][0], "Liver & spleen");
        assert_eq!(
            json["00400275"]["Value"][0]["00400009"]["Value"][0],
            "STEP1"
        );
        assert_eq!(json["7FE00010"]["BulkDataURI"], "pixels");
        assert_eq!(json["00431029"]["InlineBinary"], "AAEC");
        assert_eq!(json["00080090"], serde_json::json!({"vr": "PN"}));
    }

    #[test]
    fn what_is_not_the_native_model_says_why() {
        for (xml, said) in [
            (&b"<NativeDicomModel><DicomAttribute vr=\"LO\"/></NativeDicomModel>"[..], "without a tag"),
            (b"<NativeDicomModel><DicomAttribute tag=\"00200013\" vr=\"IS\"><Value number=\"1\">seven</Value></DicomAttribute></NativeDicomModel>", "is not a number"),
            (b"<Other/>", "not NativeDicomModel"),
            (b"<NativeDicomModel>", "unclosed"),
            (b"<NativeDicomModel><x></y></NativeDicomModel>", "not well-formed"),
        ] {
            let error = native_to_json(xml).unwrap_err();
            assert!(error.contains(said), "{error}");
        }
    }
}
