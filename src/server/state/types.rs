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
