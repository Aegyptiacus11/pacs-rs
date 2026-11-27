//! The pacs-rs REST API: patients, studies, series and instances addressed by public ID
//! (`pacs_core::public_ids`), uploads, deletion, search and archives.
//!
//! Resources, identifiers and answers follow the reference server's REST API, so clients
//! written for it work unchanged; `crates/pacs-server/tests/reference.rs` checks that.
//! The routes so far are those an annotation platform needs (upload, lookup, statistics,
//! archives), their counterparts at the other levels, deletion, labels and metadata.

use axum::Router;
use axum::extract::DefaultBodyLimit;
use pacs_archive::Archive;
use resources::parents;
use utoipa::openapi::OpenApi;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

mod archive;
mod delete;
mod documents;
mod error;
mod find;
mod labels;
mod metadata;
mod resources;
pub mod schema;
mod scope;
mod upload;

pub use error::RestError;

#[derive(Debug, Clone)]
struct AppState {
    archive: Archive,
}

/// The REST API at `/`; `max_upload_bytes` bounds `POST /instances`.
pub fn router(archive: Archive, max_upload_bytes: usize) -> Router {
    router_with_api(archive, max_upload_bytes).0
}

/// The REST API and its description. A handler is routed by its `#[utoipa::path]`, so no
/// route is served that the description does not have, and none is described that is not
/// served.
pub fn router_with_api(archive: Archive, max_upload_bytes: usize) -> (Router, OpenApi) {
    described()
        .layer(DefaultBodyLimit::max(max_upload_bytes))
        .with_state(scope::Whole::new(AppState { archive }))
        .split_for_parts()
}

/// The description alone, for writing the reference without an archive.
pub fn api() -> OpenApi {
    described().into_openapi()
}

fn described() -> OpenApiRouter<scope::Whole> {
    OpenApiRouter::new()
        .routes(routes!(upload::instance))
        .routes(routes!(resources::instance, delete::instance))
        .routes(routes!(resources::instance_file))
        .routes(routes!(documents::pdf))
        .routes(routes!(documents::add))
        .routes(routes!(parents::instance_series))
        .routes(routes!(parents::instance_study))
        .routes(routes!(parents::instance_patient))
        .routes(routes!(resources::series, delete::series))
        .routes(routes!(resources::series_instances))
        .routes(routes!(parents::series_study))
        .routes(routes!(parents::series_patient))
        .routes(routes!(resources::studies))
        .routes(routes!(resources::study, delete::study))
        .routes(routes!(resources::study_series))
        .routes(routes!(resources::study_instances))
        .routes(routes!(resources::study_statistics))
        .routes(routes!(archive::study))
        .routes(routes!(parents::study_patient))
        .routes(routes!(resources::patients))
        .routes(routes!(resources::patient, delete::patient))
        .routes(routes!(resources::patient_studies))
        .routes(routes!(resources::patient_series))
        .routes(routes!(resources::patient_instances))
        .routes(routes!(resources::patient_statistics))
        .routes(routes!(archive::patient))
        .routes(routes!(find::find))
        .routes(routes!(metadata::patients::metadata))
        .routes(routes!(
            metadata::patients::value,
            metadata::patients::put_value,
            metadata::patients::delete_value
        ))
        .routes(routes!(metadata::studies::metadata))
        .routes(routes!(
            metadata::studies::value,
            metadata::studies::put_value,
            metadata::studies::delete_value
        ))
        .routes(routes!(metadata::series::metadata))
        .routes(routes!(
            metadata::series::value,
            metadata::series::put_value,
            metadata::series::delete_value
        ))
        .routes(routes!(metadata::instances::metadata))
        .routes(routes!(
            metadata::instances::value,
            metadata::instances::put_value,
            metadata::instances::delete_value
        ))
        .routes(routes!(labels::all))
        .routes(routes!(labels::patients::labels))
        .routes(routes!(
            labels::patients::label,
            labels::patients::put_label,
            labels::patients::delete_label
        ))
        .routes(routes!(labels::studies::labels))
        .routes(routes!(
            labels::studies::label,
            labels::studies::put_label,
            labels::studies::delete_label
        ))
        .routes(routes!(labels::series::labels))
        .routes(routes!(
            labels::series::label,
            labels::series::put_label,
            labels::series::delete_label
        ))
        .routes(routes!(labels::instances::labels))
        .routes(routes!(
            labels::instances::label,
            labels::instances::put_label,
            labels::instances::delete_label
        ))
        .routes(routes!(
            archive::create_archive_get,
            archive::create_archive
        ))
}
