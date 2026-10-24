use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

/// An error answer: a status and a message, sent as `{"error": message}`.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    pub fn not_acceptable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_ACCEPTABLE, message)
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

impl From<pacs_index::Error> for ApiError {
    fn from(e: pacs_index::Error) -> Self {
        ApiError::internal(e)
    }
}

impl From<pacs_store::Error> for ApiError {
    /// A missing file for an indexed instance is the server's fault, not the client's.
    fn from(e: pacs_store::Error) -> Self {
        ApiError::internal(e)
    }
}
