//! The web interface at `/ui/`: the files of `ui/dist`, built into the binary by `build.rs`,
//! and `config.json` telling them where DICOMweb is. The page calls the REST API and
//! DICOMweb with relative URLs, so it also works behind a reverse proxy that serves pacs-rs
//! under a path.

use axum::Router;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/ui_files.rs"));
}

/// Only this origin's scripts (and the decoders' WebAssembly), styles and connections; never
/// in a frame, so another site cannot trick a click on Delete.
const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self' blob:; \
                   img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; \
                   frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

const NOT_BUILT: &str = "<!doctype html><meta charset=utf-8><title>pacs-rs</title>\
<body style=\"font:14px system-ui;background:#0b0c0d;color:#d6d9dc;padding:40px\">\
<h1 style=\"font-size:16px\">The web interface was not built into this server</h1>\
<p>Run <code>npm ci &amp;&amp; npm run build</code> in <code>ui/</code>, then build pacs-server again. \
The REST API and DICOMweb work without it.</p>";

fn find(path: &str) -> Option<(&'static str, &'static [u8])> {
    embedded::FILES
        .iter()
        .find(|(p, _, _)| *p == path)
        .map(|(_, t, b)| (*t, *b))
}

fn with_headers(mut response: Response, content_type: &str, immutable: bool) -> Response {
    let headers = response.headers_mut();
    if let Ok(value) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, value);
    }
    // assets/ names carry a content hash: they never change; the rest is checked each time
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

async fn index() -> Response {
    match find("index.html") {
        Some((t, body)) => with_headers(body.into_response(), t, false),
        None => with_headers(
            Html(NOT_BUILT).into_response(),
            "text/html; charset=utf-8",
            false,
        ),
    }
}

async fn file(Path(path): Path<String>) -> Response {
    match find(&path) {
        Some((t, body)) => with_headers(body.into_response(), t, path.starts_with("assets/")),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Whether this binary has the web interface in it.
pub(crate) fn built() -> bool {
    find("index.html").is_some()
}

/// `/ui/` and its files; `/` and `/ui` redirect there. `dicomweb_path` is where DICOMweb is
/// served, e.g. `/dicom-web`.
pub fn router(dicomweb_path: &str) -> Router {
    let config = serde_json::json!({
        "dicomweb": dicomweb_path,
        "version": env!("CARGO_PKG_VERSION"),
    })
    .to_string();
    Router::new()
        // relative, so a proxy's path prefix is kept
        .route("/", get(|| async { Redirect::temporary("ui/") }))
        .route("/ui", get(|| async { Redirect::permanent("ui/") }))
        .route("/ui/", get(index))
        .route("/ui/index.html", get(|| async { Redirect::permanent("./") }))
        .route(
            "/ui/config.json",
            get(move || async move { with_headers(config.into_response(), "application/json", false) }),
        )
        .route("/ui/{*path}", get(file))
}
