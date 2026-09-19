use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::detection::{DetectionConfig, StarDetector};
use crate::frame::Frame;
use crate::push_to::{PushToBlocker, PushToError, SolveOutcome};
use crate::server::events::ServerEvent;
use crate::server::state::AppState;

use super::push_to_tasks::{self, Lane};

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

/// The detector used for the movement check and for the stars handed to ASTAP.
///
/// Shared rather than rebuilt per frame: it is stateless configuration, and the
/// settings must not drift between the movement comparison and the solve that
/// follows it.
fn solve_detector() -> &'static StarDetector {
    static DETECTOR: OnceLock<StarDetector> = OnceLock::new();
    DETECTOR.get_or_init(|| StarDetector::new(DetectionConfig::sensitive().with_max_stars(200)))
}

/// Which camera is offering the frame.
///
/// Exactly one of them may solve at a time, and a connected guide camera wins: it is on
/// its own scope with its own exposure, free-running while the imaging camera is
/// mid-sub, so it can offer the solver a fresh star field far more often. Letting both
/// offer would not double the solve rate — the latches admit one — it would just make
/// which camera got there first, and therefore which optics the solve was planned
/// against, a race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolveSource {
    /// The imaging pipeline's stacking task.
    Main,
    /// The guide camera's free-running loop.
    Guide,
}

impl SolveSource {
    /// Whether this source is the one currently allowed to solve.
    ///
    /// Keyed on the guide loop *running* (or being recovered), not on a guide camera
    /// being connected: a camera that is registered but not exposing — mid warm-up, or
    /// with a loop that failed to start — must hand solving back rather than leave the
    /// session with no source at all. See `AppState::guide_holds_solving`.
    fn is_active(self, guide_holds_solving: bool) -> bool {
        match self {
            Self::Main => !guide_holds_solving,
            Self::Guide => guide_holds_solving,
        }
    }
}

/// Whether a frame offered now from `source` would be taken.
///
/// Callers check this *before* preparing a frame, so the common case — Community
/// edition, Pro with no target, or a Push-To task still busy with the last frame — costs
/// an atomic load instead of a frame conversion or a second frame handle (which makes
/// the render task's `Arc::try_unwrap` fail and copy). Advisory: the claims in
/// [`solve_frame`] and [`watch_frame`] still decide.
pub fn plate_solve_available(state: &Arc<AppState>, source: SolveSource) -> bool {
    if !source.is_active(state.guide_holds_solving()) {
        return false;
    }

    if crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN).is_none() {
        return false;
    }

    // `try_read`: this runs on the capture threads. A writer holds it for microseconds,
    // and declining one frame then is cheaper than waiting.
    //
    // A solve in flight is deliberately *not* a reason to decline: the frame goes to the
    // movement watch, which must see a slew to abandon a search working on sky we left.
    let Ok(guard) = state.push_to.try_read() else {
        return false;
    };
    let Some(pt) = guard.as_ref() else {
        return false;
    };
    pt.has_target
        && pt.offer_is_due(Instant::now(), MIN_SOLVE_ATTEMPT_INTERVAL, MIN_WATCH_INTERVAL)
        && push_to_tasks::is_idle(state, Lane::for_solving(pt.is_solving()))
}

/// Hand `frame` to the Push-To task that can use it now — the solve task, or the
/// movement watch while a solve runs — or drop it if that task is busy. Never queues.
/// Call once [`plate_solve_available`] says the offer could do something.
pub fn offer_plate_solve(
    state: &Arc<AppState>,
    rt: &tokio::runtime::Handle,
    frame: Arc<Frame>,
    source: SolveSource,
) -> bool {
    let lane = match state.push_to.try_read() {
        Ok(guard) => match guard.as_ref() {
            Some(pt) => Lane::for_solving(pt.is_solving()),
            None => return false,
        },
        Err(_) => return false,
    };
    push_to_tasks::offer(state, rt, lane, frame, source)
}

/// Run the movement watch on `frame` while a solve is in flight: the Push-To watch task's
/// job. Returns without looking if no solve is running, the watch ran too recently, or
/// `source` stopped being the solve source.
///
/// `source` is re-checked immediately before dispatch. Neither
/// `PushToSolverPlugin::observe_frame` nor `process_new_frame` is told which camera
/// produced its frame — `ProPushToPlugin::look()` scales and mutates the one shared
/// `MovementDetector` for whatever arrives — so a frame from a camera that stopped being
/// the source (a guide camera connected or disconnected since the offer) reads as the new
/// rig's telescope having moved and can abort a solve that just started.
pub async fn watch_frame(state: &Arc<AppState>, frame: Arc<Frame>, source: SolveSource) {
    if !source.is_active(state.guide_holds_solving()) {
        debug!(?source, "Plate solve watch skipped: no longer the active solve source");
        return;
    }
    let Some(plugin) = crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) else {
        return;
    };

    let _watch = {
        let push_to_guard = state.push_to.read().await;
        let Some(ref pt) = *push_to_guard else {
            return;
        };
        if !pt.is_solving() {
            debug!("Plate solve watch skipped: the solve it was offered for has ended");
            return;
        }
        match pt.try_begin_watch(Instant::now(), MIN_WATCH_INTERVAL) {
            Some(watch) => watch,
            None => return,
        }
    };

    let wanderer_mode = state.settings.read().await.wanderer_mode;
    if !source.is_active(state.guide_holds_solving()) {
        debug!(?source, "Plate solve watch skipped: no longer the active solve source");
        return;
    }
    match plugin
        .observe_frame(&frame, solve_detector(), wanderer_mode)
        .await
    {
        Ok(outcome) => announce_blocker(state, outcome.blocker).await,
        Err(e) => debug!(error = %e, "Movement watch failed on this frame"),
    }
}

/// Try to plate solve `frame` if a target is set and the solver is ready: the Push-To
/// solve task's job. Returns once the solve (or the decision not to run one) is done.
///
/// In the Community edition this does nothing unless the Push-To plugin is installed.
/// `source` is re-checked before each dispatch — see [`watch_frame`] — because the solve
/// path crosses further `.await` points (`get_status`) of its own.
pub async fn solve_frame(state: &Arc<AppState>, frame: Arc<Frame>, source: SolveSource) {
    if !source.is_active(state.guide_holds_solving()) {
        debug!(?source, "Plate solve skipped: no longer the active solve source");
        return;
    }

    let plugin = match crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) {
        Some(p) => p,
        None => return,
    };

    // The claim is a compare-and-swap, so it doubles as the "already busy" check: two
    // frames arriving together cannot both start a solve. Held until this returns,
    // however it returns — a stranded latch disables plate solving for good.
    let _latch = {
        let push_to_guard = state.push_to.read().await;
        let Some(ref pt) = *push_to_guard else {
            debug!("Plate solving skipped: Push-To state not initialized in AppState");
            return;
        };
        match pt.try_begin_solve(Instant::now(), MIN_SOLVE_ATTEMPT_INTERVAL) {
            Some(latch) => latch,
            None => {
                debug!("Plate solving skipped: a solve is running or one was offered too recently");
                return;
            }
        }
    };

    let wanderer_mode = state.settings.read().await.wanderer_mode;
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

    // Carried so a successful solve names the target it was solving for; the solve
    // itself does not use it.
    let target_name = push_to_status
        .current_target
        .map(|t| t.name.unwrap_or(t.designation));

    let _timer =
        crate::telemetry::metrics::time_stage(crate::telemetry::metrics::FrameStage::PlateSolving);

    // The rig may have changed while `get_status` was awaited.
    if !source.is_active(state.guide_holds_solving()) {
        debug!(?source, "Plate solve dispatch skipped: no longer the active solve source");
        return;
    }

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

                    let _ = state.events.send(ServerEvent::position_solved(
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
            announce_blocker(state, outcome.blocker).await;

            if let Some(dir) = outcome.direction {
                // The direction is recomputed every frame but only changes when
                // the position or target does, so send it only when it is
                // actually different.
                let is_news = {
                    let mut guard = state.push_to.write().await;
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

                    let _ = state.events.send(ServerEvent::push_direction_updated(
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
            let _ = state
                .events
                .send(ServerEvent::plate_solving_cancelled());
        }
        Err(e) => {
            warn!(error = %e, "Plate solve failed");
            let _ = state
                .events
                .send(ServerEvent::position_solve_failed(e.to_string()));
        }
    }
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

    // ---- SolveSource::is_active: the predicate every staleness re-check rests on --

    #[test]
    fn main_is_active_only_while_no_guide_loop_is_running() {
        assert!(SolveSource::Main.is_active(false));
        assert!(!SolveSource::Main.is_active(true));
    }

    #[test]
    fn guide_is_active_only_while_its_loop_is_running() {
        assert!(!SolveSource::Guide.is_active(false));
        assert!(SolveSource::Guide.is_active(true));
    }

    #[test]
    fn the_two_sources_are_never_both_active_at_once() {
        // The invariant the re-checks in `solve_frame` and `watch_frame` lean on: whichever way
        // `guide_loop_running` reads, at most one source may proceed to dispatch.
        for guide_loop_running in [false, true] {
            assert!(
                !(SolveSource::Main.is_active(guide_loop_running)
                    && SolveSource::Guide.is_active(guide_loop_running)),
                "guide_loop_running={guide_loop_running}"
            );
        }
    }
}
