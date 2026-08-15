use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::types::CaptureState;
use crate::camera::CameraInfo;

/// Sliding window used by [`CaptureSession::record_rejection`] to detect a
/// *current* burst of camera-capture failures, rather than a lifetime-
/// cumulative count that could trip hours into an otherwise-healthy session.
pub const REJECTION_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Rejections within `REJECTION_RATE_WINDOW` at or above this count indicate
/// the camera is actively failing right now (e.g. truly disconnected), as
/// opposed to an occasional, recoverable hiccup spread across a long session.
pub const REJECTION_RATE_THRESHOLD: usize = 10;

/// Current capture session information
#[derive(Debug, Clone)]
pub struct CaptureSession {
    /// Current state
    pub state: CaptureState,
    /// Number of frames captured
    pub frame_count: u64,
    /// Number of frames successfully stacked
    pub stacked_count: u64,
    /// Number of frames rejected (bad quality, failed alignment, or capture
    /// failure) — a lifetime-of-session stat kept for UI/reporting.
    pub rejected_count: u64,
    /// Timestamps of recent *camera-capture* failures (not stacking-quality
    /// rejections), pruned to the last `REJECTION_RATE_WINDOW`. Used by
    /// `should_stop_on_errors` to detect a current failure burst — independent
    /// of `rejected_count`, which never decays and mixes in stacking rejects.
    pub rejection_timestamps: VecDeque<Instant>,
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
            rejection_timestamps: VecDeque::new(),
            last_error: None,
            started_at: None,
            exposure_us: 1_000_000,
            gain: 0,
        }
    }
}

impl CaptureSession {
    /// Record a camera-capture failure at `now`, prune entries older than
    /// `REJECTION_RATE_WINDOW`, and return whether the rate within the window
    /// has reached `REJECTION_RATE_THRESHOLD`.
    ///
    /// `now` is a parameter rather than calling `Instant::now()` internally so
    /// this is unit-testable without real sleeps.
    pub fn record_rejection(&mut self, now: Instant) -> bool {
        self.rejection_timestamps.push_back(now);
        while self
            .rejection_timestamps
            .front()
            .is_some_and(|t| now.duration_since(*t) > REJECTION_RATE_WINDOW)
        {
            self.rejection_timestamps.pop_front();
        }
        self.rejection_rate_exceeded()
    }

    /// Whether the current rejection rate indicates an active failure burst.
    pub fn rejection_rate_exceeded(&self) -> bool {
        self.rejection_timestamps.len() >= REJECTION_RATE_THRESHOLD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_rejection_trips_on_burst_within_window() {
        let mut session = CaptureSession::default();
        let base = Instant::now();
        let mut tripped = false;
        for i in 0..REJECTION_RATE_THRESHOLD {
            tripped = session.record_rejection(base + Duration::from_millis(i as u64 * 10));
        }
        assert!(tripped, "threshold rejections within the window should trip");
    }

    #[test]
    fn record_rejection_does_not_trip_when_spread_out() {
        let mut session = CaptureSession::default();
        let base = Instant::now();
        let mut tripped = false;
        // One every 2s — by the time the Nth lands, everything before the
        // window start has already been pruned, so the count never reaches
        // the threshold no matter how many we record.
        for i in 0..(REJECTION_RATE_THRESHOLD * 3) {
            tripped = session.record_rejection(base + Duration::from_secs(i as u64 * 2));
        }
        assert!(!tripped, "rejections spread beyond the window should not trip");
    }

    #[test]
    fn record_rejection_prunes_stale_entries() {
        let mut session = CaptureSession::default();
        let base = Instant::now();
        for i in 0..5u64 {
            session.record_rejection(base + Duration::from_millis(i * 10));
        }
        assert_eq!(session.rejection_timestamps.len(), 5);

        // A gap longer than the window — the next record should prune every
        // prior entry, leaving only itself.
        session.record_rejection(base + REJECTION_RATE_WINDOW + Duration::from_secs(1));
        assert_eq!(session.rejection_timestamps.len(), 1);
    }

    #[test]
    fn rejection_rate_exceeded_matches_record_rejection_return() {
        let mut session = CaptureSession::default();
        let base = Instant::now();
        for i in 0..(REJECTION_RATE_THRESHOLD - 1) {
            session.record_rejection(base + Duration::from_millis(i as u64));
        }
        assert!(!session.rejection_rate_exceeded());

        let tripped = session.record_rejection(base + Duration::from_millis(REJECTION_RATE_THRESHOLD as u64));
        assert!(tripped);
        assert!(session.rejection_rate_exceeded());
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
