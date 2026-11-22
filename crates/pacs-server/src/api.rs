//! The description of the HTTP API, and the reference page written from it.
//!
//! The description is OpenAPI 3.1, collected from the `#[utoipa::path]` of each handler
//! (`pacs_rest::api`) and served at `/openapi.json`. `reference` writes
//! docs/api-reference.md from it; `tests/api_reference.rs` fails when that page is not what
//! `reference` writes today, so the page cannot fall behind the handlers.

use axum::http::Method;
use serde_json::{Map, Value};
use utoipa::openapi::tag::TagBuilder;
use utoipa::openapi::{Info, License, OpenApi};

use crate::auth::{Right, is_open, right_for};

/// The sections of the reference, in order, each with what it is about.
const TAGS: [(&str, &str); 13] = [
    (
        "Patients",
        "A patient is the studies that share a Patient ID.",
    ),
    ("Studies", "One examination of a patient."),
    ("Series", "One acquisition within a study."),
    (
        "Instances",
        "One stored DICOM file. Uploads arrive here, one file a request.",
    ),
    (
        "Search and archives",
        "Finding resources by their DICOM attributes, and several resources in one ZIP.",
    ),
    (
        "DICOMweb",
        "The standard's own HTTP interface (PS3.18), under `/dicom-web` unless `PACS_DICOMWEB_PATH` says otherwise: what viewers speak. What of it is served, and how it matches and transcodes, is on [DICOMweb](dicomweb.md).",
    ),
    (
        "Remote archives",
        "Other DICOM systems, reached over the DICOM network protocol: testing the link, searching their studies, bringing studies in.",
    ),
    (
        "Jobs",
        "Moves from a remote archive that run in the background.",
    ),
    (
        "Integrity",
        "Each stored file checked against the size and SHA-256 the index took when it was stored. Only for a role with `users`.",
    ),
    (
        "Signing in",
        "With `PACS_AUTH=true`: sessions for people, and what a signed-in account may ask about itself.",
    ),
    (
        "Users",
        "With `PACS_AUTH=true`, for a role with `users`: who may sign in, with which role, on which projects.",
    ),
    (
        "Roles",
        "With `PACS_AUTH=true`: what each role allows. Read by anyone signed in, changed by a role with `users`.",
    ),
    (
        "Tools",
        "With `PACS_AUTH=true`: programs that connect without a person, with an id, a secret and a token of five minutes.",
    ),
];

/// The answers of the login's routes, as types.
pub mod accounts {
    pub use crate::auth_schema::*;
}

/// The answers of the remote AE routes, as types.
pub mod remote {
    pub use crate::modalities::schema::*;
}

const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];

/// The description of the API this server serves.
pub fn description() -> OpenApi {
    let api = pacs_rest::api();
    // under the path it is served at unless the settings say otherwise
    let mut api = api.nest("/dicom-web", pacs_dicomweb::api());
    api.merge(crate::modalities::api());
    api.merge(crate::auth::api());
    api.merge(crate::checks::api());
    let mut info = Info::new("pacs-rs", env!("CARGO_PKG_VERSION"));
    info.description = Some(
        "The REST API of pacs-rs: patients, studies, series and instances by identifier. \
         With the login on, every request carries `Authorization: Bearer <access token>`."
            .to_owned(),
    );
    info.license = Some(License::new("MIT OR Apache-2.0"));
    api.info = info;
    api.tags = Some(
        TAGS.iter()
            .map(|(name, about)| {
                TagBuilder::new()
                    .name(*name)
                    .description(Some(*about))
                    .build()
            })
            .collect(),
    );
    api
}

/// A line of a table cell: no line break, and no `|` that would end the cell.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn anchor(name: &str) -> String {
    name.to_lowercase()
}

/// A schema in words: `string`, `list of [Study](#study)`, `integer or null`.
fn kind(schema: &Value, at: &str) -> Result<String, String> {
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{at}: a schema is an object, not {schema}"))?;
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let name = reference
            .strip_prefix("#/components/schemas/")
            .ok_or_else(|| format!("{at}: a reference outside the schemas: {reference}"))?;
        return Ok(format!("[{name}](#{})", anchor(name)));
    }
    if let Some(alternatives) = object.get("oneOf").and_then(Value::as_array) {
        let kinds: Result<Vec<_>, _> = alternatives.iter().map(|a| kind(a, at)).collect();
        return Ok(kinds?.join(" or "));
    }
    let one = |name: &str| -> Result<String, String> {
        Ok(match name {
            "array" => {
                let items = object
                    .get("items")
                    .ok_or_else(|| format!("{at}: a list without its items"))?;
                format!("list of {}", kind(items, at)?)
            }
            "object" => match object.get("additionalProperties") {
                Some(values @ Value::Object(_)) => format!("object of {}", kind(values, at)?),
                _ => "object".to_owned(),
            },
            "string" if object.get("format").and_then(Value::as_str) == Some("binary") => {
                "bytes".to_owned()
            }
            "string" | "integer" | "number" | "boolean" | "null" => name.to_owned(),
            other => return Err(format!("{at}: a type this page cannot show: {other}")),
        })
    };
    match object.get("type") {
        Some(Value::String(name)) => one(name),
        Some(Value::Array(names)) => {
            let kinds: Result<Vec<_>, String> = names
                .iter()
                .map(|n| {
                    one(n
                        .as_str()
                        .ok_or_else(|| format!("{at}: a type is a name"))?)
                })
                .collect();
            Ok(kinds?.join(" or "))
        }
        _ => Err(format!("{at}: a schema without a type: {schema}")),
    }
}

/// The body of a request or an answer: its media type and what it holds.
fn body(content: Option<&Value>, at: &str) -> Result<String, String> {
    let Some(content) = content.and_then(Value::as_object) else {
        return Ok("nothing".to_owned());
    };
    let mut bodies = Vec::new();
    for (media, described) in content {
        let schema = described
            .get("schema")
            .ok_or_else(|| format!("{at}: {media} without a schema"))?;
        bodies.push(match media.as_str() {
            "application/json" => kind(schema, at)?,
            _ => format!("`{media}`"),
        });
    }
    Ok(bodies.join(" or "))
}

fn permission(method: &str, path: &str) -> Result<String, String> {
    let method = Method::from_bytes(method.to_uppercase().as_bytes())
        .map_err(|e| format!("{method} {path}: {e}"))?;
    if is_open(&method, path) {
        return Ok("none: it needs no token".to_owned());
    }
    Ok(match right_for(&method, path) {
        Right::SignedIn => "any signed-in user".to_owned(),
        right => match right.permission() {
            Some(permission) => format!("`{}`", permission.as_str()),
            None => return Err(format!("{method} {path}: a right without a permission")),
        },
    })
}

fn text<'a>(value: &'a Value, key: &str, at: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| format!("{at}: no {key}"))
}

fn operation(page: &mut String, method: &str, path: &str, described: &Value) -> Result<(), String> {
    let at = format!("{method} {path}");
    page.push_str(&format!("### `{} {path}`\n\n", method.to_uppercase()));
    page.push_str(&format!("{}\n\n", text(described, "summary", &at)?));
    if let Some(more) = described.get("description").and_then(Value::as_str) {
        page.push_str(&format!("{}\n\n", more.trim()));
    }
    page.push_str(&format!(
        "Permission, when the login is on: {}.\n\n",
        permission(method, path)?
    ));

    let parameters = described.get("parameters").and_then(Value::as_array);
    let sent = described.get("requestBody");
    if parameters.is_some() || sent.is_some() {
        page.push_str("| Send | Where | Type | |\n|---|---|---|---|\n");
        for parameter in parameters.into_iter().flatten() {
            let name = text(parameter, "name", &at)?;
            let at = format!("{at}: {name}");
            let schema = parameter
                .get("schema")
                .ok_or_else(|| format!("{at}: no schema"))?;
            let optional = match parameter.get("required").and_then(Value::as_bool) {
                Some(true) => "",
                _ => ", optional",
            };
            page.push_str(&format!(
                "| `{name}` | {}{optional} | {} | {} |\n",
                text(parameter, "in", &at)?,
                kind(schema, &at)?,
                cell(text(parameter, "description", &at)?),
            ));
        }
        if let Some(sent) = sent {
            let at = format!("{at}: body");
            page.push_str(&format!(
                "| the body | | {} | {} |\n",
                body(sent.get("content"), &at)?,
                cell(text(sent, "description", &at)?),
            ));
        }
        page.push('\n');
    }

    let responses = described
        .get("responses")
        .and_then(Value::as_object)
        .filter(|responses| !responses.is_empty())
        .ok_or_else(|| format!("{at}: no answer is described"))?;
    page.push_str("| Status | Answer | |\n|---|---|---|\n");
    for (status, response) in responses {
        let at = format!("{at}: {status}");
        page.push_str(&format!(
            "| {status} | {} | {} |\n",
            body(response.get("content"), &at)?,
            cell(text(response, "description", &at)?),
        ));
    }
    page.push('\n');
    Ok(())
}

fn object(page: &mut String, name: &str, schema: &Value) -> Result<(), String> {
    page.push_str(&format!("### {name}\n\n"));
    page.push_str(&format!("{}\n\n", cell(text(schema, "description", name)?)));
    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        // not an object with fields: a list of alternatives, or bytes
        page.push_str(&format!("{}.\n\n", kind(schema, name)?));
        return Ok(());
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    page.push_str("| Field | Type | |\n|---|---|---|\n");
    for (field, described) in properties {
        let at = format!("{name}.{field}");
        // a reference with a description beside it comes as one alternative of a oneOf
        let about = described
            .get("description")
            .or_else(|| {
                described
                    .get("oneOf")?
                    .as_array()?
                    .iter()
                    .find_map(|alternative| alternative.get("description"))
            })
            .and_then(Value::as_str)
            .unwrap_or("");
        let optional = if required.contains(&field.as_str()) {
            ""
        } else {
            ", optional"
        };
        page.push_str(&format!(
            "| `{field}` | {}{optional} | {} |\n",
            kind(described, &at)?,
            cell(about),
        ));
    }
    page.push('\n');
    Ok(())
}

/// docs/api-reference.md for this description. An error names what the page cannot show,
/// so that nothing is left out of it silently.
pub fn reference(api: &OpenApi) -> Result<String, String> {
    reference_of(&serde_json::to_value(api).map_err(|e| e.to_string())?)
}

/// The same, for a description as JSON.
pub fn reference_of(api: &Value) -> Result<String, String> {
    let empty = Map::new();
    let paths = api
        .get("paths")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let tags = api
        .get("tags")
        .and_then(Value::as_array)
        .ok_or("the description has no tags: the page has no sections")?;

    let mut page = String::from(
        "# REST API reference\n\n\
         <!-- Written by `cargo run -p pacs-server --example api_reference`. Do not edit: \
         change the handler's description, then run it again. -->\n\n\
         Every request of the REST API, written from the description each handler carries in \
         the code: a request is on this page because the server serves it, and the answers \
         are the ones its tests read. For how the requests fit together, see \
         [REST API, task by task](rest-guide.md).\n\n\
         The same description is served by a running server at `/openapi.json` (OpenAPI \
         3.1), for a client generator or an API browser.\n\n\
         An identifier that names nothing is answered 404 with an empty body. Every other \
         refusal is an [Error](#error); the login's own routes refuse with \
         [Refused](#refused). With the [login](login.md) on, a request without a \
         valid token, or with one that lacks the permission, is answered 401.\n\n\
         DICOMweb's own refusals are a [Problem](#problem).\n\n",
    );

    let mut shown = 0;
    for tag in tags {
        let name = text(tag, "name", "a tag")?;
        page.push_str(&format!(
            "## {name}\n\n{}\n\n",
            text(tag, "description", name)?
        ));
        for (path, item) in paths {
            for method in METHODS {
                let Some(described) = item.get(method) else {
                    continue;
                };
                let tagged = described.get("tags").and_then(Value::as_array);
                if tagged.is_some_and(|t| t.len() == 1 && t[0] == name) {
                    operation(&mut page, method, path, described)?;
                    shown += 1;
                }
            }
        }
    }
    let operations: usize = paths
        .values()
        .filter_map(Value::as_object)
        .map(Map::len)
        .sum();
    if shown != operations {
        return Err(format!(
            "{operations} operations are described and {shown} are on the page: each needs \
             one tag of the page's sections and a known method"
        ));
    }

    page.push_str("## Objects\n\nWhat the requests above send and answer.\n\n");
    let schemas = api
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    for (name, schema) in schemas {
        // bytes are shown by their media type where they are sent, not as an object
        if schema.get("format").and_then(Value::as_str) == Some("binary") {
            continue;
        }
        object(&mut page, name, schema)?;
    }
    Ok(page)
}
