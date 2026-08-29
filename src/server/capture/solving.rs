use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::detection::{DetectionConfig, StarDetector};
use crate::frame::Frame;
use crate::push_to::{PushToBlocker, PushToError, SolveOutcome};
use crate::server::events::ServerEvent;
use crate::server::state::AppState;

/// Shortest gap between two frames being offered to the solver.
///
/// Not a limit on how often a *solve* runs — that is the movement detector's job —
/// but on how often we pay for the star detection that feeds it. See
/// [`PushToState::try_begin_solve`](crate::server::services::PushToState::try_begin_solve).
const MIN_SOLVE_ATTEMPT_INTERVAL: Duration = Duration::from_millis(1000);

/// The detector used for the movement check and for the stars handed to ASTAP.
///
/// Shared rather than rebuilt per frame: it is stateless configuration, and the
/// settings must not drift between the movement comparison and the solve that
/// follows it.
fn solve_detector() -> &'static StarDetector {
    static DETECTOR: OnceLock<StarDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| StarDetector::new(DetectionConfig::sensitive().with_max_stars(200)))
}

/// Whether a plate solve could possibly run right now.
///
/// Callers check this *before* preparing a frame, so the common case — Community
/// edition, or Pro with no target set — costs an atomic load instead of a
/// full-frame copy. Only covers the checks that are cheap and synchronous; the
/// rest still happen inside [`try_plate_solve`].
pub fn plate_solve_available(state: &Arc<AppState>) -> bool {
    if crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN).is_none() {
        return false;
    }

    // Check local state via try_read to avoid blocking the stacking pipeline.
    // If we have no target or are already solving, we can safely skip spawning
    // the heavy tokio task and save the 50MB frame clone.
    if let Ok(guard) = state.push_to.try_read() {
        if let Some(ref pt) = *guard {
            if pt.is_solving() || !pt.has_target {
                return false;
            }
        }
    }

    true
}

/// Try to plate solve if a target is set and solver is ready
///
/// In the Community edition, this does nothing unless the Push-To plugin is installed
/// (i.e. running Night Amplifier Pro). Takes an `Arc<Frame>` because the solve
/// runs on a detached task: sharing the handle avoids copying a full-resolution
/// frame on the stacking thread for a solve that usually will not happen.
pub async fn try_plate_solve(state: &Arc<AppState>, frame: Arc<Frame>) {
    let plugin = match crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) {
        Some(p) => p,
        None => return,
    };

    // Claim the solve slot first. The claim is a compare-and-swap, so it doubles as
    // the "already solving" check that used to be a separate read — two frames
    // arriving together both passed that read and both went on to spawn.
    let latch = {
        let push_to_guard = state.push_to.read().await;
        let Some(ref pt) = *push_to_guard else {
            debug!("Plate solving skipped: Push-To state not initialized in AppState");
            return;
        };
        match pt.try_begin_solve(Instant::now(), MIN_SOLVE_ATTEMPT_INTERVAL) {
            Some(latch) => latch,
            None => {
                debug!("Plate solving skipped: already solving, or offered too recently");
                return;
            }
        }
    };

    let push_to_status = plugin.get_status().await;
    let settings = state.settings.read().await.clone();
    let wanderer_mode = settings.wanderer_mode;

    let has_target = push_to_status.current_target.is_some();
    let solver_ready = push_to_status.solver_ready;

    // This is the authoritative read of the plugin's target state, so use it to
    // correct the cached flag `plate_solve_available` gates on. Without this the
    // mirror could only ever be repaired by an API call.
    state.set_push_to_has_target(has_target).await;

    // Say *why* nothing is happening. Every one of these branches used to log at
    // `debug!` and return, which is what "I installed ASTAP and nothing happens"
    // looks like from the UI. Announced only on a change, so it costs nothing per
    // frame.
    let blocker = if !has_target {
        Some(PushToBlocker::NoTarget)
    } else if !solver_ready {
        Some(PushToBlocker::SolverNotReady)
    } else {
        None
    };
    announce_blocker(state, blocker).await;

    if let Some(blocker) = blocker {
        debug!(reason = blocker.reason(), "Plate solving skipped");
        return;
    }

    // Carried into the spawned task so a successful solve names the target it was
    // solving for; the solve itself does not use it.
    let target_name = push_to_status
        .current_target
        .map(|t| t.name.unwrap_or(t.designation));

    let state_clone = Arc::clone(state);

    tokio::spawn(async move {
        // Moved in so the slot is released when this task ends, however it ends —
        // including a panic inside the plugin. A stranded latch permanently disables
        // plate solving for the rest of the process.
        let _latch = latch;
        let _timer = crate::telemetry::metrics::time_stage(
            crate::telemetry::metrics::FrameStage::PlateSolving,
        );

        // Let the plugin do all the heavy lifting and math
        let plugin = crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN).unwrap();
        let result = plugin
            .process_new_frame(&frame, solve_detector(), wanderer_mode)
            .await;

        match result {
            Ok(outcome) => {
                let fov_deg = outcome.position.as_ref().and_then(|p| {
                    if p.fov_deg > 0.0 {
                        Some(p.fov_deg)
                    } else {
                        None
                    }
                });

                // Only a solve that actually ran is news. Announcing the cached
                // position on every frame filled the log with ~1500 identical
                // "Plate solve succeeded" lines in one session and overwrote any
                // real failure in the UI on the following frame.
                if outcome.outcome == SolveOutcome::Solved {
                    if let Some(pos) = outcome.position {
                        info!(
                            ra = pos.ra_degrees,
                            dec = pos.dec_degrees,
                            stars = pos.stars_matched,
                            target = target_name.as_deref().unwrap_or("-"),
                            "Plate solve succeeded"
                        );

                        let _ = state_clone.events.send(ServerEvent::position_solved(
                            pos.ra_degrees,
                            pos.dec_degrees,
                            pos.ra_string,
                            pos.dec_string,
                            pos.stars_matched,
                            pos.confidence,
                            pos.rotation_deg,
                        ));

                        // The solved FOV is not persisted here. Plate solving is a Pro
                        // feature and the plugin keeps its own solver state, so this stays
                        // out of the Community settings file — it also avoids rewriting
                        // settings.json on every solved frame.
                    }
                }

                if let Some(dir) = outcome.direction {
                    // The direction is recomputed every frame but only changes when
                    // the position or target does, so send it only when it is
                    // actually different.
                    let is_news = {
                        let mut guard = state_clone.push_to.write().await;
                        guard.as_mut().is_none_or(|pt| {
                            pt.direction_is_news(dir.angle_deg, dir.distance_deg, dir.is_close)
                        })
                    };

                    if is_news {
                        info!(
                            celestial_angle = dir.angle_deg,
                            hint = dir.direction_hint,
                            "Push direction calculated"
                        );

                        let _ = state_clone.events.send(ServerEvent::push_direction_updated(
                            dir.angle_deg,
                            dir.distance_deg,
                            dir.direction_hint,
                            dir.is_close,
                            fov_deg,
                        ));
                    }
                }
            }
            Err(PushToError::Cancelled) => {
                // Not a failure: the user asked for it. Reported as its own event so
                // the UI stops the spinner without claiming the sky could not be
                // matched and without discrediting the last known position.
                info!("Plate solve cancelled");
                let _ = state_clone
                    .events
                    .send(ServerEvent::plate_solving_cancelled());
            }
            Err(e) => {
                warn!(error = %e, "Plate solve failed");
                let _ = state_clone
                    .events
                    .send(ServerEvent::position_solve_failed(e.to_string()));
            }
        }
    });
}

/// Broadcast a change in why Push-To is idle, ignoring repeats.
async fn announce_blocker(state: &Arc<AppState>, blocker: Option<PushToBlocker>) {
    let is_news = {
        let mut guard = state.push_to.write().await;
        match guard.as_mut() {
            Some(pt) => pt.blocker_is_news(blocker),
            None => false,
        }
    };
    if is_news {
        let _ = state
            .events
            .send(ServerEvent::push_to_blocked(
                blocker.map(|b| b.reason().to_string()),
            ));
    }
}

/// Abandon any solve in flight, e.g. because the capture pipeline is stopping.
///
/// A solve outlives its frame by design — it runs on a detached task — but nothing
/// used to end one when the session that produced the frame did. The field log for
/// 2026-08-22 has a solve still working through its rungs three minutes after the
/// camera stalled and the pipeline shut down, with the latch held the whole time, so
/// the restarted session could not solve either.
pub async fn abandon_solve_on_shutdown(state: &Arc<AppState>) {
    let Some(plugin) = crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) else {
        return;
    };

    // Clear the "why is nothing happening" notice so the next session starts from a
    // blank slate rather than inheriting this one's last blocker.
    if let Some(ref mut pt) = *state.push_to.write().await {
        pt.blocker_is_news(None);
    }

    match plugin.cancel_solve().await {
        Ok(true) => info!("Capture ended; abandoned the plate solve that was in flight"),
        Ok(false) => {}
        Err(e) => warn!(error = %e, "Could not cancel the in-flight plate solve"),
    }
}
