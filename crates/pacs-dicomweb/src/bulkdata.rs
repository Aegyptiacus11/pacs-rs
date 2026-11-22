//! Bulk data (PS3.18 8.6.1.3, Annex F.2.6): in metadata, a binary value larger than
//! [`INLINE_MAX`] is not written inline but named by a BulkDataURI, which this module serves:
//! `.../instances/{instance}/bulkdata/{path}`, the path the element's tag, or the tags and
//! item numbers (from 0) down to it inside sequences: `00431029`, or `00400275/0/00081032`.

use std::borrow::Cow;

use axum::extract::Path;
use axum::http::HeaderMap;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use dicom_core::Tag;
use dicom_core::header::Header;
use dicom_core::value::Value as DicomValue;
use pacs_core::Instance;
use serde_json::{Map, Value};

use crate::ApiError;
use crate::media::{Accept, Packaging, choose};
use crate::multipart;
use crate::qido::instance_url;
use crate::schema::{Binary, Problem};
use crate::scope::Seen;

/// A binary value of more than this many bytes is bulk data in metadata.
pub const INLINE_MAX: usize = 1024;

const OCTET_STREAM: &str = "application/octet-stream";

/// The bytes base64 of this length stands for, at most.
fn decoded_len(base64: &str) -> usize {
    base64.len() / 4 * 3
}

/// An instance's stored metadata as it is answered: its large binary values replaced by
/// BulkDataURIs under `base`. Text without any binary value is passed on as it is.
pub(crate) fn with_bulk_uris<'a>(text: &'a str, base: &str) -> Cow<'a, str> {
    if !text.contains("\"InlineBinary\"") {
        return Cow::Borrowed(text);
    }
    let Ok(mut metadata) = serde_json::from_str::<Value>(text) else {
        return Cow::Borrowed(text);
    };
    let uid = |tag: &str| {
        pacs_index::matching::values(&metadata, tag)
            .into_iter()
            .next()
            .unwrap_or_default()
    };
    let at = format!(
        "{}/bulkdata",
        instance_url(base, &uid("0020000D"), &uid("0020000E"), &uid("00080018"))
    );
    let mut changed = false;
    if let Some(object) = metadata.as_object_mut() {
        replace(object, &at, &mut changed);
    }
    if !changed {
        return Cow::Borrowed(text);
    }
    Cow::Owned(metadata.to_string())
}

fn replace(object: &mut Map<String, Value>, at: &str, changed: &mut bool) {
    for (tag, element) in object.iter_mut() {
        let Some(element) = element.as_object_mut() else {
            continue;
        };
        let large = element
            .get("InlineBinary")
            .and_then(Value::as_str)
            .is_some_and(|b| decoded_len(b) > INLINE_MAX);
        if large {
            element.remove("InlineBinary");
            element.insert("BulkDataURI".into(), Value::String(format!("{at}/{tag}")));
            *changed = true;
            continue;
        }
        if let Some(Value::Array(items)) = element.get_mut("Value") {
            for (number, item) in items.iter_mut().enumerate() {
                if let Some(item) = item.as_object_mut() {
                    replace(item, &format!("{at}/{tag}/{number}"), changed);
                }
            }
        }
    }
}

/// The value of the element `path` names in `instance`, as bytes.
fn value_at(instance: &Instance, path: &str) -> Result<Vec<u8>, ApiError> {
    let bad = || {
        ApiError::bad_request(format!(
            "a bulk data path is a tag, or tags and item numbers down to one: not {path:?}"
        ))
    };
    let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
    if parts.len().is_multiple_of(2) {
        return Err(bad());
    }
    let tag = |text: &str| {
        (text.len() == 8)
            .then(|| u32::from_str_radix(text, 16).ok())
            .flatten()
            .map(|n| Tag((n >> 16) as u16, n as u16))
            .ok_or_else(bad)
    };
    let missing = || ApiError::not_found(format!("the instance has no element at {path}"));
    let mut object: &dicom_object::InMemDicomObject = &instance.object;
    for pair in parts[..parts.len() - 1].chunks(2) {
        let (sequence, item) = (tag(pair[0])?, pair[1].parse::<usize>().map_err(|_| bad())?);
        object = object
            .get(sequence)
            .and_then(|e| e.items())
            .and_then(|items| items.get(item))
            .ok_or_else(missing)?;
    }
    let last = tag(parts[parts.len() - 1])?;
    if last == dicom_dictionary_std::tags::PIXEL_DATA && parts.len() == 1 {
        return Err(ApiError::not_found(
            "the pixel data is retrieved by its frames (.../frames/{list})",
        ));
    }
    let element = object.get(last).ok_or_else(missing)?;
    match element.value() {
        DicomValue::Primitive(value) => Ok(value.to_bytes().into_owned()),
        _ => Err(ApiError::not_found(format!(
            "the element at {path} ({}) has no value of bytes",
            element.tag()
        ))),
    }
}

/// Retrieves bulk data of an instance (WADO-RS).
///
/// The value of one element that its metadata names by a BulkDataURI, as `application/octet-stream`.
#[utoipa::path(
    get,
    path = "/studies/{study}/series/{series}/instances/{instance}/bulkdata/{*path}",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID."), ("series" = String, Path, description = "Series Instance UID."), ("instance" = String, Path, description = "SOP Instance UID."), ("path" = String, Path, description = "The element's tag, or tags and item numbers from 0 down to it, separated by `/`.")),
    responses(
        (status = 200, description = "The value, one part.", body = Binary, content_type = "multipart/related"),
        (status = 400, description = "Not a path of tags and item numbers.", body = Problem),
        (status = 404, description = "No such instance, or no element of bytes there; the pixel data is retrieved by its frames.", body = Problem),
        (status = 406, description = "`Accept` takes neither `application/octet-stream` nor a multipart of it.", body = Problem)
    )
)]
pub(crate) async fn bulkdata(
    Seen(state): Seen,
    headers: HeaderMap,
    Path((study, series, instance, path)): Path<(String, String, String, String)>,
) -> Result<Response, ApiError> {
    let accept = Accept::parse(
        headers
            .get(axum::http::header::ACCEPT)
            .and_then(|h| h.to_str().ok()),
    );
    let (_, packaging) = choose(
        &accept,
        (OCTET_STREAM, pacs_core::codec::EXPLICIT_VR_LE),
        &[],
    )
    .ok_or_else(|| ApiError::not_acceptable("bulk data comes as application/octet-stream"))?;
    let entry = state
        .archive
        .index()
        .instance(&study, &series, &instance)
        .await?
        .ok_or_else(|| ApiError::not_found("no such instance"))?;
    let file = state.archive.file(&entry).await?;
    let reading = state.archive.admit_stored(&file).await;
    let value = tokio::task::spawn_blocking(move || {
        let _reading = reading;
        let instance = Instance::from_bytes(&file).map_err(ApiError::internal)?;
        value_at(&instance, &path)
    })
    .await
    .map_err(ApiError::internal)??;
    if packaging == Packaging::Single {
        return Ok(([(CONTENT_TYPE, OCTET_STREAM)], value).into_response());
    }
    let boundary = multipart::new_boundary();
    let body = multipart::body(
        &boundary,
        std::iter::once((OCTET_STREAM.to_owned(), Bytes::from(value))),
    );
    Ok((
        [(
            CONTENT_TYPE,
            format!(r#"multipart/related; type="{OCTET_STREAM}"; boundary={boundary}"#),
        )],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_binary_values_become_bulk_data_uris() {
        let large = "A".repeat(2000);
        let text = serde_json::json!({
            "0020000D": {"vr": "UI", "Value": ["1.2"]},
            "0020000E": {"vr": "UI", "Value": ["1.2.3"]},
            "00080018": {"vr": "UI", "Value": ["1.2.3.4"]},
            "00431029": {"vr": "OB", "InlineBinary": large},
            "00281201": {"vr": "OW", "InlineBinary": "AAAA"},
            "00400275": {"vr": "SQ", "Value": [{"00091001": {"vr": "OB", "InlineBinary": large}}]},
        })
        .to_string();
        let answered: Value = serde_json::from_str(&with_bulk_uris(&text, "http://h/dw")).unwrap();
        let at = "http://h/dw/studies/1.2/series/1.2.3/instances/1.2.3.4/bulkdata";
        assert_eq!(
            answered["00431029"]["BulkDataURI"],
            format!("{at}/00431029")
        );
        assert!(answered["00431029"].get("InlineBinary").is_none());
        assert_eq!(answered["00281201"]["InlineBinary"], "AAAA");
        assert_eq!(
            answered["00400275"]["Value"][0]["00091001"]["BulkDataURI"],
            format!("{at}/00400275/0/00091001")
        );
        // nothing binary: as it is
        let plain = r#"{"00100010":{"vr":"PN"}}"#;
        assert!(matches!(with_bulk_uris(plain, "x"), Cow::Borrowed(_)));
    }
}
