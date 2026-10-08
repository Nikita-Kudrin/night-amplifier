//! The error a session use case reports. Its HTTP status and response body are the
//! server's business: see `server::error`.

use thiserror::Error;

/// What a session use case refuses or fails with. `server::error` maps it to HTTP.
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

    /// A resume found no capture paused for recovery — it was stopped or disconnected.
    #[error("There is no paused capture to resume")]
    CaptureNotPaused,

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

    #[error("No guide camera is connected. Connect one before starting it.")]
    NoGuideCameraConnected,

    #[error("Cannot change stacking type while capturing")]
    StackingTypeChangeNotAllowed,

    /// Entering the mode drops the banding correction from frames a running stack keeps.
    #[error("Cannot enter Focus/Finder mode while stacking - it would mix hot pixels and banding into the stack. Stop the capture first.")]
    FocusModeWhileStacking,

    #[error("{0} is a Pro feature")]
    ProFeatureRequired(&'static str),

    #[error("Invalid camera ID format. Expected: provider_index")]
    InvalidCameraIdFormat,

    #[error("Invalid camera index")]
    InvalidCameraIndex,

    #[error("Failed to open camera: {0}")]
    CameraOpenFailed(String),

    /// The device that opened is not the camera the id or the recovery named — the USB
    /// list reordered between enumerating it and opening it.
    #[error("Opened '{found}' where '{expected}' was expected; the camera list changed")]
    CameraIdentityMismatch { expected: String, found: String },

    /// The camera's handle was lost and recovery is reopening it; there is nothing to
    /// hand out until it has.
    #[error("'{camera}' is being reconnected")]
    CameraRecovering { camera: String },

    /// A hand-off found the handle gone with nothing holding it. The caller hands back
    /// `None` like any capture that lost its handle; `return_from_capture` recovers it.
    #[error("'{camera}' lost its camera handle")]
    CameraHandleLost { camera: String },

    #[error("Failed to configure simulator: {0}")]
    SimulatorConfigFailed(String),

    /// The AI compute benchmark is checking or measuring the hardware; captures wait.
    #[error("Benchmarking hardware for AI denoising; start the capture when it finishes")]
    HardwareBenchmarkRunning,

    /// Measure again would compete with the running capture for the CPU.
    #[error("Stop the capture to measure the hardware again")]
    BenchmarkDuringCapture,

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type for API handlers
pub type ApiResult<T> = Result<T, ApiError>;

#[cfg(test)]
mod tests {
    use super::*;

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
