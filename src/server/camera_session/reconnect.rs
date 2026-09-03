//! Recovery after a camera drops out mid-session. A USB stall or bus reset leaves
//! the session dead though the hardware is usually fine seconds later — the
//! observing log this was built from shows three dropouts costing three app
//! restarts and an hour of integration each time. Safe to automate only because of
//! two guards: a stale handle can no longer close a device a reconnect just opened
//! (`camera::DeviceLease`), and a reopened handle isn't assumed to work just because
//! `open()` returned (`lifecycle::connect` probes it) — without both, the first
//! reconnect attempt lands inside the abandoned handle's still-alive window and
//! loops forever.
//!
//! Deliberately reluctant: **bounded** (`MAX_ATTEMPTS` inside `TOTAL_BUDGET`, then
//! stops and says so), **backed off** (first wait longer than the stall, since an
//! abandoned SDK handle may still be running), **re-enumerated** (a missing device
//! index means unplugged, not hiccuped, so it's not reopened), **single-flight**
//! (one supervisor at a time, none while the user disconnects on purpose).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use super::lifecycle;
use crate::camera::CameraRegistry;
use crate::server::events::ServerEvent;
use crate::server::state::{AppState, CaptureState, SessionResumePlan};

/// How many times to try before giving up.
const MAX_ATTEMPTS: u32 = 5;

/// Wait before the first attempt.
///
/// Longer than the monitor's and capture path's own 3 s call budgets on
/// purpose: when those time out they abandon a handle to a thread still inside
/// the vendor SDK, and reopening the device while that thread is running is the
/// exact race this whole subsystem exists to avoid. The field log shows a
/// manual reconnect 9 s after a stall still landing inside that window.
const FIRST_BACKOFF: Duration = Duration::from_secs(5);

/// Ceiling for the doubling backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Overall wall-clock budget. Past this the camera is not coming back on its
/// own and something physical needs attention.
const TOTAL_BUDGET: Duration = Duration::from_secs(300);

/// Start recovering `camera_id` in the background, unless a supervisor is
/// already running or the user has turned auto-reconnect off.
pub(super) fn spawn(state: &Arc<AppState>, camera_id: &str, camera_name: &str) {
    if state
        .reconnect_in_flight
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        warn!(
            camera_id,
            "Reconnect already in progress; not starting another"
        );
        return;
    }

    let state = Arc::clone(state);
    let camera_id = camera_id.to_string();
    let camera_name = camera_name.to_string();

    tokio::spawn(async move {
        let outcome = supervise(&state, &camera_id, &camera_name).await;
        state.reconnect_in_flight.store(false, Ordering::SeqCst);

        match outcome {
            Ok(()) => info!(camera_id = %camera_id, "Camera recovered"),
            Err(reason) => {
                warn!(camera_id = %camera_id, %reason, "Giving up on reconnecting the camera");
                let _ = state.events.send(ServerEvent::camera_reconnect_failed(
                    camera_name.clone(),
                    MAX_ATTEMPTS,
                    reason.clone(),
                ));
                state.send_error(format!(
                    "Could not bring camera '{}' back ({}). Check the cable and reconnect.",
                    camera_name, reason
                ));
            }
        }
    });
}

/// Run the attempt sequence. `Err` carries the reason to show the user.
async fn supervise(
    state: &Arc<AppState>,
    camera_id: &str,
    camera_name: &str,
) -> Result<(), String> {
    if !state.settings.read().await.auto_reconnect {
        return Err("automatic reconnect is switched off".to_string());
    }

    let started = Instant::now();
    let mut backoff = FIRST_BACKOFF;

    for attempt in 1..=MAX_ATTEMPTS {
        let _ = state.events.send(ServerEvent::camera_reconnecting(
            camera_name,
            attempt,
            MAX_ATTEMPTS,
            backoff.as_secs(),
        ));
        info!(
            camera_id,
            attempt,
            of = MAX_ATTEMPTS,
            wait_s = backoff.as_secs(),
            "Waiting before reconnect attempt"
        );
        tokio::time::sleep(backoff).await;

        if let Some(reason) = abandon_reason(state, camera_id).await {
            return Err(reason);
        }
        if started.elapsed() >= TOTAL_BUDGET {
            return Err(format!(
                "still unreachable after {} s",
                TOTAL_BUDGET.as_secs()
            ));
        }

        if !device_is_present(state, camera_id).await {
            warn!(camera_id, attempt, "Device is not enumerated; will retry");
            backoff = (backoff * 2).min(MAX_BACKOFF);
            continue;
        }

        // `connect` probes the handle before reporting success, so reaching
        // here means the camera answered, not merely that `open()` returned.
        match lifecycle::connect(state, camera_id).await {
            Ok(_) => {
                info!(camera_id, attempt, "Reconnected");
                resume_capture_if_planned(state, camera_id, camera_name).await;
                return Ok(());
            }
            Err(e) => {
                warn!(camera_id, attempt, error = %e, "Reconnect attempt failed");
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }

    Err(format!("{} attempts failed", MAX_ATTEMPTS))
}

/// Why the supervisor should stop trying, if it should. Re-checked before every
/// attempt because all of these can change while it is sleeping.
async fn abandon_reason(state: &Arc<AppState>, camera_id: &str) -> Option<String> {
    if !state.settings.read().await.auto_reconnect {
        return Some("automatic reconnect was switched off".to_string());
    }
    if state.cameras.read().await.contains_key(camera_id) {
        // Somebody connected it by hand while we were waiting.
        return Some("the camera was reconnected manually".to_string());
    }
    None
}

/// Whether the device index is still enumerated by its provider.
///
/// Reopening an index the SDK no longer lists is how a reconnect ends up
/// holding a handle to nothing — which is indistinguishable, from the outside,
/// from the failure this module exists to fix.
async fn device_is_present(state: &Arc<AppState>, camera_id: &str) -> bool {
    let Ok((provider, index)) = lifecycle::parse_camera_id(camera_id) else {
        return false;
    };
    let provider = provider.to_string();
    let use_simulated = state.settings.read().await.use_simulated_camera;

    tokio::task::spawn_blocking(move || {
        let mut registry = CameraRegistry::new();
        registry.register_defaults();
        if use_simulated {
            let _ = registry.register(crate::camera::SimulatedProvider::new());
        }
        let Some(name) = registry
            .providers()
            .into_iter()
            .find(|p| p.eq_ignore_ascii_case(&provider))
            .map(str::to_string)
        else {
            return false;
        };
        registry
            .list_cameras(&name)
            .map(|cameras| index < cameras.len())
            .unwrap_or(false)
    })
    .await
    .unwrap_or(false)
}

/// Restart the capture the dropout interrupted, in the mode it was running in,
/// keeping the stack it had already built.
async fn resume_capture_if_planned(state: &Arc<AppState>, camera_id: &str, camera_name: &str) {
    let plan = state.session_resume_plan.read().await.clone();
    let Some(plan) = plan else {
        return;
    };
    if plan.camera_id != camera_id {
        return;
    }
    if !state.settings.read().await.auto_resume_capture {
        info!(
            camera_id,
            "Not resuming capture — auto-resume is switched off"
        );
        return;
    }
    if state.capture_state().await != CaptureState::Idle {
        return;
    }

    restore_settings(state, &plan).await;

    // Read the live counter rather than a number snapshotted at capture start:
    // what the observer wants to know is how much of the integration survived.
    let stacked_count = state.session.read().await.stacked_count;

    match crate::server::services::CaptureService::resume_capture(state, &plan).await {
        Ok(()) => {
            info!(camera_id, stacked_count, "Capture resumed after reconnect");
            let _ = state
                .events
                .send(ServerEvent::capture_resumed(camera_name, stacked_count));
        }
        Err(e) => {
            warn!(camera_id, error = %e, "Could not resume capture after reconnect");
            state.send_error(format!(
                "Camera '{}' is back, but the capture could not be resumed: {}",
                camera_name, e
            ));
        }
    }
}

/// Put back the capture-shaping settings the session was running with.
///
/// `connect` applies the camera's stored profile, which can differ from what
/// the interrupted session was actually using — and the settings file may have
/// been edited while the supervisor was waiting. The camera-hardware fields
/// (cooler, dew heater, sensor mode) are deliberately left as `connect` set
/// them, since those belong to the device rather than the session.
async fn restore_settings(state: &Arc<AppState>, plan: &SessionResumePlan) {
    let mut settings = state.settings.write().await;
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
