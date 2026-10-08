//! One fault detector for the whole server: the capture loop's frame and status-poll
//! watchdogs, and the camera-session monitor's cooler poll, see the same hardware
//! through different paths, so a fault alternating between them is still one fault.
//!
//! Everything deciding "persistently unresponsive" lives here: the threshold, the
//! per-camera streak (kept in `CameraRoster`, keyed by role+name), and
//! the escalation event — a counter per call site would let a camera failing every other
//! poll stay "healthy". Also the per-camera `RestartHistory` of whether an in-place restart still works, deciding how soon a stall reopens the camera.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CameraRole};

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
    /// The slot's handle is gone with nothing holding it — abandoned to a stuck call, or
    /// lost on a path nobody logged. Nothing can command the camera until it is reopened.
    HandleLost,
}

/// Clear a camera's fault streak. Called whenever any SDK call returns within
/// its budget and without a device-lost error, since that proves the camera is
/// currently responding regardless of which call site observed it.
pub(crate) fn clear_fault_streak(state: &Arc<AppState>, role: CameraRole, camera_name: &str) {
    state.roster.clear_fault_streak(role, camera_name);
}

/// Whether `camera_name`'s last calls failed, inside `FAULT_STREAK_TTL`. A camera in that
/// state is not one to hold a five-minute warm-up on: the next call is likely to fail too.
pub(crate) fn has_recent_fault(state: &AppState, role: CameraRole, camera_name: &str) -> bool {
    state
        .roster
        .fault_streak(role, camera_name)
        .is_some_and(|(count, at)| count > 0 && at.elapsed() <= FAULT_STREAK_TTL)
}

/// Record one fault against `camera_name` and report the resulting streak.
///
/// Escalates with `ServerEvent::CameraPersistentlyUnresponsive` on reaching
/// `PERSISTENT_FAULT_THRESHOLD`. A single incident sends the user nothing: whether it
/// is worth a message is decided by `camera_session::recovery`.
pub(crate) fn record_fault(
    state: &Arc<AppState>,
    role: CameraRole,
    camera_name: &str,
    kind: FaultKind,
) -> u32 {
    let consecutive =
        state.roster.bump_fault_streak(role, camera_name, FAULT_STREAK_TTL, Instant::now());

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
/// the `CameraRoster` rather than the loop's `StallTracker`, which the reopen
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
    role: CameraRole,
    camera_name: &str,
    recovered: bool,
) {
    if recovered {
        state.roster.forget_restart_history(role, camera_name);
    } else {
        state.roster.restart_failed(role, camera_name, Instant::now());
    }
}

/// Forget what restarts did for a camera the observer is connecting: a reseated cable or
/// another hub is a new record. Not called by recovery's reopen, whose whole point is to
/// carry the record past the tracker it rebuilds.
pub(crate) fn forget_restart_history(
    state: &AppState,
    role: CameraRole,
    camera_name: &str,
) {
    state.roster.forget_restart_history(role, camera_name);
}

/// The failed restarts in a row behind skipping the next one, or `None` while restarting
/// in place is still worth a stall budget for this camera.
pub(crate) fn distrusted_restarts(
    state: &AppState,
    role: CameraRole,
    camera_name: &str,
) -> Option<u32> {
    state
        .roster
        .restart_history(role, camera_name)
        .and_then(|history| history.distrusted(Instant::now()))
}
