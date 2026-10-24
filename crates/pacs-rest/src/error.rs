use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// An error answer. Unknown resources are a bare 404, as the reference server answers;
/// other errors carry `HttpError`, `HttpStatus`, `Message` and `Details`.
#[derive(Debug)]
pub enum RestError {
    NotFound,
    Status {
        status: StatusCode,
        message: String,
        details: Option<String>,
    },
}

impl RestError {
    pub fn bad_request(message: impl Into<String>, details: impl Into<String>) -> Self {
        RestError::Status {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            details: Some(details.into()),
        }
    }

    pub fn internal(error: impl std::fmt::Display) -> Self {
        RestError::Status {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "Internal error".into(),
            details: Some(error.to_string()),
        }
    }
}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        match self {
            RestError::NotFound => StatusCode::NOT_FOUND.into_response(),
            RestError::Status {
                status,
                message,
                details,
            } => {
                let mut body = json!({
                    "HttpError": status.canonical_reason().unwrap_or(""),
                    "HttpStatus": status.as_u16(),
                    "Message": message,
                });
                if let Some(details) = details {
                    body["Details"] = details.into();
                }
                (status, Json(body)).into_response()
            }
        }
    }
}

impl From<pacs_index::Error> for RestError {
    fn from(e: pacs_index::Error) -> Self {
        RestError::internal(e)
    }
}

impl From<pacs_store::Error> for RestError {
    fn from(e: pacs_store::Error) -> Self {
        RestError::internal(e)
    }
}
