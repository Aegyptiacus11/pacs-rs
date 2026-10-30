//! The pacs-rs REST API: patients, studies, series and instances addressed by public ID
//! (`pacs_core::public_ids`), uploads, deletion, search and archives.
//!
//! Resources, identifiers and answers follow the reference server's REST API, so clients
//! written for it work unchanged; `crates/pacs-server/tests/reference.rs` checks that.
//! The routes so far are the ones Quorum uses, their counterparts at the other levels,
//! and deletion.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use pacs_archive::Archive;
use resources::parents;

mod archive;
mod delete;
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
        .route(
            "/instances/{id}",
            get(resources::instance).delete(delete::instance),
        )
        .route("/instances/{id}/file", get(resources::instance_file))
        .route("/instances/{id}/series", get(parents::instance_series))
        .route("/instances/{id}/study", get(parents::instance_study))
        .route("/instances/{id}/patient", get(parents::instance_patient))
        .route(
            "/series/{id}",
            get(resources::series).delete(delete::series),
        )
        .route("/series/{id}/instances", get(resources::series_instances))
        .route("/series/{id}/study", get(parents::series_study))
        .route("/series/{id}/patient", get(parents::series_patient))
        .route("/studies", get(resources::studies))
        .route("/studies/{id}", get(resources::study).delete(delete::study))
        .route("/studies/{id}/series", get(resources::study_series))
        .route("/studies/{id}/instances", get(resources::study_instances))
        .route("/studies/{id}/statistics", get(resources::study_statistics))
        .route("/studies/{id}/archive", get(archive::study))
        .route("/studies/{id}/patient", get(parents::study_patient))
        .route("/patients", get(resources::patients))
        .route(
            "/patients/{id}",
            get(resources::patient).delete(delete::patient),
        )
        .route("/patients/{id}/studies", get(resources::patient_studies))
        .route("/patients/{id}/series", get(resources::patient_series))
        .route(
            "/patients/{id}/instances",
            get(resources::patient_instances),
        )
        .route(
            "/patients/{id}/statistics",
            get(resources::patient_statistics),
        )
        .route("/patients/{id}/archive", get(archive::patient))
        .route("/tools/find", post(find::find))
        .route(
            "/tools/create-archive",
            get(archive::create_archive_get).post(archive::create_archive),
        )
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(AppState { archive })
}
