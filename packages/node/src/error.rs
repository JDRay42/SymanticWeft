//! Application-level error type returned by handlers.
//!
//! All variants serialise to the spec's [`ErrorResponse`] JSON format and
//! map to the appropriate HTTP status code.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use semanticweft_node_api::ErrorResponse;

use crate::storage::StorageError;

/// An error that a handler can return; converts directly to an HTTP response.
#[derive(Debug)]
pub enum AppError {
    /// The requested resource does not exist; serialises to `"not_found"` with HTTP 404.
    NotFound(String),
    /// The request is malformed or contains invalid parameters; serialises to `"invalid_parameter"` with HTTP 400.
    BadRequest(String),
    /// A resource with the given identifier already exists; serialises to `"id_conflict"` with HTTP 409.
    Conflict(String),
    /// The request is syntactically valid but fails semantic validation; serialises to `"validation_failed"` with HTTP 422.
    UnprocessableEntity(String),
    /// An unexpected server-side failure occurred; serialises to `"internal_error"` with HTTP 500.
    Internal(String),
    /// The caller is authenticated but lacks permission for the operation; serialises to `"forbidden"` with HTTP 403.
    Forbidden(String),
    /// The operation requires authentication that was not provided or is invalid; serialises to `"unauthorized"` with HTTP 401.
    Unauthorized(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "invalid_parameter", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "id_conflict", msg),
            AppError::UnprocessableEntity(msg) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "validation_failed", msg)
            }
            AppError::Internal(msg) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg)
            }
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg),
            AppError::Unauthorized(msg) => (StatusCode::UNAUTHORIZED, "unauthorized", msg),
        };
        let body = ErrorResponse::new(code, message);
        (status, Json(body)).into_response()
    }
}

impl From<StorageError> for AppError {
    fn from(e: StorageError) -> Self {
        match e {
            StorageError::NotFound => AppError::NotFound("not found".into()),
            StorageError::Conflict(msg) => AppError::Conflict(msg),
            StorageError::Internal(msg) => AppError::Internal(msg),
        }
    }
}
