//! Clean-up of dicom-json's output.
//!
//! dicom-json writes string values as stored, padding included (`"1 "`, UIDs ending in NUL),
//! and IS / DS as those strings. PS3.18 Table F.2.3-1 allows IS and DS as either JSON
//! numbers or strings. IS become numbers when they parse; DS stay strings, trimmed, which
//! keeps the stored decimal exactly ("2.50" is not 2.5 to everyone) and is what the
//! reference server answers too.
//!
//! dicom-json also writes an empty person name as `{"Alphabetic": ""}`. An attribute with no
//! value has no `Value` (PS3.18 F.2.5), and an empty value among others is `null`: so here.

use serde_json::Value;

/// Normalize a DICOM JSON data set in place, sequences included.
pub fn normalize(dataset: &mut Value) {
    let Some(map) = dataset.as_object_mut() else {
        return;
    };
    for element in map.values_mut() {
        let vr = element
            .get("vr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let Some(Value::Array(values)) = element.get_mut("Value") else {
            continue;
        };
        for value in values.iter_mut() {
            match value {
                Value::Object(_) if vr == "SQ" => normalize(value),
                Value::String(s) => {
                    let trimmed = s.trim_end_matches([' ', '\0']);
                    *value = match vr.as_str() {
                        "IS" => int(trimmed.trim_start()),
                        "DS" => Value::String(trimmed.trim_start().to_owned()),
                        _ => Value::String(trimmed.to_owned()),
                    };
                }
                _ => {}
            }
        }
        if vr == "PN" {
            for value in values.iter_mut() {
                if no_name(value) {
                    *value = Value::Null;
                }
            }
            if values.iter().all(Value::is_null)
                && let Some(element) = element.as_object_mut()
            {
                element.remove("Value");
            }
        }
    }
}

/// Whether a person name value says nothing: no component group, or empty ones only.
fn no_name(value: &Value) -> bool {
    match value {
        Value::Object(groups) => groups.values().all(|group| {
            group
                .as_str()
                .is_some_and(|text| text.trim_end().is_empty())
        }),
        Value::String(text) => text.is_empty(),
        _ => false,
    }
}

fn int(s: &str) -> Value {
    match s.strip_prefix('+').unwrap_or(s).parse::<i64>() {
        Ok(n) => Value::from(n),
        Err(_) => Value::String(s.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use serde_json::json;

    #[test]
    fn an_empty_person_name_has_no_value() {
        let mut ds = json!({
            "00100010": {"vr": "PN", "Value": [{"Alphabetic": ""}]},
            "00080090": {"vr": "PN", "Value": [{"Alphabetic": "", "Ideographic": ""}]},
            "00081048": {"vr": "PN", "Value": [{"Alphabetic": "A^B"}, {"Alphabetic": ""}]},
            "00081050": {"vr": "PN", "Value": [{"Ideographic": "山田^太郎"}]},
            "00081060": {"vr": "PN"},
            "00400275": {"vr": "SQ", "Value": [{"00321032": {"vr": "PN", "Value": [{}]}}]},
            // not a name: an empty string of another VR is left to what wrote it
            "00081030": {"vr": "LO", "Value": [""]}
        });
        normalize(&mut ds);
        assert_eq!(
            ds,
            json!({
                "00100010": {"vr": "PN"},
                "00080090": {"vr": "PN"},
                "00081048": {"vr": "PN", "Value": [{"Alphabetic": "A^B"}, null]},
                "00081050": {"vr": "PN", "Value": [{"Ideographic": "山田^太郎"}]},
                "00081060": {"vr": "PN"},
                "00400275": {"vr": "SQ", "Value": [{"00321032": {"vr": "PN"}}]},
                "00081030": {"vr": "LO", "Value": [""]}
            })
        );
    }

    #[test]
    fn padding_goes_and_numbers_become_numbers() {
        let mut ds = json!({
            "00200013": {"vr": "IS", "Value": ["1 "]},
            "00200032": {"vr": "DS", "Value": ["-12.5", " 0", "1e3 "]},
            "0020000D": {"vr": "UI", "Value": ["1.2.3\u{0}"]},
            "00080060": {"vr": "CS", "Value": ["CT "]},
            "00281050": {"vr": "DS", "Value": ["not a number"]},
            "00081115": {"vr": "SQ", "Value": [{"00200013": {"vr": "IS", "Value": ["+7"]}}]},
            "00280010": {"vr": "US", "Value": [512]},
            "00100010": {"vr": "PN", "Value": [{"Alphabetic": "A^B"}]},
            "00081030": {"vr": "LO"}
        });
        normalize(&mut ds);
        assert_eq!(
            ds,
            json!({
                "00200013": {"vr": "IS", "Value": [1]},
                "00200032": {"vr": "DS", "Value": ["-12.5", "0", "1e3"]},
                "0020000D": {"vr": "UI", "Value": ["1.2.3"]},
                "00080060": {"vr": "CS", "Value": ["CT"]},
                "00281050": {"vr": "DS", "Value": ["not a number"]},
                "00081115": {"vr": "SQ", "Value": [{"00200013": {"vr": "IS", "Value": [7]}}]},
                "00280010": {"vr": "US", "Value": [512]},
                "00100010": {"vr": "PN", "Value": [{"Alphabetic": "A^B"}]},
                "00081030": {"vr": "LO"}
            })
        );
    }
}
