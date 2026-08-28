//! Camera error types

use std::time::Duration;
use thiserror::Error;

use crate::camera::device_lost;
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

    /// The SDK answered, but its error code says this handle's device is gone
    /// — unplugged, USB reset, or closed underneath us. Distinct from
    /// `Disconnected`, which is the server's own verdict rather than the
    /// vendor's.
    #[error("Camera device lost: {0}")]
    DeviceLost(String),

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
    /// Whether this error means the camera's SDK handle no longer refers to a
    /// live device — unplugged, USB bus reset, or closed underneath us.
    /// Callers must abandon the handle rather than retry on it.
    ///
    /// Two sources, and deliberately no third: the typed variants that say so
    /// outright, and the [`device_lost`] marker each shim attaches after
    /// classifying its own vendor code. Nothing here matches vendor
    /// vocabulary — that approach only ever worked for PlayerOne, because it
    /// is the one provider that renders its error enum symbolically.
    pub fn is_sdk_disconnected(&self) -> bool {
        match self {
            CameraError::Disconnected | CameraError::NotOpen | CameraError::DeviceLost(_) => true,
            CameraError::SdkError { message, .. }
            | CameraError::ExposureFailed(message)
            | CameraError::CoolingFailed(message)
            | CameraError::ImageReadFailed(message)
            | CameraError::TemperatureReadFailed(message)
            | CameraError::ParameterNotSupported(message)
            | CameraError::InvalidParameter { message, .. }
            | CameraError::OpenFailed(message)
            | CameraError::CloseFailed(message)
            | CameraError::FfiBoundaryError(message) => device_lost::is_marked(message),
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
    use crate::camera::device_lost::mark;

    #[test]
    fn typed_variants_are_disconnects_without_a_message() {
        assert!(CameraError::Disconnected.is_sdk_disconnected());
        assert!(CameraError::NotOpen.is_sdk_disconnected());
    }

    /// One case per provider, using the exact text that provider's shim now
    /// produces. ZWO, SVBony and ToupTek used to render a bare number, which
    /// is why none of them were ever classified.
    #[test]
    fn every_provider_marks_its_own_device_loss_codes() {
        let cases = vec![
            CameraError::ExposureFailed(mark("POAImageReady failed: POA_ERROR_NOT_OPENED")),
            CameraError::CoolingFailed(mark("POASetConfig failed: POA_ERROR_DEVICE_NOT_FOUND")),
            CameraError::SdkError {
                code: -1,
                message: format!(
                    "Failed to set exposure: {}",
                    mark("POASetConfig failed: POA_ERROR_NOT_OPENED")
                ),
            },
            CameraError::ImageReadFailed(mark(
                "ASIGetDataAfterExp failed: ASI_ERROR_CAMERA_REMOVED (5)",
            )),
            CameraError::CoolingFailed(mark(
                "ASISetControlValue failed: ASI_ERROR_CAMERA_CLOSED (4)",
            )),
            CameraError::ExposureFailed(mark(
                "SVBGetVideoData failed: SVB_ERROR_CAMERA_REMOVED (5)",
            )),
            CameraError::SdkError {
                code: 0,
                message: mark("Toupcam_put_Option(0x04) failed: HRESULT 0x8007001F"),
            },
            CameraError::ParameterNotSupported(format!(
                "dew_heater: {}",
                mark("POASetConfig failed: POA_ERROR_NOT_OPENED")
            )),
        ];
        for e in cases {
            assert!(e.is_sdk_disconnected(), "{} should be a disconnect", e);
        }
    }

    /// The regression this replaced: an unsupported parameter, a value out of
    /// range and an ordinary timeout are not device loss. The old matcher
    /// keyed on `ParameterNotSupported` as a variant, so any camera without a
    /// dew heater looked disconnected.
    #[test]
    fn ordinary_failures_are_not_disconnects() {
        let cases = vec![
            CameraError::ExposureFailed("timeout".to_string()),
            CameraError::CoolingFailed("POASetConfig failed: POA_ERROR_OUT_OF_LIMIT".to_string()),
            CameraError::ImageReadFailed(
                "ASIGetVideoData failed: ASI_ERROR_TIMEOUT (11)".to_string(),
            ),
            CameraError::ParameterNotSupported("dew_heater".to_string()),
            CameraError::SdkError {
                code: 2,
                message: "some generic error".to_string(),
            },
            CameraError::ProviderNotFound("foo".to_string()),
            CameraError::NoCamerasFound,
            CameraError::OpenFailed("SDK not loaded".to_string()),
            CameraError::FfiBoundaryError("panic".to_string()),
            CameraError::ExposureTimeout(Duration::from_secs(1)),
            CameraError::Cancelled,
        ];
        for e in cases {
            assert!(!e.is_sdk_disconnected(), "{} should NOT be a disconnect", e);
        }
    }
}
