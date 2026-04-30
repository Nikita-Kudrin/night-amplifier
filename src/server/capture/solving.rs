use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::detection::{DetectionConfig, StarDetector};
use crate::frame::Frame;
use crate::server::events::ServerEvent;
use crate::server::state::AppState;

/// Try to plate solve if a target is set and solver is ready
///
/// In the Community edition, this does nothing unless the Push-To plugin is installed
/// (i.e. running Night Amplifier Pro).
pub async fn try_plate_solve(state: &Arc<AppState>, frame: &Frame) {
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

    let plugin = match crate::push_to::PUSH_TO_PLUGIN.get() {
        Some(p) => p,
        None => return, // Do nothing if Push-To is not available
    };

    // 2. Now check if solver is ready and target is set via plugin status
    let push_to_status = plugin.get_status().await;

    // Double check solving state from plugin just in case, and check target/ready
    let has_target = push_to_status.current_target.is_some();
    let solver_ready = push_to_status.solver_ready;
    let plugin_is_solving = push_to_status.is_solving;

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

    // Get target name for the event (prefer common name)
    let target_name = push_to_status.current_target.map(|t| t.name.unwrap_or(t.designation));

    // Mark as solving
    {
        let mut push_to_guard = state.push_to.write().await;
        if let Some(ref mut pt) = *push_to_guard {
            pt.solving_in_progress = true;
        }
    }

    info!(target_name = ?target_name, "Broadcasting plate_solving_started event");
    let _ = state
        .events
        .send(ServerEvent::plate_solving_started(target_name.clone()));

    let frame_clone = frame.clone();
    let state_clone = Arc::clone(state);

    tokio::spawn(async move {
        // Plate solving consistent detector
        let detector = StarDetector::new(DetectionConfig::sensitive().with_max_stars(200));

        // Let the plugin do all the heavy lifting and math
        let plugin = crate::push_to::PUSH_TO_PLUGIN.get().unwrap();
        let result = plugin.process_new_frame(&frame_clone, &detector).await;

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

                    let pos_fov = pos.fov_deg;
                    // If FOV was unknown, save it to settings now that we have it
                    if pos_fov > 0.0 {
                        tokio::spawn({
                            let state = Arc::clone(&state_clone);
                            async move {
                                {
                                    let mut settings = state.settings.write().await;
                                    settings.push_to_fov = Some(pos_fov as f32);
                                }
                                state.save_settings().await;
                            }
                        });
                    }
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
