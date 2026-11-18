//! Errors as the API reports them.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use serde_json::Value;

/// An error answered to the client: an HTTP status, a stable code, a message for people and
/// optional details (validation issues, the explanation of a refusal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct ApiError {
    /// HTTP status.
    #[serde(skip)]
    pub status: u16,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// What went wrong, in plain English.
    pub message: String,
    /// More information (validation issues, a permission explanation).
    #[serde(skip_serializing_if = "Value::is_null")]
    pub details: Value,
}

impl ApiError {
    fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            details: Value::Null,
        }
    }

    /// 400: the request is malformed.
    pub fn bad_request(message: impl Into<String>) -> Self {
        ApiError::new(400, "bad_request", message)
    }

    /// 403: the actor may not do this; `details` carries the explanation.
    pub fn forbidden(message: impl Into<String>, details: Value) -> Self {
        ApiError {
            details,
            ..ApiError::new(403, "forbidden", message)
        }
    }

    /// 404: there is no such thing.
    pub fn not_found(what: impl Into<String>) -> Self {
        ApiError::new(404, "not_found", format!("{} does not exist", what.into()))
    }

    /// 409: the change conflicts with the current state.
    pub fn conflict(message: impl Into<String>) -> Self {
        ApiError::new(409, "conflict", message)
    }

    /// 422: the values are not acceptable; `details` lists the issues.
    pub fn invalid(message: impl Into<String>, details: Value) -> Self {
        ApiError {
            details,
            ..ApiError::new(422, "invalid", message)
        }
    }

    /// 501: the feature is planned but not built yet.
    pub fn not_yet(message: impl Into<String>) -> Self {
        ApiError::new(501, "not_yet_available", message)
    }

    /// 500: something failed inside the server.
    pub fn internal(message: impl Into<String>) -> Self {
        ApiError::new(500, "internal", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

/// The result of an API operation.
pub type ApiResult<T> = Result<T, ApiError>;
