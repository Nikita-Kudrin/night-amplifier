use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::settings::CaptureSettings;
use crate::camera::CameraInfo;

/// Sliding window used by [`SessionStats::record_failure`] to detect a
/// *current* burst of camera-capture failures, rather than a lifetime-
/// cumulative count that could trip hours into an otherwise-healthy session.
pub const REJECTION_RATE_WINDOW: Duration = Duration::from_secs(1);
/// Rejections within `REJECTION_RATE_WINDOW` at or above this count indicate
/// the camera is actively failing right now (e.g. truly disconnected), as
/// opposed to an occasional, recoverable hiccup spread across a long session.
pub const REJECTION_RATE_THRESHOLD: usize = 10;

/// The capture session's frame counters.
///
/// Atomics, so the capture and stacking threads count without a lock or a `block_on`;
/// only the failure-burst window sits behind a mutex, and it is touched on failures alone.
/// The counters move independently, so a reader racing a frame can see one counter a frame
/// ahead of another — they only ever feed the UI's tallies.
#[derive(Debug, Default)]
pub struct SessionStats {
    frames: AtomicU64,
    stacked: AtomicU64,
    /// Bad quality, failed alignment or a capture failure, counted only while stacking — a
    /// lifetime-of-session tally for the UI, never decaying.
    rejected: AtomicU64,
    /// Frames the camera handed the pipeline, the denominator the drop count needs.
    delivered: AtomicU64,
    /// Frames the pipeline's back-pressure dropped.
    dropped: AtomicU64,
    /// Unix milliseconds the session started; 0 before the first start.
    started_at_ms: AtomicU64,
    /// Recent *camera-capture* failures, pruned to [`REJECTION_RATE_WINDOW`]: a current
    /// failure burst, independent of `rejected`, which mixes in stacking rejects.
    failures: Mutex<VecDeque<Instant>>,
}

/// One reading of the frame counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameCounts {
    pub frames: u64,
    pub stacked: u64,
    pub rejected: u64,
}

impl SessionStats {
    pub fn counts(&self) -> FrameCounts {
        FrameCounts {
            frames: self.frames.load(Ordering::SeqCst),
            stacked: self.stacked.load(Ordering::SeqCst),
            rejected: self.rejected.load(Ordering::SeqCst),
        }
    }

    /// When the session started, Unix milliseconds.
    pub fn started_at(&self) -> Option<u64> {
        Some(self.started_at_ms.load(Ordering::SeqCst)).filter(|&ms| ms != 0)
    }

    /// A frame the stack decided on. An unstacked one counts as rejected only while
    /// `stacking`: live view drops nothing.
    pub fn frame_captured(&self, stacked: bool, stacking: bool) -> FrameCounts {
        self.frames.fetch_add(1, Ordering::SeqCst);
        if stacked {
            self.stacked.fetch_add(1, Ordering::SeqCst);
        } else if stacking {
            self.rejected.fetch_add(1, Ordering::SeqCst);
        }
        self.counts()
    }

    /// A frame the camera failed to deliver, at `now`: counted like a rejected frame, and
    /// recorded in the failure-burst window whatever `stacking` says — that window is about
    /// whether the camera responds.
    pub fn frame_failed(&self, stacking: bool, now: Instant) -> FrameCounts {
        self.frames.fetch_add(1, Ordering::SeqCst);
        if stacking {
            self.rejected.fetch_add(1, Ordering::SeqCst);
        }
        self.record_failure(now);
        self.counts()
    }

    /// Records a camera-capture failure at `now`, prunes the window, and returns whether
    /// the failures within it reached [`REJECTION_RATE_THRESHOLD`]. `now` is a parameter so
    /// the window is testable without real sleeps.
    pub fn record_failure(&self, now: Instant) -> bool {
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        failures.push_back(now);
        while failures
            .front()
            .is_some_and(|t| now.duration_since(*t) > REJECTION_RATE_WINDOW)
        {
            failures.pop_front();
        }
        failures.len() >= REJECTION_RATE_THRESHOLD
    }

    /// Whether the camera is failing right now: a burst within the window.
    pub fn failing(&self) -> bool {
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).len() >= REJECTION_RATE_THRESHOLD
    }

    /// Zeroes the counters, keeping the start time: a stack reset, not a new session.
    pub fn reset_counters(&self) {
        for counter in [&self.frames, &self.stacked, &self.rejected, &self.delivered, &self.dropped] {
            counter.store(0, Ordering::SeqCst);
        }
        self.failures.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// A new session starting at `now_ms` (Unix milliseconds).
    pub fn start(&self, now_ms: u64) {
        self.reset_counters();
        self.started_at_ms.store(now_ms, Ordering::SeqCst);
    }

    /// A frame the camera handed the pipeline, dropped or not. Returns the running total.
    pub fn frame_delivered(&self) -> u64 {
        self.delivered.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// A frame back-pressure dropped. Returns the running total.
    pub fn frame_dropped(&self) -> u64 {
        self.dropped.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::SeqCst)
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
    }

    /// Share of delivered frames the pipeline could not take, `0.0..=1.0`; `0.0` before any
    /// frame was delivered — no frames is no evidence, not a perfect session.
    pub fn drop_rate(&self) -> f64 {
        match self.delivered() {
            0 => 0.0,
            delivered => self.dropped() as f64 / delivered as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_within_the_window_trips() {
        let stats = SessionStats::default();
        let base = Instant::now();
        let mut tripped = false;
        for i in 0..REJECTION_RATE_THRESHOLD {
            tripped = stats.record_failure(base + Duration::from_millis(i as u64 * 10));
        }
        assert!(tripped, "threshold failures within the window should trip");
        assert!(stats.failing());
    }

    #[test]
    fn failures_spread_beyond_the_window_never_trip() {
        let stats = SessionStats::default();
        let base = Instant::now();
        // One every 2 s: by the time the Nth lands everything before the window start is
        // pruned, so the count never reaches the threshold.
        for i in 0..(REJECTION_RATE_THRESHOLD * 3) {
            assert!(!stats.record_failure(base + Duration::from_secs(i as u64 * 2)));
        }
        assert!(!stats.failing());
    }

    #[test]
    fn a_gap_longer_than_the_window_prunes_every_earlier_failure() {
        let stats = SessionStats::default();
        let base = Instant::now();
        for i in 0..(REJECTION_RATE_THRESHOLD as u64 - 1) {
            stats.record_failure(base + Duration::from_millis(i * 10));
        }
        let later = base + REJECTION_RATE_WINDOW + Duration::from_secs(1);
        for i in 0..(REJECTION_RATE_THRESHOLD as u64 - 1) {
            assert!(!stats.record_failure(later + Duration::from_millis(i)), "pruned, not summed");
        }
    }

    /// Live view rejects nothing: only a stacking session counts an unstacked frame.
    #[test]
    fn an_unstacked_frame_counts_as_rejected_only_while_stacking() {
        let stats = SessionStats::default();
        stats.frame_captured(true, true);
        stats.frame_captured(false, true);
        stats.frame_captured(false, false);
        let now = Instant::now();
        stats.frame_failed(true, now);
        stats.frame_failed(false, now);
        assert_eq!(
            stats.counts(),
            FrameCounts {
                frames: 5,
                stacked: 1,
                rejected: 2
            }
        );
    }

    #[test]
    fn a_counter_reset_keeps_the_start_and_a_start_resets_everything() {
        let stats = SessionStats::default();
        assert_eq!(stats.started_at(), None);
        stats.start(1_700_000_000_000);
        stats.frame_captured(true, true);
        stats.frame_delivered();
        stats.frame_dropped();

        stats.reset_counters();
        assert_eq!(stats.counts(), FrameCounts::default());
        assert_eq!((stats.delivered(), stats.dropped()), (0, 0));
        assert_eq!(stats.started_at(), Some(1_700_000_000_000));
    }

    /// The count alone is not the number an observer needs: 40 drops is a ruined evening
    /// at 30 s subs and a rounding error at 100 ms.
    #[test]
    fn the_drop_rate_is_a_share_of_what_the_camera_delivered() {
        let stats = SessionStats::default();
        assert_eq!(stats.drop_rate(), 0.0, "no frames is no evidence");
        for _ in 0..100 {
            stats.frame_delivered();
        }
        for _ in 0..35 {
            stats.frame_dropped();
        }
        assert!((stats.drop_rate() - 0.35).abs() < 1e-9);

        let fresh = SessionStats::default();
        fresh.frame_dropped();
        assert_eq!(fresh.drop_rate(), 0.0, "a drop with nothing delivered divides by nothing");
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
    /// Which position this camera occupies. At most one camera holds each role, and
    /// the role decides which slot owns its handle and which stream it feeds.
    pub role: super::CameraRole,
    /// Camera info
    pub info: CameraInfo,
}

/// Everything an interrupted capture needs to pick up where it left off.
///
/// Recorded when a capture session starts and consumed by the reconnect
/// supervisor. Restoring from a snapshot rather than re-reading live state
/// matters twice over: the settings file may have moved on by the time the
/// reconnect lands, and the disk session directory has to be rejoined rather
/// than recreated so one observation stays in one folder.
#[derive(Debug, Clone)]
pub struct SessionResumePlan {
    /// The camera the capture was running against.
    pub camera_id: String,
    /// The settings in force when the capture started, including the mode
    /// (live / wanderer / stacking / planetary) the user was in.
    pub settings: CaptureSettings,
    /// The raw-frame directory to rejoin, if the session was saving frames.
    pub disk_session_dir: Option<PathBuf>,
    /// The number the resumed run's first frame takes. The writer names raw files
    /// `frame_{:06}.fits` and replaces one that exists, so a resume that numbered from 1
    /// again overwrote the subs its folder already held.
    pub next_frame: u64,
}
