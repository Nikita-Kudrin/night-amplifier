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

/// Shortest gap between two frames being offered to the movement watch that runs
/// *while* a solve is in flight.
///
/// Deliberately slower than the solve offer: this exists to notice a slew, and a slew
/// takes seconds. Every run costs a full sensitive detection over the whole sensor,
/// and it competes with the ASTAP process it may be about to abandon.
const MIN_WATCH_INTERVAL: Duration = Duration::from_millis(1500);

/// Which slot this frame claimed, and therefore what it is allowed to do.
enum Claim {
    /// Nothing was running: this frame may start a solve and wait for it.
    Solve(crate::server::services::SolveLatch),
    /// A solve is running: this frame may only look, and must return promptly.
    Watch(crate::server::services::WatchLatch),
}

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
    // Without a target there is nothing to do, so the heavy tokio task and the extra
    // frame handle are both skipped.
    //
    // A solve being in flight is deliberately *not* a reason to skip any more. It was,
    // and the consequence was that the movement detector saw nothing for the whole
    // length of a solve: a slew could not arm the gate, could not abandon a search
    // already working on sky we had left, and could not update the status. See
    // `PushToSolverPlugin::observe_frame` — the in-flight path is cheap and returns
    // promptly, so it can afford to run.
    if let Ok(guard) = state.push_to.try_read() {
        if let Some(ref pt) = *guard {
            if !pt.has_target {
                return false;
            }
            if !pt.offer_is_due(
                Instant::now(),
                MIN_SOLVE_ATTEMPT_INTERVAL,
                MIN_WATCH_INTERVAL,
            ) {
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

    // Claim a slot first. The claim is a compare-and-swap, so it doubles as the
    // "already busy" check that used to be a separate read — two frames arriving
    // together both passed that read and both went on to spawn.
    //
    // Which slot depends on whether a solve is already running: the solve path may
    // block for the length of an ASTAP ladder, the watch path may not.
    let claim = {
        let push_to_guard = state.push_to.read().await;
        let Some(ref pt) = *push_to_guard else {
            debug!("Plate solving skipped: Push-To state not initialized in AppState");
            return;
        };
        let now = Instant::now();
        if pt.is_solving() {
            match pt.try_begin_watch(now, MIN_WATCH_INTERVAL) {
                Some(watch) => Claim::Watch(watch),
                None => return,
            }
        } else {
            match pt.try_begin_solve(now, MIN_SOLVE_ATTEMPT_INTERVAL) {
                Some(latch) => Claim::Solve(latch),
                None => {
                    debug!("Plate solving skipped: offered too recently");
                    return;
                }
            }
        }
    };

    let settings = state.settings.read().await.clone();
    let wanderer_mode = settings.wanderer_mode;

    // The watch path exists to notice a slew and abandon a solve that can no longer
    // be right. It deliberately skips the target/readiness round-trip below: a solve
    // is already running, so both were true a moment ago, and this path is on a clock
    // that is competing with ASTAP for the machine.
    if let Claim::Watch(watch) = claim {
        let _watch = watch;
        match plugin
            .observe_frame(&frame, solve_detector(), wanderer_mode)
            .await
        {
            Ok(outcome) => announce_blocker(state, outcome.blocker).await,
            Err(e) => debug!(error = %e, "Movement watch failed on this frame"),
        }
        return;
    }
    let Claim::Solve(latch) = claim else {
        unreachable!("the watch arm returns above")
    };

    let push_to_status = plugin.get_status().await;

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
                            stars = ?pos.stars_detected,
                            target = target_name.as_deref().unwrap_or("-"),
                            "Plate solve succeeded"
                        );

                        let _ = state_clone.events.send(ServerEvent::position_solved(
                            pos.ra_degrees,
                            pos.dec_degrees,
                            pos.ra_string,
                            pos.dec_string,
                            pos.stars_detected,
                            pos.confidence,
                            pos.rotation_deg,
                        ));

                        // The solved FOV is not persisted here. Plate solving is a Pro
                        // feature and the plugin keeps its own solver state, so this stays
                        // out of the Community settings file — it also avoids rewriting
                        // settings.json on every solved frame.
                    }
                }

                // Say why nothing is happening — "telescope is moving", "waiting for
                // the view to settle" — through the same de-duplication as every
                // other blocker, so a state that holds for a hundred frames costs one
                // event. A solve that ran clears it by reporting `None`.
                announce_blocker(&state_clone, outcome.blocker).await;

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
    // blank slate rather than inheriting this one's last blocker. Through
    // `announce_blocker`, not by poking the de-duplication record directly: that
    // updated the server's idea of what clients had been told without telling them
    // anything, so the last blocker stayed on screen until the next transition —
    // and a blocker now outranks the last solve verdict in the UI.
    announce_blocker(state, None).await;

    match plugin.cancel_solve().await {
        Ok(true) => info!("Capture ended; abandoned the plate solve that was in flight"),
        Ok(false) => {}
        Err(e) => warn!(error = %e, "Could not cancel the in-flight plate solve"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::services::PushToState;

    async fn state_with_push_to() -> Arc<AppState> {
        let (state, _disk_writer) = AppState::new_for_testing();
        let state = Arc::new(state);
        *state.push_to.write().await = Some(PushToState::default());
        state
    }

    #[tokio::test]
    async fn clearing_the_blocker_tells_the_clients_and_not_just_the_bookkeeping() {
        // `abandon_solve_on_shutdown` used to poke `blocker_is_news` for its side
        // effect and drop the result, so the server recorded that clients had been
        // told "nothing is blocking" without ever sending it. The last blocker then
        // stayed on screen indefinitely — and it now outranks the last solve verdict.
        let state = state_with_push_to().await;
        let mut events = state.events.subscribe();

        announce_blocker(&state, Some(PushToBlocker::TelescopeMoving)).await;
        assert!(matches!(
            events.try_recv(),
            Ok(ServerEvent::PushToBlocked { .. })
        ));

        announce_blocker(&state, None).await;
        match events.try_recv() {
            Ok(ServerEvent::PushToBlocked { reason }) => assert_eq!(reason, None),
            other => panic!("the clear must reach the bus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_blocker_that_has_not_changed_is_not_re_sent() {
        let state = state_with_push_to().await;
        let mut events = state.events.subscribe();

        announce_blocker(&state, Some(PushToBlocker::Settling)).await;
        let _ = events.try_recv().expect("the first one is news");

        for _ in 0..5 {
            announce_blocker(&state, Some(PushToBlocker::Settling)).await;
        }
        assert!(
            events.try_recv().is_err(),
            "a state that holds for a hundred frames must cost one event"
        );
    }
}
