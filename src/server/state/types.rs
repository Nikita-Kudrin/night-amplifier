use crate::stacking::{StackingType, WeightingPreset};

/// Capture session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureState {
    /// No capture in progress
    #[default]
    Idle,
    /// Capture is starting up
    Starting,
    /// Actively capturing frames
    Capturing,
    /// Capture is stopping
    Stopping,
    /// Capture encountered an error
    Error,
}

/// Lifecycle phase of a connected camera handle.
///
/// Orthogonal to `CaptureState`: a camera can be `WarmingUp` while the
/// capture session is `Idle` (the user disconnected but TEC must ramp down
/// before the USB handle is closed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraPhase {
    /// No handle open; camera not connected.
    #[default]
    Disconnected,
    /// Handle open, cooler off or not supported — ambient.
    Idle,
    /// Handle open, cooler driving toward `target_temp_c`.
    Precooling,
    /// Handle currently owned by the capture thread.
    Capturing,
    /// Handle open, cooler ramping off before release.
    WarmingUp,
}
