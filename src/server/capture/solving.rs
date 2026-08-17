use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::detection::{DetectionConfig, StarDetector};
use crate::frame::Frame;
use crate::server::events::ServerEvent;
use crate::server::state::AppState;

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
            if pt.solving_in_progress || !pt.has_target {
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
    // 1. Check local state first - cheap and prevents blocking on plugin status if already solving
    {
        let push_to_guard = state.push_to.read().await;
        if let Some(ref pt) = *push_to_guard {
            if pt.solving_in_progress {
                debug!("Plate solving skipped: solving already in progress (local check)");
                return;
            }
        } else {
            debug!("Plate solving skipped: Push-To state not initialized in AppState");
            return;
        }
    }

    let plugin = match crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN) {
        Some(p) => p,
        None => return,
    };

    // 2. Now check if solver is ready and target is set via plugin status
    let push_to_status = plugin.get_status().await;
    let settings = state.settings.read().await.clone();
    let wanderer_mode = settings.wanderer_mode;

    // Double check solving state from plugin just in case, and check target/ready
    let has_target = push_to_status.current_target.is_some();
    let solver_ready = push_to_status.solver_ready;
    let plugin_is_solving = push_to_status.is_solving;

    // This is the authoritative read of the plugin's target state, so use it to
    // correct the cached flag `plate_solve_available` gates on. Without this the
    // mirror could only ever be repaired by an API call.
    state.set_push_to_has_target(has_target).await;

    if !has_target {
        debug!("Plate solving skipped: no target set");
        return;
    }
    if !solver_ready {
        debug!("Plate solving skipped: solver not ready (database/binary missing)");
        return;
    }
    if plugin_is_solving {
        debug!("Plate solving skipped: plugin reports solving already in progress");
        return;
    }

    // Get target name for logging
    let target_name = push_to_status
        .current_target
        .map(|t| t.name.unwrap_or(t.designation));

    // Mark as solving
    {
        let mut push_to_guard = state.push_to.write().await;
        if let Some(ref mut pt) = *push_to_guard {
            pt.solving_in_progress = true;
        }
    }

    let state_clone = Arc::clone(state);

    tokio::spawn(async move {
        let _timer = crate::telemetry::metrics::time_stage(crate::telemetry::metrics::FrameStage::PlateSolving);

        // Plate solving consistent detector
        let detector = StarDetector::new(DetectionConfig::sensitive().with_max_stars(200));

        // Let the plugin do all the heavy lifting and math
        let plugin = crate::license::pro_plugin(&crate::push_to::PUSH_TO_PLUGIN).unwrap();
        let result = plugin
            .process_new_frame(&frame, &detector, wanderer_mode)
            .await;

        {
            let mut push_to_guard = state_clone.push_to.write().await;
            if let Some(ref mut pt) = *push_to_guard {
                pt.solving_in_progress = false;
            }
        }

        match result {
            Ok((pos_opt, dir_opt)) => {
                let fov_deg = pos_opt.as_ref().and_then(|p| {
                    if p.fov_deg > 0.0 {
                        Some(p.fov_deg)
                    } else {
                        None
                    }
                });

                if let Some(pos) = pos_opt {
                    info!(
                        ra = pos.ra_degrees,
                        dec = pos.dec_degrees,
                        stars = pos.stars_matched,
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

                if let Some(dir) = dir_opt {
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
            Err(e) => {
                warn!(error = %e, "Plate solve failed");
                let _ = state_clone
                    .events
                    .send(ServerEvent::position_solve_failed(e.to_string()));
            }
        }
    });
}
