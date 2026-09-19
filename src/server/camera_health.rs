//! One fault detector for the whole server. Three places can discover a camera has
//! stopped answering — the capture loop's frame and status-poll watchdogs, and the
//! camera-session monitor's cooler poll — seeing the same hardware through
//! different code paths, so a fault alternating between them is still one fault.
//!
//! Everything deciding "persistently unresponsive" lives here: the threshold, the
//! per-camera streak (`AppState.consecutive_watchdog_timeouts`), and the escalation
//! event. A counter per call site instead would need each site to independently
//! reach the threshold, letting a camera failing every other poll stay "healthy".
//! So does the per-camera record of whether restarting a stalled stream in place still
//! works (`RestartHistory`), which decides how soon a stall reopens the camera.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use crate::server::events::ServerEvent;
use crate::server::state::AppState;

/// Consecutive faults against one camera before escalating from an ordinary
/// disconnect to a distinct "persistently unresponsive" signal
/// (`ServerEvent::CameraPersistentlyUnresponsive`).
pub(crate) const PERSISTENT_FAULT_THRESHOLD: u32 = 3;

/// How long a streak survives without new evidence.
///
/// A plain "any success resets to zero" rule loses a fault that alternates
/// between a dead-handle error and an ordinary transient one — each transient
/// wipes the evidence and the threshold is never reached. Ageing the streak out
/// instead means only a genuinely quiet interval clears it.
pub(crate) const FAULT_STREAK_TTL: Duration = Duration::from_secs(60);

/// What kind of evidence a fault report carries. Both kinds count toward the
/// same streak — a hung call and a handle the SDK has invalidated are two
/// symptoms of one dead camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FaultKind {
    /// A bounded SDK call did not return within its budget. The handle was
    /// abandoned to a detached thread and must not be used again.
    Timeout,
    /// The SDK answered, but said the device is gone — see
    /// `CameraError::is_sdk_disconnected`.
    DeviceLost,
}

/// Clear a camera's fault streak. Called whenever any SDK call returns within
/// its budget and without a device-lost error, since that proves the camera is
/// currently responding regardless of which call site observed it.
pub(crate) fn clear_fault_streak(state: &Arc<AppState>, camera_name: &str) {
    let mut counts = state
        .consecutive_watchdog_timeouts
        .lock()
        .expect("consecutive_watchdog_timeouts mutex poisoned");
    counts.remove(camera_name);
}

/// Record one fault against `camera_name` and report the resulting streak.
///
/// Escalates with `ServerEvent::CameraPersistentlyUnresponsive` on reaching
/// `PERSISTENT_FAULT_THRESHOLD`. A single incident sends the user nothing: whether it
/// is worth a message is decided by `camera_session::recovery`.
pub(crate) fn record_fault(state: &Arc<AppState>, camera_name: &str, kind: FaultKind) -> u32 {
    let consecutive = state.bump_fault_streak(camera_name, FAULT_STREAK_TTL);

    warn!(
        camera_name = %camera_name,
        ?kind,
        consecutive,
        "Camera fault recorded"
    );

    if consecutive >= PERSISTENT_FAULT_THRESHOLD {
        error!(
            camera_name = %camera_name,
            consecutive,
            "Camera appears persistently unresponsive"
        );
        let _ = state
            .events
            .send(ServerEvent::camera_persistently_unresponsive(
                camera_name.to_string(),
                consecutive,
            ));
    }

    consecutive
}

/// Whether a streak has reached the point where the handle should be given up
/// on rather than retried.
pub(crate) fn is_persistent(consecutive: u32) -> bool {
    consecutive >= PERSISTENT_FAULT_THRESHOLD
}

/// In-place stream restarts that must fail on one camera, in a row, before its next stall
/// reopens it straight away.
///
/// 2026-09-14 field log: restarting in place cured at most 3 of 145 guide stalls and none
/// of 6 imaging ones, while every reopen brought frames back. Each useless restart cost a
/// whole stall budget — ~9 s of a ~15 s outage at a 0.5 s exposure.
pub(crate) const RESTART_DISTRUST_AFTER: u32 = 2;

/// How long failed restarts are remembered. Spans a bad spell of reopen-and-stall cycles
/// (~15 s apart) without carrying one evening's cable into the next session.
pub(crate) const RESTART_HISTORY_TTL: Duration = Duration::from_secs(600);

/// What restarting the stream in place has lately done for one camera. Kept in
/// `AppState.restart_histories` rather than the loop's `StallTracker`, which the reopen
/// it is meant to shortcut rebuilds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RestartHistory {
    failed_in_a_row: u32,
    last_failure: Option<Instant>,
}

impl RestartHistory {
    /// A restart that worked is not recorded here: it removes the whole history
    /// (`record_restart_outcome`).
    pub(crate) fn record_failure(&mut self, now: Instant) {
        if !self.is_current(now) {
            self.failed_in_a_row = 0;
        }
        self.failed_in_a_row += 1;
        self.last_failure = Some(now);
    }

    /// Restarts that failed in a row, while that is still enough to skip the next one.
    pub(crate) fn distrusted(&self, now: Instant) -> Option<u32> {
        (self.failed_in_a_row >= RESTART_DISTRUST_AFTER && self.is_current(now))
            .then_some(self.failed_in_a_row)
    }

    fn is_current(&self, now: Instant) -> bool {
        self.last_failure
            .is_some_and(|at| now.saturating_duration_since(at) <= RESTART_HISTORY_TTL)
    }
}

/// Record whether the in-place restart before this capture brought the camera back.
///
/// Keyed by role as well as name: two bodies of one model, one per role, are two devices
/// on two cables — twins once ended each other's recovery through a per-name phase.
pub(crate) fn record_restart_outcome(
    state: &AppState,
    role: crate::server::state::CameraRole,
    camera_name: &str,
    recovered: bool,
) {
    let mut histories = state
        .restart_histories
        .lock()
        .expect("restart_histories mutex poisoned");
    let key = (role, camera_name.to_string());
    if recovered {
        histories.remove(&key);
        return;
    }
    histories.entry(key).or_default().record_failure(Instant::now());
}

/// Forget what restarts did for a camera the observer is connecting: a reseated cable or
/// another hub is a new record. Not called by recovery's reopen, whose whole point is to
/// carry the record past the tracker it rebuilds.
pub(crate) fn forget_restart_history(
    state: &AppState,
    role: crate::server::state::CameraRole,
    camera_name: &str,
) {
    state
        .restart_histories
        .lock()
        .expect("restart_histories mutex poisoned")
        .remove(&(role, camera_name.to_string()));
}

/// The failed restarts in a row behind skipping the next one, or `None` while restarting
/// in place is still worth a stall budget for this camera.
pub(crate) fn distrusted_restarts(
    state: &AppState,
    role: crate::server::state::CameraRole,
    camera_name: &str,
) -> Option<u32> {
    state
        .restart_histories
        .lock()
        .expect("restart_histories mutex poisoned")
        .get(&(role, camera_name.to_string()))
        .and_then(|history| history.distrusted(Instant::now()))
}
