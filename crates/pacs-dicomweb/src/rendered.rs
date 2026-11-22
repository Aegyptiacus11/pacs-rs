//! Rendered resources (PS3.18 9.5, 8.3.5.1): an instance's frame as a picture, JPEG or PNG,
//! for clients that do not read DICOM: windowed as asked (`window`), or as the file says,
//! cropped and scaled to a `viewport`, of a JPEG `quality`.

use axum::extract::{Path, Query};
use axum::http::HeaderMap;
use axum::http::header::{ACCEPT, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use pacs_core::Instance;
use pacs_core::render::{self, RenderError, Rendering, Viewport, Window};

use crate::media::{Accept, rendered_type};
use crate::schema::{Binary, Problem};
use crate::scope::Seen;
use crate::{ApiError, AppState};

/// JPEG quality when none is asked for.
const QUALITY: u8 = 90;

type Params = Query<Vec<(String, String)>>;

/// The rendering the query asks for. `annotation` is taken and not done: no burned-in
/// text is added (PS3.18 8.3.5.1.1 lets a server leave it out).
fn rendering(params: &[(String, String)]) -> Result<Rendering, ApiError> {
    let mut rendering = Rendering::default();
    for (name, value) in params {
        match name.as_str() {
            "window" => {
                rendering.window = Some(Window::parse(value).map_err(ApiError::bad_request)?)
            }
            "viewport" => {
                rendering.viewport = Some(Viewport::parse(value).map_err(ApiError::bad_request)?)
            }
            "quality" => {
                rendering.quality = Some(
                    value
                        .parse::<u8>()
                        .ok()
                        .filter(|q| (1..=100).contains(q))
                        .ok_or_else(|| {
                            ApiError::bad_request(format!(
                                "quality is from 1 to 100, not {value:?}"
                            ))
                        })?,
                )
            }
            "annotation" => {}
            other => {
                return Err(ApiError::bad_request(format!(
                    "{other} is not a parameter of a rendered resource (window, viewport, quality, annotation)"
                )));
            }
        }
    }
    Ok(rendering)
}

async fn rendered(
    state: AppState,
    headers: &HeaderMap,
    (study, series, instance): (&str, &str, &str),
    frame: u32,
    params: &[(String, String)],
) -> Result<Response, ApiError> {
    let rendering = rendering(params)?;
    let accept = Accept::parse(headers.get(ACCEPT).and_then(|h| h.to_str().ok()));
    let media_type = rendered_type(&accept).ok_or_else(|| {
        ApiError::not_acceptable("a rendered resource is image/jpeg or image/png")
    })?;
    let entry = state
        .archive
        .index()
        .instance(study, series, instance)
        .await?
        .ok_or_else(|| ApiError::not_found("no such instance"))?;
    let file = state.archive.file(&entry).await?;
    let reading = state.archive.admit_stored(&file).await;
    let picture = tokio::task::spawn_blocking(move || {
        let _reading = reading;
        let instance = Instance::from_bytes(&file).map_err(ApiError::internal)?;
        let frames = instance.number_of_frames();
        if frame == 0 || frame > frames {
            return Err(ApiError::not_found(format!(
                "frame {frame} of an instance of {frames}"
            )));
        }
        let picture = render::render(&instance, frame, &rendering).map_err(|e| match e {
            RenderError::Encode(_) => ApiError::internal(e),
            // what cannot be rendered of this instance, or asked of its frame
            _ => ApiError::new(axum::http::StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        })?;
        match media_type {
            "image/png" => render::png(&picture),
            _ => render::jpeg(&picture, rendering.quality.unwrap_or(QUALITY)),
        }
        .map_err(ApiError::internal)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(([(CONTENT_TYPE, media_type)], picture).into_response())
}

/// Renders an instance (WADO-RS).
///
/// Its first frame as a picture: monochrome through its rescale and a window (the one asked for, else the file's first, else its range of values), colour as RGB.
#[utoipa::path(
    get,
    path = "/studies/{study}/series/{series}/instances/{instance}/rendered",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID."), ("series" = String, Path, description = "Series Instance UID."), ("instance" = String, Path, description = "SOP Instance UID."), ("window" = Option<String>, Query, description = "`center,width,function`, the function `linear`, `linear-exact` or `sigmoid`."), ("viewport" = Option<String>, Query, description = "`vw,vh`, or `vw,vh,sx,sy,sw,sh` with a region of the frame: the picture fits the viewport, its proportions kept."), ("quality" = Option<u8>, Query, description = "JPEG quality, 1 to 100 (90).")),
    responses(
        (status = 200, description = "The picture.", body = Binary, content_type = "image/jpeg"),
        (status = 400, description = "A parameter that is not one, or not a valid value.", body = Problem),
        (status = 404, description = "No such instance.", body = Problem),
        (status = 406, description = "`Accept` takes neither JPEG nor PNG.", body = Problem),
        (status = 422, description = "An instance this server does not render (no pixel data, a photometric interpretation it does not take), or a viewport region past the frame.", body = Problem)
    )
)]
pub(crate) async fn instance(
    Seen(state): Seen,
    headers: HeaderMap,
    Path((study, series, instance)): Path<(String, String, String)>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    rendered(state, &headers, (&study, &series, &instance), 1, &params).await
}

/// Renders a frame of an instance (WADO-RS).
///
/// One frame, by its number from 1, as a picture: see the instance's.
#[utoipa::path(
    get,
    path = "/studies/{study}/series/{series}/instances/{instance}/frames/{frame}/rendered",
    tag = "DICOMweb",
    params(("study" = String, Path, description = "Study Instance UID."), ("series" = String, Path, description = "Series Instance UID."), ("instance" = String, Path, description = "SOP Instance UID."), ("frame" = String, Path, description = "One frame number, from 1."), ("window" = Option<String>, Query, description = "`center,width,function`."), ("viewport" = Option<String>, Query, description = "`vw,vh` or `vw,vh,sx,sy,sw,sh`."), ("quality" = Option<u8>, Query, description = "JPEG quality, 1 to 100 (90).")),
    responses(
        (status = 200, description = "The picture.", body = Binary, content_type = "image/jpeg"),
        (status = 400, description = "Not one frame number, or a parameter that is not valid.", body = Problem),
        (status = 404, description = "No such instance, or a frame past the last.", body = Problem),
        (status = 406, description = "`Accept` takes neither JPEG nor PNG.", body = Problem),
        (status = 422, description = "An instance this server does not render, or a viewport region past the frame.", body = Problem)
    )
)]
pub(crate) async fn frame(
    Seen(state): Seen,
    headers: HeaderMap,
    Path((study, series, instance, frame)): Path<(String, String, String, String)>,
    Query(params): Params,
) -> Result<Response, ApiError> {
    let number = frame
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "a rendered frame is one frame number from 1, not {frame:?}"
            ))
        })?;
    rendered(
        state,
        &headers,
        (&study, &series, &instance),
        number,
        &params,
    )
    .await
}
