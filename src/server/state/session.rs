use super::types::CaptureState;
use crate::camera::CameraInfo;

/// Current capture session information
#[derive(Debug, Clone)]
pub struct CaptureSession {
    /// Current state
    pub state: CaptureState,
    /// Number of frames captured
    pub frame_count: u64,
    /// Number of frames successfully stacked
    pub stacked_count: u64,
    /// Number of frames rejected (bad quality, failed alignment)
    pub rejected_count: u64,
    /// Last error message (if any)
    pub last_error: Option<String>,
    /// Capture start time (Unix timestamp ms)
    pub started_at: Option<u64>,
    /// Current exposure time in microseconds
    pub exposure_us: u64,
    /// Current gain
    pub gain: i32,
}

impl Default for CaptureSession {
    fn default() -> Self {
        Self {
            state: CaptureState::Idle,
            frame_count: 0,
            stacked_count: 0,
            rejected_count: 0,
            last_error: None,
            started_at: None,
            exposure_us: 1_000_000,
            gain: 0,
        }
    }
}

/// Connected camera information
#[derive(Debug, Clone)]
pub struct ConnectedCameraInfo {
    /// Camera ID
    pub id: String,
    /// Provider name
    pub provider: String,
    /// Provider index
    pub index: usize,
    /// Camera info
    pub info: CameraInfo,
}
