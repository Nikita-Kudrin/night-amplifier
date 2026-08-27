//! Camera error types

use std::time::Duration;
use thiserror::Error;

use crate::ffi_safety::FfiError;

/// Camera-specific error types
#[derive(Error, Debug, Clone, PartialEq)]
pub enum CameraError {
    /// No cameras found on the system
    #[error("No cameras found")]
    NoCamerasFound,

    /// Camera index out of range
    #[error("Camera index {index} out of range (found {count} cameras)")]
    InvalidCameraIndex { index: usize, count: usize },

    /// Camera is already open
    #[error("Camera {0} is already open")]
    AlreadyOpen(String),

    /// Camera is not open
    #[error("Camera is not open")]
    NotOpen,

    /// Failed to open camera
    #[error("Failed to open camera: {0}")]
    OpenFailed(String),

    /// Failed to close camera
    #[error("Failed to close camera: {0}")]
    CloseFailed(String),

    /// Camera was disconnected during operation
    #[error("Camera disconnected")]
    Disconnected,

    /// Exposure failed
    #[error("Exposure failed: {0}")]
    ExposureFailed(String),

    /// Exposure timed out
    #[error("Exposure timed out after {0:?}")]
    ExposureTimeout(Duration),

    /// Failed to read image data
    #[error("Failed to read image data: {0}")]
    ImageReadFailed(String),

    /// Invalid parameter value
    #[error("Invalid parameter {name}: {message}")]
    InvalidParameter { name: String, message: String },

    /// Parameter not supported by this camera
    #[error("Parameter {0} not supported by this camera")]
    ParameterNotSupported(String),

    /// Temperature reading failed
    #[error("Failed to read temperature: {0}")]
    TemperatureReadFailed(String),

    /// Cooling control failed
    #[error("Cooling control failed: {0}")]
    CoolingFailed(String),

    /// SDK error with error code
    #[error("SDK error: {message} (code: {code})")]
    SdkError { code: i32, message: String },

    /// SDK not available (feature not enabled)
    #[error("{0} SDK not available. Enable the corresponding feature.")]
    SdkNotAvailable(String),

    /// Buffer allocation failed
    #[error("Failed to allocate buffer of size {0} bytes")]
    BufferAllocationFailed(usize),

    /// Operation was cancelled
    #[error("Operation cancelled")]
    Cancelled,

    /// Provider not found
    #[error("Camera provider '{0}' not found")]
    ProviderNotFound(String),

    /// Provider already registered
    #[error("Camera provider '{0}' is already registered")]
    ProviderAlreadyRegistered(String),

    /// FFI boundary error (panic, null pointer, etc.)
    #[error("FFI error: {0}")]
    FfiBoundaryError(String),

    /// Buffer size mismatch from FFI layer
    #[error("FFI buffer error: expected {expected} bytes, got {actual}")]
    FfiBufferError { expected: usize, actual: usize },
}

impl CameraError {
    /// Whether this error signals that the camera's SDK handle is no longer
    /// valid — typically because the device was physically disconnected or
    /// the USB bus reset. Callers should treat this the same as
    /// `CameraError::Disconnected` (abandon the handle, finalize cleanup).
    pub fn is_sdk_disconnected(&self) -> bool {
        match self {
            CameraError::Disconnected => true,
            CameraError::SdkError { message, .. }
            | CameraError::ExposureFailed(message)
            | CameraError::CoolingFailed(message)
            | CameraError::ImageReadFailed(message)
            | CameraError::ParameterNotSupported(message) => {
                let m = message.to_uppercase();
                m.contains("NOT_OPENED")
                    || m.contains("INVALID_ID")
                    || m.contains("ERROR_CLOSED")
            }
            _ => false,
        }
    }
}

impl From<FfiError> for CameraError {
    fn from(err: FfiError) -> Self {
        match err {
            FfiError::BufferOverflow { expected, actual } => {
                CameraError::FfiBufferError { expected, actual }
            }
            other => CameraError::FfiBoundaryError(other.to_string()),
        }
    }
}

/// Result type for camera operations
pub type CameraResult<T> = std::result::Result<T, CameraError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sdk_disconnected() {
        assert!(CameraError::Disconnected.is_sdk_disconnected());

        let errors_to_test = vec![
            CameraError::ExposureFailed("poa_error_NOT_OPENED".to_string()),
            CameraError::CoolingFailed("ASI_ERROR_INVALID_ID".to_string()),
            CameraError::ImageReadFailed("ERROR_CLOSED returned by SDK".to_string()),
            CameraError::ParameterNotSupported("some NOT_OPENED err".to_string()),
            CameraError::SdkError {
                code: 1,
                message: "Camera NOT_OPENED".to_string(),
            },
        ];

        for e in errors_to_test {
            assert!(e.is_sdk_disconnected(), "Expected {} to be a disconnect error", e);
        }

        let non_disconnect_errors = vec![
            CameraError::ExposureFailed("timeout".to_string()),
            CameraError::CoolingFailed("value out of range".to_string()),
            CameraError::ImageReadFailed("buffer too small".to_string()),
            CameraError::ParameterNotSupported("foo".to_string()),
            CameraError::SdkError {
                code: 2,
                message: "some generic error".to_string(),
            },
            CameraError::ProviderNotFound("foo".to_string()),
            CameraError::NoCamerasFound,
            CameraError::OpenFailed("foo".to_string()),
            CameraError::FfiBoundaryError("panic".to_string()),
        ];

        for e in non_disconnect_errors {
            assert!(!e.is_sdk_disconnected(), "Expected {} to NOT be a disconnect error", e);
        }
    }
}
