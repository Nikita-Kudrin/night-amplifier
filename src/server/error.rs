//! Server error types
//!
//! Centralized error handling for the server module using thiserror.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

/// API error response body
#[derive(Debug, Serialize)]
struct ErrorResponse {
    success: bool,
    error: String,
}

/// Server-level errors (startup, binding, etc.)
#[derive(Debug, Clone, Error)]
pub enum ServerError {
    #[error("Failed to bind server: {0}")]
    BindFailed(String),

    #[error("Server error: {0}")]
    ServeFailed(String),
}

/// API-level errors returned from endpoint handlers
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("No camera selected")]
    NoCameraSelected,

    #[error("Camera '{0}' not found")]
    CameraNotFound(String),

    #[error("Camera '{0}' not connected")]
    CameraNotConnected(String),

    #[error("Capture already in progress")]
    CaptureInProgress,

    #[error("Cannot disconnect camera while capturing")]
    CameraInUse,

    #[error("Cannot change stacking type while capturing")]
    StackingTypeChangeNotAllowed,

    #[error("Invalid camera ID format. Expected: provider_index")]
    InvalidCameraIdFormat,

    #[error("Invalid camera index")]
    InvalidCameraIndex,

    #[error("Failed to open camera: {0}")]
    CameraOpenFailed(String),

    #[error("Failed to configure simulator: {0}")]
    SimulatorConfigFailed(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NoCameraSelected => StatusCode::BAD_REQUEST,
            ApiError::CameraNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::CameraNotConnected(_) => StatusCode::NOT_FOUND,
            ApiError::CaptureInProgress => StatusCode::CONFLICT,
            ApiError::CameraInUse => StatusCode::CONFLICT,
            ApiError::StackingTypeChangeNotAllowed => StatusCode::CONFLICT,
            ApiError::InvalidCameraIdFormat => StatusCode::BAD_REQUEST,
            ApiError::InvalidCameraIndex => StatusCode::BAD_REQUEST,
            ApiError::CameraOpenFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::SimulatorConfigFailed(_) => StatusCode::BAD_REQUEST,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let body = ErrorResponse {
            success: false,
            error: self.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_status_codes() {
        assert_eq!(
            ApiError::NoCameraSelected.status_code(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            ApiError::CameraNotFound("x".into()).status_code(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            ApiError::CaptureInProgress.status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::Internal("x".into()).status_code(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_api_error_messages() {
        assert_eq!(ApiError::NoCameraSelected.to_string(), "No camera selected");
        assert_eq!(
            ApiError::CameraNotFound("cam1".into()).to_string(),
            "Camera 'cam1' not found"
        );
    }
}
