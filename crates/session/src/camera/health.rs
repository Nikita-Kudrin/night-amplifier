//! One fault detector for the whole server: the capture loop's frame and status-poll
//! watchdogs, and the camera-session monitor's cooler poll, see the same hardware
//! through different paths, so a fault alternating between them is still one fault.
//!
//! Everything deciding "persistently unresponsive" lives here: the threshold, the
//! per-camera streak (kept in `CameraRoster`, keyed by role+name), and
//! the escalation event — a counter per call site would let a camera failing every other
//! poll stay "healthy". Also the per-camera `RestartHistory` of whether an in-place
//! restart still works, deciding how soon a stall reopens the camera, and the run of opens
//! that never delivered a frame, deciding when recovery stops reopening.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use crate::events::ServerEvent;
use crate::state::{AppState, CameraRole, FramelessRun};

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
/// is worth a message is decided by `camera::recovery`.
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

/// Opens in a row — a connect's first capture, or a reopen's — that faulted before their
/// first frame, at which recovery reopens the camera with the other camera on its SDK
/// closed (`recovery::isolate_from_sibling`). 2026-10-12: the Ares-C PRO reopened 20 times
/// without a frame beside an open Neptune-C II, and streamed the moment that was closed.
pub(crate) const FRAMELESS_ISOLATE_AT: u32 = 2;

/// Frameless opens in a row after which recovery gives up and says so, once the run is
/// [`FRAMELESS_GIVE_UP_AFTER`] old. Every one of that night's reopens counted as a
/// recovery, so the outage stayed silent for three minutes and the fault streak never
/// passed 1.
pub(crate) const FRAMELESS_GIVE_UP_AT: u32 = 3;

/// A run younger than this is not given up on, whatever its count: with 1 s subs three
/// frameless opens take seconds, and a brownout the 300 s reconnect budget rides out
/// would end in a give-up inside the 20 s recovery stays quiet for. That night's ~9 s
/// cycle reaches it in about seven opens.
#[cfg(not(test))]
pub(crate) const FRAMELESS_GIVE_UP_AFTER: Duration = Duration::from_secs(60);
#[cfg(test)]
pub(crate) const FRAMELESS_GIVE_UP_AFTER: Duration = Duration::from_millis(600);

/// How long a frameless run is remembered without a new one, as for restarts.
const FRAMELESS_HISTORY_TTL: Duration = RESTART_HISTORY_TTL;

/// Record that the camera's latest open faulted before its first frame. Returns the run.
pub(crate) fn record_frameless_open(state: &AppState, role: CameraRole, camera_name: &str) -> u32 {
    let run = state
        .roster
        .bump_frameless_opens(role, camera_name, FRAMELESS_HISTORY_TTL, Instant::now())
        .count;
    warn!(camera_name, role = role.label(), run, "The camera faulted before delivering its first frame since it was opened");
    run
}

/// A frame arrived: whatever the opens before it did, the camera delivers, and the other
/// camera on its SDK, if isolation closed it, may reopen beside it.
pub(crate) fn frame_arrived(state: &AppState, role: CameraRole, camera_name: &str) {
    forget_frameless_opens(state, role, camera_name);
    super::recovery::end_isolation(state, role);
}

/// Forget the run for a fresh attempt: the observer's own Connect or Start, or a reopen
/// no capture will judge by its first frame. Kept past those, a run of 2 on an idle
/// camera closed the healthy guide camera on its next, unrelated fault.
pub(crate) fn forget_frameless_opens(state: &AppState, role: CameraRole, camera_name: &str) {
    state.roster.forget_frameless_opens(role, camera_name);
}

/// The camera's current run of opens that faulted before their first frame.
pub(crate) fn frameless_opens(state: &AppState, role: CameraRole, camera_name: &str) -> u32 {
    frameless_run(state, role, camera_name).map_or(0, |run| run.count)
}

fn frameless_run(state: &AppState, role: CameraRole, camera_name: &str) -> Option<FramelessRun> {
    state
        .roster
        .frameless_run(role, camera_name, FRAMELESS_HISTORY_TTL, Instant::now())
}

/// What recovery does about a camera's run of frameless opens before its next reopen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FramelessVerdict {
    Reopen,
    /// Reopen it with the other camera on its SDK closed.
    Isolate,
    GiveUp { opens: u32 },
}

pub(crate) fn frameless_verdict(state: &AppState, role: CameraRole, camera_name: &str) -> FramelessVerdict {
    verdict_for(frameless_run(state, role, camera_name), Instant::now())
}

fn verdict_for(run: Option<FramelessRun>, now: Instant) -> FramelessVerdict {
    let Some(run) = run else {
        return FramelessVerdict::Reopen;
    };
    let old_enough = now.saturating_duration_since(run.since) >= FRAMELESS_GIVE_UP_AFTER;
    if run.count >= FRAMELESS_GIVE_UP_AT && old_enough {
        return FramelessVerdict::GiveUp { opens: run.count };
    }
    if run.count >= FRAMELESS_ISOLATE_AT {
        return FramelessVerdict::Isolate;
    }
    FramelessVerdict::Reopen
}

#[cfg(test)]
mod frameless_tests {
    use super::*;
    use crate::state::CameraRoster;

    fn run_of(count: u32, start: Instant) -> Option<FramelessRun> {
        let roster = CameraRoster::default();
        (0..count)
            .map(|_| roster.bump_frameless_opens(CameraRole::Main, "Ares", FRAMELESS_HISTORY_TTL, start))
            .last()
    }

    #[test]
    fn the_run_isolates_at_two_and_gives_up_at_three_once_old_enough() {
        let start = Instant::now();
        let old = start + FRAMELESS_GIVE_UP_AFTER;
        assert_eq!(verdict_for(None, old), FramelessVerdict::Reopen);
        assert_eq!(verdict_for(run_of(1, start), old), FramelessVerdict::Reopen);
        assert_eq!(verdict_for(run_of(2, start), old), FramelessVerdict::Isolate);
        assert_eq!(verdict_for(run_of(3, start), old), FramelessVerdict::GiveUp { opens: 3 });
    }

    /// Short subs reach three frameless opens in seconds: too soon to call the camera gone.
    #[test]
    fn a_young_run_is_isolated_not_given_up_on() {
        let start = Instant::now();
        let young = start + FRAMELESS_GIVE_UP_AFTER / 2;
        assert_eq!(verdict_for(run_of(5, start), young), FramelessVerdict::Isolate);
    }
}
