//! The reconnect supervisor: reopens a suspended camera (see `recovery`) and resumes
//! what it was doing.
//!
//! Safe to automate only because of three guards: a stale handle can no longer close a
//! device a reconnect just opened (`camera::DeviceLease`), a reopened handle isn't
//! trusted until it answers (`lifecycle::verify_responsive`), and it isn't trusted
//! until it is the *same camera* (`camera::identity`) — reopening by list position put
//! the imaging camera in the guide role on 2026-09-07.
//!
//! Prompt, but not reckless: the first attempt waits for any SDK call the watchdog
//! abandoned to come back out of the vendor library (up to `ABANDONED_CALL_WAIT`),
//! retries follow `RETRY_SCHEDULE`, and the whole effort is bounded by `TOTAL_BUDGET`.
//! Silent until `NOTICE_AFTER`. Single-flight per slot.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::recovery;
use crate::server::error::ApiError;
use crate::server::events::ServerEvent;
use crate::server::state::{
    AppState, CameraRole, CaptureState, ConnectedCameraInfo, Recovery, SessionResumePlan,
};

#[cfg(not(test))]
mod timing {
    use std::time::Duration;
    /// Floor on the first attempt, so a device that is re-enumerating is not asked for
    /// before the OS has it back.
    pub const FIRST_ATTEMPT_MIN_WAIT: Duration = Duration::from_secs(1);
    /// Cap on waiting for abandoned SDK calls. A call stuck for good never returns, and
    /// the lease makes reopening past it safe; 5 s is what the old fixed wait was.
    pub const ABANDONED_CALL_WAIT: Duration = Duration::from_secs(5);
    /// Waits between failed attempts; the last one repeats.
    pub const RETRY_SCHEDULE: &[Duration] = &[
        Duration::from_secs(2),
        Duration::from_secs(3),
        Duration::from_secs(5),
        Duration::from_secs(10),
    ];
    /// How long recovery runs before the observer is told about it. Every dropout in the
    /// 2026-09-07 log was back within ~6 s.
    pub const NOTICE_AFTER: Duration = Duration::from_secs(20);
    /// Past this the camera is not coming back on its own and something physical needs
    /// attention.
    pub const TOTAL_BUDGET: Duration = Duration::from_secs(300);
    /// Cap on each vendor stage of a connect or a reopen — listing, opening and probing,
    /// seeding the cooler and dew heater — all of which hold the connect lock. One that
    /// hangs otherwise held every Connect and never let the recovery budget run out.
    pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
}

/// Test-time shadow of the production timings, same shape, milliseconds instead of
/// seconds — the pattern `RAMP_RATE_C_PER_MIN` uses.
#[cfg(test)]
pub(super) mod timing {
    use std::time::Duration;
    pub const FIRST_ATTEMPT_MIN_WAIT: Duration = Duration::from_millis(20);
    pub const ABANDONED_CALL_WAIT: Duration = Duration::from_millis(400);
    pub const RETRY_SCHEDULE: &[Duration] = &[
        Duration::from_millis(40),
        Duration::from_millis(60),
        Duration::from_millis(100),
        Duration::from_millis(150),
    ];
    pub const NOTICE_AFTER: Duration = Duration::from_millis(700);
    pub const TOTAL_BUDGET: Duration = Duration::from_millis(2_500);
    pub const OPEN_TIMEOUT: Duration = Duration::from_millis(300);
}

pub(super) use timing::*;

/// The wait after failed attempt number `attempt` (counting from 1).
pub(super) fn retry_delay(attempt: u32) -> Duration {
    let last = RETRY_SCHEDULE.len() - 1;
    RETRY_SCHEDULE[(attempt.max(1) as usize - 1).min(last)]
}

/// How many more attempts still start inside `TOTAL_BUDGET` after attempt `attempt`
/// ended `elapsed` into recovery — for the "attempt N of M" the UI shows. Counted from the
/// time actually left: attempts are not instant, and a count from zero-cost attempts
/// promised 32 where the budget ran out near 20.
pub(super) fn attempts_left(elapsed: Duration, attempt: u32) -> u32 {
    let mut starts_at = elapsed;
    let mut after = attempt;
    let mut left = 0;
    loop {
        starts_at += retry_delay(after);
        if starts_at >= TOTAL_BUDGET {
            return left;
        }
        left += 1;
        after += 1;
    }
}

/// Why the supervisor stopped without reconnecting.
enum Stop {
    /// Out of budget, or reconnecting was switched off. The observer must be told.
    GaveUp { attempts: u32, reason: String },
    /// The observer took over — disconnected the camera or replaced it — so there is
    /// nothing left to recover and nothing to report.
    Abandoned(&'static str),
}

/// Start reconnecting `recorded`'s camera in the background, unless its slot already
/// has a supervisor.
///
/// The single-flight guard is per slot, not global: a guide camera dropping out while
/// the main camera is being recovered has its own device to reopen, and refusing it —
/// which one shared flag did — left the guide camera down for the rest of the night.
pub(super) fn spawn(state: &Arc<AppState>, recorded: ConnectedCameraInfo) {
    let role = recorded.role;
    let in_flight = Arc::clone(&state.slot(role).reconnect_in_flight);
    if in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        warn!(
            camera_id = %recorded.id,
            role = role.label(),
            "Reconnect already in progress for this slot; not starting another"
        );
        return;
    }

    let state = Arc::clone(state);
    tokio::spawn(async move {
        match supervise(&state, &recorded).await {
            Ok(()) => info!(camera_id = %recorded.id, role = role.label(), "Camera recovered"),
            Err(Stop::Abandoned(why)) => {
                info!(camera_id = %recorded.id, why, "Stopped reconnecting the camera")
            }
            Err(Stop::GaveUp { attempts, reason }) => {
                recovery::give_up(&state, &recorded, attempts, &reason).await
            }
        }
        release_flight(&state, role).await;
    });
}

/// End a supervisor's hold on `role`'s slot.
///
/// A fault that landed while the supervisor was finishing — resuming the capture, say —
/// found it still in flight and was refused one of its own. Cleared first and checked
/// second, so either this check sees that suspension or the suspension's own spawn got
/// through.
pub(super) async fn release_flight(state: &Arc<AppState>, role: CameraRole) {
    state.slot(role).reconnect_in_flight.store(false, Ordering::SeqCst);
    if state.slot(role).recovery() != Recovery::Suspended {
        return;
    }
    let Some(orphan) = state.camera_in_role(role).await else {
        return;
    };
    warn!(camera_id = %orphan.id, role = role.label(), "Camera failed again as its recovery ended; recovering it again");
    spawn(state, orphan);
}

/// Run the attempt sequence until the camera is back or the supervisor has to stop.
async fn supervise(state: &Arc<AppState>, recorded: &ConnectedCameraInfo) -> Result<(), Stop> {
    let started = Instant::now();
    let role = recorded.role;
    let name = &recorded.info.name;
    let mut recorded = recorded.clone();

    if !state.settings.read().await.auto_reconnect {
        return Err(Stop::GaveUp {
            attempts: 0,
            reason: "automatic reconnect is switched off".to_string(),
        });
    }

    if !state.slot(role).sdk_calls.wait_drained(ABANDONED_CALL_WAIT).await {
        info!(camera = %name, "An abandoned SDK call is still running; reopening past it");
    }
    tokio::time::sleep(FIRST_ATTEMPT_MIN_WAIT.saturating_sub(started.elapsed())).await;

    let mut noticed = false;
    let mut attempt = 0;
    loop {
        attempt += 1;
        if !state.settings.read().await.auto_reconnect {
            return Err(Stop::GaveUp {
                attempts: attempt - 1,
                reason: "automatic reconnect was switched off".to_string(),
            });
        }
        if !recovery::is_recovering(state, &recorded).await {
            return Err(Stop::Abandoned("the camera was disconnected or replaced"));
        }
        if started.elapsed() >= TOTAL_BUDGET {
            return Err(Stop::GaveUp {
                attempts: attempt - 1,
                reason: format!("still unreachable after {} s", TOTAL_BUDGET.as_secs()),
            });
        }

        // A handle that failed during its install left the entry at its new position.
        if let Some(current) = state.camera_in_role(role).await.filter(|c| c.id == recorded.id) {
            recorded = current;
        }
        match recovery::reopen_for_recovery(state, &recorded).await {
            Ok(connected) => {
                info!(camera = %name, role = role.label(), attempt, elapsed = ?started.elapsed(), "Reconnected");
                // Only the imaging camera has a capture to resume. For the guide camera
                // the reopen has already restarted its loop and reapplied its profile.
                if role == CameraRole::Main {
                    resume_capture_if_planned(state, &connected, noticed).await;
                }
                return Ok(());
            }
            Err(e) => warn!(camera = %name, attempt, error = %e, "Reconnect attempt failed"),
        }

        let wait = retry_delay(attempt);
        let left = attempts_left(started.elapsed(), attempt);
        if started.elapsed() + wait >= NOTICE_AFTER && left > 0 {
            noticed = true;
            let _ = state.events.send(ServerEvent::camera_reconnecting(
                name.clone(),
                role,
                attempt + 1,
                attempt + left,
                wait.as_secs(),
            ));
        }
        tokio::time::sleep(wait).await;
    }
}

/// Restart the capture the dropout interrupted, in the mode it was running in, keeping
/// the stack it had already built. Tells the observer only if they were told about the
/// dropout in the first place.
async fn resume_capture_if_planned(state: &Arc<AppState>, connected: &ConnectedCameraInfo, noticed: bool) {
    // Failed again straight after the install: the capture stays paused for the recovery
    // that fault started, which resumes it instead.
    if state.slot(CameraRole::Main).is_recovering() {
        info!(camera_id = %connected.id, "Camera failed again before its capture could resume");
        return;
    }
    let plan = state.session_resume_plan.read().await.clone();
    let Some(plan) = plan.filter(|plan| plan.camera_id == connected.id) else {
        state.end_paused_capture().await;
        return;
    };
    if !state.settings.read().await.auto_resume_capture {
        info!(camera_id = %connected.id, "Not resuming capture — auto-resume is switched off");
        state.end_paused_capture().await;
        return;
    }
    if state.capture_state().await != CaptureState::Recovering {
        return;
    }

    restore_settings(state, &plan).await;

    // Read the live counter rather than a number snapshotted at capture start:
    // what the observer wants to know is how much of the integration survived.
    let stacked_count = state.session.read().await.stacked_count;

    match crate::server::services::CaptureService::resume_capture(state, &plan).await {
        Ok(()) => {
            info!(camera_id = %connected.id, stacked_count, "Capture resumed after reconnect");
            if noticed {
                let _ = state
                    .events
                    .send(ServerEvent::capture_resumed(connected.info.name.clone(), stacked_count));
            }
        }
        // Stopped or disconnected between the check above and the resume.
        Err(ApiError::CaptureNotPaused) => {
            info!(camera_id = %connected.id, "The paused capture was ended before it could resume")
        }
        Err(e) => {
            warn!(camera_id = %connected.id, error = %e, "Could not resume capture after reconnect");
            state.end_paused_capture().await;
            state.send_error(format!(
                "Camera '{}' is back, but the capture could not be resumed: {}",
                connected.info.name, e
            ));
        }
    }
}

/// Put back the capture-shaping settings the session was running with.
///
/// The reopen applies the camera's stored profile, which can differ from what the
/// interrupted session was actually using. The plan follows every settings update made
/// while the capture ran or was paused (`update_settings`), so this restores the
/// observer's latest values, not the ones the capture started with. Saved and announced:
/// a silent write left every client showing values the resumed capture was not using.
/// The camera-hardware fields (cooler, dew heater, sensor mode) are deliberately left as
/// the reopen set them, since those belong to the device rather than the session.
async fn restore_settings(state: &Arc<AppState>, plan: &SessionResumePlan) {
    restore_capture_fields(&mut *state.settings.write().await, plan);
    state.save_settings().await;
    let _ = state.events.send(ServerEvent::SettingsUpdated);
}

fn restore_capture_fields(settings: &mut crate::server::state::CaptureSettings, plan: &SessionResumePlan) {
    let planned = &plan.settings;

    settings.exposure_us = planned.exposure_us;
    settings.gain = planned.gain;
    settings.offset = planned.offset;
    settings.bin = planned.bin;
    settings.stacking = planned.stacking;
    settings.stacking_type = planned.stacking_type;
    settings.wanderer_mode = planned.wanderer_mode;
    settings.raw_frame_saving = planned.raw_frame_saving;
    settings.save_stacked_image = planned.save_stacked_image;
    settings.comet_roi = planned.comet_roi;
    settings.planetary_roi = planned.planetary_roi;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_follow_the_schedule_and_then_hold() {
        assert_eq!(retry_delay(1), RETRY_SCHEDULE[0]);
        assert_eq!(retry_delay(2), RETRY_SCHEDULE[1]);
        let last = *RETRY_SCHEDULE.last().unwrap();
        assert_eq!(retry_delay(RETRY_SCHEDULE.len() as u32), last);
        assert_eq!(retry_delay(50), last);
    }

    /// Every advertised attempt starts inside the budget, and none past it is promised.
    #[test]
    fn the_advertised_attempts_all_start_inside_the_budget() {
        let elapsed = FIRST_ATTEMPT_MIN_WAIT;
        let left = attempts_left(elapsed, 1);
        let mut starts_at = elapsed;
        for after in 1..=left {
            starts_at += retry_delay(after);
            assert!(starts_at < TOTAL_BUDGET, "attempt {} would start past the budget", after + 1);
        }
        assert!(starts_at + retry_delay(left + 1) >= TOTAL_BUDGET, "one more attempt would still fit");
        assert!(NOTICE_AFTER < TOTAL_BUDGET, "the observer must hear before the give-up");
    }

    /// Time spent inside slow attempts shrinks the promise instead of being ignored.
    #[test]
    fn slow_attempts_leave_fewer_attempts_to_advertise() {
        let quick = attempts_left(FIRST_ATTEMPT_MIN_WAIT, 3);
        let slow = attempts_left(TOTAL_BUDGET / 2, 3);
        assert!(slow < quick, "{slow} attempts left at half the budget, {quick} at the start");
        assert_eq!(attempts_left(TOTAL_BUDGET, 3), 0);
    }
}
