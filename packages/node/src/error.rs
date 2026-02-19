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
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    UnprocessableEntity(String),
    Internal(String),
    Forbidden(String),
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
