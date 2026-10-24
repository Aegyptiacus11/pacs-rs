//! DICOMweb for pacs-rs (PS3.18): QIDO-RS search, WADO-RS retrieve and STOW-RS store,
//! plus `POST /instances` for one file in the request body.
//!
//! [`router`] serves the DICOMweb root at `/`; nest it under a prefix to mount it
//! elsewhere, and set [`Config::base_url`] to the public address of that root so that
//! Retrieve URLs point at it.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::HeaderMap;
use axum::http::header::HOST;
use axum::routing::get;
use pacs_archive::Archive;

mod error;
pub mod media;
pub mod multipart;
mod qido;
mod wado;

pub use error::ApiError;

#[derive(Debug, Clone)]
pub struct Config {
    /// The public URL of the DICOMweb root, e.g. `https://pacs.example.org/dicom-web`.
    /// `None` builds it from the request's Host header as `http://<host>`.
    pub base_url: Option<String>,
    /// The largest request body accepted, for STOW-RS and uploads.
    pub max_upload_bytes: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            base_url: None,
            max_upload_bytes: 1 << 30,
        }
    }
}

#[derive(Debug, Clone)]
struct AppState {
    archive: Archive,
    config: Config,
}

impl AppState {
    fn base_url(&self, headers: &HeaderMap) -> String {
        match &self.config.base_url {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => headers
                .get(HOST)
                .and_then(|h| h.to_str().ok())
                .map(|host| format!("http://{host}"))
                .unwrap_or_default(),
        }
    }
}

pub fn router(archive: Archive, config: Config) -> Router {
    let limit = config.max_upload_bytes;
    let state = AppState { archive, config };
    Router::new()
        .route("/studies", get(qido::studies))
        .route(
            "/studies/{study}",
            get(wado::study),
        )
        .route("/studies/{study}/metadata", get(wado::study_metadata))
        .route("/studies/{study}/series", get(qido::series))
        .route("/studies/{study}/series/{series}", get(wado::series))
        .route(
            "/studies/{study}/series/{series}/metadata",
            get(wado::series_metadata),
        )
        .route(
            "/studies/{study}/series/{series}/instances",
            get(qido::instances),
        )
        .route(
            "/studies/{study}/series/{series}/instances/{instance}",
            get(wado::instance),
        )
        .route(
            "/studies/{study}/series/{series}/instances/{instance}/metadata",
            get(wado::instance_metadata),
        )
        .route(
            "/studies/{study}/series/{series}/instances/{instance}/frames/{frames}",
            get(wado::frames),
        )
        .layer(DefaultBodyLimit::max(limit))
        .with_state(state)
}
