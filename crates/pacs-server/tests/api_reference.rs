//! docs/api-reference.md against the handlers' descriptions, and `/openapi.json`.

use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clap::Parser;
use http_body_util::BodyExt;
use pacs_server::api::{description, reference, reference_of};
use pacs_server::{Settings, app, open_archive};
use serde_json::{Value, json};
use tower::ServiceExt;

/// The description as JSON, with one change.
fn changed(change: impl FnOnce(&mut Value)) -> Value {
    let mut api = serde_json::to_value(description()).unwrap();
    change(&mut api);
    api
}

#[test]
fn the_page_is_what_the_handlers_say_today() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/api-reference.md");
    let page = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("{}: {e}", file.display()))
        // a Windows checkout may give the page other line endings
        .replace("\r\n", "\n");
    assert!(
        page == reference(&description()).unwrap(),
        "docs/api-reference.md is behind the handlers' descriptions: \
         run `cargo run -p pacs-server --example api_reference` and commit the page"
    );
}

#[test]
fn every_request_and_object_is_on_the_page() {
    let api = serde_json::to_value(description()).unwrap();
    let page = reference(&description()).unwrap();
    let mut operations = 0;
    for (path, item) in api["paths"].as_object().unwrap() {
        for method in item.as_object().unwrap().keys() {
            operations += 1;
            let heading = format!("### `{} {path}`\n", method.to_uppercase());
            assert_eq!(page.matches(&heading).count(), 1, "{heading}");
        }
    }
    assert!(operations > 30, "{operations} operations");
    for name in ["Patient", "Study", "Series", "Instance", "Error", "Find"] {
        assert!(page.contains(&format!("\n### {name}\n")), "{name}");
    }
    // the permission comes from the guard's own rule, not from the description
    let deletion = page.split("### `DELETE /studies/{id}`").nth(1).unwrap();
    assert!(
        deletion.starts_with("\n\nDeletes a study.\n"),
        "{deletion:.80}"
    );
    assert!(deletion.contains("Permission, when the login is on: `delete`."));
    let search = page.split("### `POST /tools/find`").nth(1).unwrap();
    assert!(search.contains("Permission, when the login is on: `read`."));
    let upload = page.split("### `POST /instances`").nth(1).unwrap();
    assert!(upload.contains("Permission, when the login is on: `write`."));
    assert!(upload.contains("| the body | | `application/dicom` |"));
    // a `|` in a description does not end its table cell
    assert!(page.contains("`PatientID\\|StudyInstanceUID`"));
}

#[test]
fn what_the_page_cannot_show_is_an_error_not_a_gap() {
    for (change, why) in [
        (
            Box::new(|api: &mut Value| {
                api["paths"]["/studies"]["get"]["tags"] = json!(["Elsewhere"]);
            }) as Box<dyn FnOnce(&mut Value)>,
            "are on the page",
        ),
        (
            Box::new(|api: &mut Value| {
                api["paths"]["/studies"]["get"]["summary"] = json!(" ");
            }),
            "get /studies: no summary",
        ),
        (
            Box::new(|api: &mut Value| {
                api["paths"]["/studies"]["get"]["responses"] = json!({});
            }),
            "get /studies: no answer is described",
        ),
        (
            Box::new(|api: &mut Value| {
                api["paths"]["/studies"]["get"]["responses"]["200"]["description"] = json!("");
            }),
            "get /studies: 200: no description",
        ),
        (
            Box::new(|api: &mut Value| {
                api["paths"]["/studies/{id}"]["get"]["parameters"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("description");
            }),
            "get /studies/{id}: id: no description",
        ),
        (
            Box::new(|api: &mut Value| {
                api["components"]["schemas"]["Study"]["properties"]["ID"]["type"] = json!("uuid");
            }),
            "Study.ID: a type this page cannot show: uuid",
        ),
        (
            Box::new(|api: &mut Value| {
                api["components"]["schemas"]["Study"]
                    .as_object_mut()
                    .unwrap()
                    .remove("description");
            }),
            "Study: no description",
        ),
        (
            Box::new(|api: &mut Value| {
                api.as_object_mut().unwrap().remove("tags");
            }),
            "no tags",
        ),
    ] {
        let error = reference_of(&changed(change)).unwrap_err();
        assert!(error.contains(why), "{why}: {error}");
    }
}

#[tokio::test]
async fn the_server_serves_its_description() {
    let dir = tempfile::tempdir().unwrap();
    let settings = Settings::try_parse_from([
        "pacs-server".to_owned(),
        "--index".into(),
        dir.path().join("index.sqlite").display().to_string(),
        "--storage".into(),
        format!("file:{}", dir.path().join("files").display()),
    ])
    .unwrap();
    let app = app(open_archive(&settings).await.unwrap(), &settings).unwrap();
    let request = Request::get("/openapi.json").body(Body::empty()).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let served: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(served, serde_json::to_value(description()).unwrap());
    assert_eq!(served["openapi"], "3.1.0");
    assert_eq!(served["info"]["title"], "pacs-rs");
    assert_eq!(served["info"]["version"], env!("CARGO_PKG_VERSION"));

    // each described request is one the server routes: an unknown identifier is refused by
    // its handler (404 or 400), never by the router for the method (405)
    for (path, item) in served["paths"].as_object().unwrap() {
        for method in item.as_object().unwrap().keys() {
            // each `{name}` of the path, as a name that names nothing
            let uri: String = path
                .split('/')
                .map(|part| {
                    if part.starts_with('{') {
                        "nothing"
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let request = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&uri)
                .body(Body::empty())
                .unwrap();
            let status = app.clone().oneshot(request).await.unwrap().status();
            assert_ne!(status, StatusCode::METHOD_NOT_ALLOWED, "{method} {uri}");
        }
    }
}
