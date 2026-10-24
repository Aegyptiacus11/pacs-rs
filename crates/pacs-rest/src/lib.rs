//! The pacs-rs REST API: studies, series and instances addressed by public ID
//! (`pacs_core::public_ids`), uploads, search and study archives.
//!
//! Resources, identifiers and answers follow the reference server's REST API, so clients
//! written for it work unchanged; `crates/pacs-server/tests/reference.rs` checks that.
//! The routes so far are the ones Quorum uses, plus their series and instance
//! counterparts.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use pacs_archive::Archive;

mod archive;
mod error;
mod find;
mod resources;
mod upload;

pub use error::RestError;

#[derive(Debug, Clone)]
struct AppState {
    archive: Archive,
}

/// The REST API at `/`; `max_upload_bytes` bounds `POST /instances`.
pub fn router(archive: Archive, max_upload_bytes: usize) -> Router {
    Router::new()
        .route("/instances", post(upload::instance))
        .route("/instances/{id}", get(resources::instance))
        .route("/instances/{id}/file", get(resources::instance_file))
        .route("/series/{id}", get(resources::series))
        .route("/series/{id}/instances", get(resources::series_instances))
        .route("/studies", get(resources::studies))
        .route("/studies/{id}", get(resources::study))
        .route("/studies/{id}/series", get(resources::study_series))
        .route("/studies/{id}/instances", get(resources::study_instances))
        .route("/studies/{id}/statistics", get(resources::study_statistics))
        .route("/studies/{id}/archive", get(archive::study))
        .route("/tools/find", post(find::find))
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(AppState { archive })
}
