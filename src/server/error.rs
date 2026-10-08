//! Server error types, and how a session's [`ApiError`] answers over HTTP.

use axum::http::StatusCode;
use thiserror::Error;

pub use crate::session::error::{ApiError, ApiResult};

/// Server-level errors (startup, binding, etc.)
#[derive(Debug, Clone, Error)]
pub enum ServerError {
    #[error("Failed to bind server: {0}")]
    BindFailed(String),

    #[error("Server error: {0}")]
    ServeFailed(String),
}

/// How a session's [`ApiError`] answers over HTTP. A trait only because `ApiError` lives in
/// the session crate.
///
/// Handlers that build their own response body delegate the status here. They used to keep
/// parallel `match` arms instead, and those drifted: `start_capture` had no arm for a role
/// mismatch, so a conflict the client could act on went out as a 500.
pub(crate) trait HttpStatus {
    fn status_code(&self) -> StatusCode;
}

impl HttpStatus for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            ApiError::NoCameraSelected => StatusCode::BAD_REQUEST,
            ApiError::CameraNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::CameraNotConnected(_) => StatusCode::NOT_FOUND,
            ApiError::CaptureInProgress => StatusCode::CONFLICT,
            ApiError::CaptureNotPaused => StatusCode::CONFLICT,
            ApiError::CameraRoleBusy { .. } => StatusCode::CONFLICT,
            ApiError::CameraRoleMismatch { .. } => StatusCode::CONFLICT,
            ApiError::CaptureCameraIsNotMain { .. } => StatusCode::CONFLICT,
            ApiError::NoGuideCameraConnected => StatusCode::BAD_REQUEST,
            ApiError::StackingTypeChangeNotAllowed => StatusCode::CONFLICT,
            ApiError::FocusModeWhileStacking => StatusCode::CONFLICT,
            ApiError::ProFeatureRequired(_) => StatusCode::FORBIDDEN,
            ApiError::InvalidCameraIdFormat => StatusCode::BAD_REQUEST,
            ApiError::InvalidCameraIndex => StatusCode::BAD_REQUEST,
            ApiError::CameraOpenFailed(_) => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::CameraIdentityMismatch { .. } => StatusCode::CONFLICT,
            ApiError::CameraRecovering { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::CameraHandleLost { .. } => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::SimulatorConfigFailed(_) => StatusCode::BAD_REQUEST,
            ApiError::HardwareBenchmarkRunning => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::BenchmarkDuringCapture => StatusCode::CONFLICT,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

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
}
