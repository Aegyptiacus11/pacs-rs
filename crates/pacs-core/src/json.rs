//! Clean-up of dicom-json's output.
//!
//! dicom-json writes string values as stored, padding included (`"1 "`, UIDs ending in NUL),
//! and IS / DS as those strings. PS3.18 Table F.2.3-1 allows IS and DS as either JSON
//! numbers or strings; viewers parse numbers most reliably, so they become numbers when
//! they parse without loss of meaning, and stay (trimmed) strings otherwise.

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
                        "DS" => decimal(trimmed.trim_start()),
                        _ => Value::String(trimmed.to_owned()),
                    };
                }
                _ => {}
            }
        }
    }
}

fn int(s: &str) -> Value {
    match s.strip_prefix('+').unwrap_or(s).parse::<i64>() {
        Ok(n) => Value::from(n),
        Err(_) => Value::String(s.to_owned()),
    }
}

fn decimal(s: &str) -> Value {
    match s.parse::<f64>() {
        Ok(x) if x.is_finite() => serde_json::Number::from_f64(x)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(s.to_owned())),
        _ => Value::String(s.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize;
    use serde_json::json;

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
                "00200032": {"vr": "DS", "Value": [-12.5, 0.0, 1000.0]},
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
