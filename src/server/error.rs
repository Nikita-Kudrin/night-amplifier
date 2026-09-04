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
    #[error("No imaging camera is connected. Connect one before starting a capture.")]
    NoCameraSelected,

    #[error("Camera '{0}' not found")]
    CameraNotFound(String),

    #[error("Camera '{0}' not connected")]
    CameraNotConnected(String),

    #[error("Capture already in progress")]
    CaptureInProgress,

    #[error("Cannot disconnect camera while capturing")]
    CameraInUse,

    #[error("The {role} camera slot is taken by '{camera}', which is busy. Stop it first.")]
    CameraRoleBusy {
        role: &'static str,
        camera: String,
    },

    #[error("'{camera}' is already connected as the {held} camera; disconnect it before connecting it as the {requested} camera")]
    CameraRoleMismatch {
        camera: String,
        held: &'static str,
        requested: &'static str,
    },

    /// Asked to capture with a camera that holds some other role.
    ///
    /// Separate from [`ApiError::CameraRoleMismatch`] because the remedy is different
    /// and so is the request: that one answers a *connect*, and telling someone who
    /// pressed Start to "disconnect it before connecting it" describes an action they
    /// did not take and does not want.
    #[error("'{camera}' is the {held} camera; captures run on the imaging camera")]
    CaptureCameraIsNotMain {
        camera: String,
        held: &'static str,
    },

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
    /// The HTTP status this error maps to.
    ///
    /// `pub(crate)` so handlers that build their own response body can still delegate
    /// the status here. They used to keep parallel `match` arms instead, and those
    /// drifted: `start_capture` had no arm for a role mismatch, so a conflict the
    /// client could act on went out as a 500.
    pub(crate) fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NoCameraSelected => StatusCode::BAD_REQUEST,
            ApiError::CameraNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::CameraNotConnected(_) => StatusCode::NOT_FOUND,
            ApiError::CaptureInProgress => StatusCode::CONFLICT,
            ApiError::CameraInUse => StatusCode::CONFLICT,
            ApiError::CameraRoleBusy { .. } => StatusCode::CONFLICT,
            ApiError::CameraRoleMismatch { .. } => StatusCode::CONFLICT,
            ApiError::CaptureCameraIsNotMain { .. } => StatusCode::CONFLICT,
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
        // A role conflict is something the client can act on. It used to leave
        // `start_capture` as a 500, because that handler kept its own `match` with no
        // arm for it — which is why handlers now delegate here instead.
        assert_eq!(
            ApiError::CaptureCameraIsNotMain {
                camera: "Guiding".into(),
                held: "guide",
            }
            .status_code(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            ApiError::CameraRoleMismatch {
                camera: "Guiding".into(),
                held: "guide",
                requested: "main",
            }
            .status_code(),
            StatusCode::CONFLICT
        );
    }

    #[test]
    fn test_api_error_messages() {
        assert_eq!(
            ApiError::NoCameraSelected.to_string(),
            "No imaging camera is connected. Connect one before starting a capture."
        );
        assert_eq!(
            ApiError::CaptureCameraIsNotMain {
                camera: "Simulator: 35mm-imx464-orion-tiff (17 files)".into(),
                held: "guide",
            }
            .to_string(),
            "'Simulator: 35mm-imx464-orion-tiff (17 files)' is the guide camera; \
             captures run on the imaging camera"
        );
        assert_eq!(
            ApiError::CameraNotFound("cam1".into()).to_string(),
            "Camera 'cam1' not found"
        );
    }
}
