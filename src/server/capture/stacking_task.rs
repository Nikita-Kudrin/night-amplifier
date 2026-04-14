use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, info};

use crate::frame::Frame;
use crate::server::state::{AppState, StackingType};
use crate::stacking::CometContext;

use super::channel::{CapturedFrame, StackedFrame};
use super::context::{PlanetaryStackingContext, StackingContext};
use super::{pipeline, solving, storage};

/// Stacking pipeline running on a dedicated OS thread.
///
/// Receives captured frames, runs star detection, registration, and
/// accumulation. Sends the resulting display frame to the render channel.
/// Owns all stacking contexts exclusively — no shared mutable state.
pub fn run_stacking_task(
    state: Arc<AppState>,
    stacking_rx: mpsc::Receiver<CapturedFrame>,
    render_tx: mpsc::SyncSender<StackedFrame>,
    rt: tokio::runtime::Handle,
) {
    debug!("Stacking task started");

    let mut stacking_ctx: Option<StackingContext> = None;
    let mut comet_ctx: Option<Box<dyn CometContext>> = None;
    let mut planetary_ctx: Option<PlanetaryStackingContext> = None;
    let mut stacking_failed = false;
    let mut was_stacking_enabled = false;
    let mut last_stacking_type = StackingType::DeepSky;

    while let Ok(msg) = stacking_rx.recv() {
        let CapturedFrame {
            frame,
            settings,
            ..
        } = msg;

        // Detect when stacking is toggled on or stacking type changes — reset context
        let stacking_enabled = settings.stacking && settings.stacking_type.supports_stacking();
        let stacking_type_changed = settings.stacking_type != last_stacking_type;

        if (stacking_enabled && !was_stacking_enabled)
            || (stacking_enabled && stacking_type_changed)
        {
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            stacking_failed = false;
            rt.block_on(state.reset_counters());
            info!(
                stacking_type = ?settings.stacking_type,
                "Live stacking enabled/changed, resetting context and counters"
            );
        }
        was_stacking_enabled = stacking_enabled;
        last_stacking_type = settings.stacking_type;

        // Check frame dimension mismatch (e.g. after binning change)
        let dimension_mismatch = check_dimension_mismatch(
            &frame,
            &stacking_ctx,
            &comet_ctx,
            &planetary_ctx,
        );
        if dimension_mismatch {
            info!("Frame dimensions changed (likely due to binning change), resetting stack");
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            rt.block_on(state.reset_counters());
        }

        // Process frame through stacking pipeline
        let mut registration_succeeded = true;
        let mut display_frame = if stacking_enabled && !stacking_failed {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                "Processing frame through stacking pipeline"
            );

            // The pipeline functions expect &Frame — Arc<Frame> derefs transparently
            let (res_frame, matched) = match settings.stacking_type {
                StackingType::Comet => {
                    rt.block_on(pipeline::process_frame_with_comet_stacking(
                        &frame,
                        &settings,
                        &mut comet_ctx,
                        &mut stacking_failed,
                    ))
                }
                StackingType::Planetary => {
                    rt.block_on(pipeline::process_frame_with_planetary_stacking(
                        &frame,
                        &settings,
                        &mut planetary_ctx,
                        &mut stacking_failed,
                    ))
                }
                _ => rt.block_on(pipeline::process_frame_with_stacking(
                    &frame,
                    &settings,
                    &mut stacking_ctx,
                    &mut stacking_failed,
                )),
            };
            registration_succeeded = matched;
            res_frame
        } else {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                stacking_failed = stacking_failed,
                "Stacking disabled or failed, using raw frame"
            );
            registration_succeeded = false;
            frame.as_ref().clone()
        };

        // Fallback to raw frame for live view when registration fails
        if stacking_enabled && !registration_succeeded {
            debug!("Registration failed, falling back to raw frame for live view");
            display_frame = frame.as_ref().clone();
        }

        // Wanderer mode: reset stack if movement detected
        if settings.wanderer_mode && stacking_enabled && !registration_succeeded {
            info!("Wanderer mode: movement detected (registration failed), resetting stack");
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            rt.block_on(state.reset_counters());
            display_frame = frame.as_ref().clone();
        }

        // Track whether this frame was successfully stacked
        let was_stacked = if stacking_enabled {
            match settings.stacking_type {
                StackingType::Comet => comet_ctx
                    .as_ref()
                    .map(|ctx| ctx.frame_count() > 0)
                    .unwrap_or(false),
                StackingType::Planetary => planetary_ctx
                    .as_ref()
                    .map(|ctx| ctx.frame_count() > 0)
                    .unwrap_or(false),
                _ => stacking_ctx
                    .as_ref()
                    .map(|ctx| ctx.frame_count() > 0)
                    .unwrap_or(false),
            }
        } else {
            true
        };

        // Trigger plate solving asynchronously
        rt.spawn({
            let state = Arc::clone(&state);
            let solve_frame = display_frame.clone();
            async move {
                solving::try_plate_solve(&state, &solve_frame).await;
            }
        });

        // Update frame counters
        rt.block_on(state.frame_captured(was_stacked));

        // Send to render channel (non-blocking — skip if render is busy)
        let render_msg = StackedFrame {
            display_frame,
            was_stacked,
            settings,
        };
        if let Err(mpsc::TrySendError::Disconnected(_)) = render_tx.try_send(render_msg) {
            debug!("Render channel disconnected, stopping stacking task");
            break;
        }
    }

    // Save stacked result before exiting
    save_stacked_result(&state, &stacking_ctx, &comet_ctx, &planetary_ctx, &rt);

    debug!("Stacking task ended");
}

/// Check if frame dimensions match any existing stacking context.
fn check_dimension_mismatch(
    frame: &Frame,
    stacking_ctx: &Option<StackingContext>,
    comet_ctx: &Option<Box<dyn CometContext>>,
    planetary_ctx: &Option<PlanetaryStackingContext>,
) -> bool {
    if let Some(ctx) = stacking_ctx.as_ref() {
        return frame.width() != ctx.width()
            || frame.height() != ctx.height()
            || frame.channels() != ctx.channels();
    }
    if let Some(ctx) = comet_ctx.as_ref() {
        return frame.width() != ctx.width()
            || frame.height() != ctx.height()
            || frame.channels() != ctx.channels();
    }
    if let Some(ctx) = planetary_ctx.as_ref() {
        return frame.width() != ctx.width()
            || frame.height() != ctx.height()
            || frame.channels() != ctx.channels();
    }
    false
}

/// Save the final stacked result at the end of a capture session.
fn save_stacked_result(
    state: &Arc<AppState>,
    stacking_ctx: &Option<StackingContext>,
    comet_ctx: &Option<Box<dyn CometContext>>,
    planetary_ctx: &Option<PlanetaryStackingContext>,
    rt: &tokio::runtime::Handle,
) {
    let stacked_frame = stacking_ctx
        .as_ref()
        .and_then(|ctx| ctx.compute().ok())
        .or_else(|| comet_ctx.as_ref().and_then(|ctx| ctx.compute().ok()))
        .or_else(|| planetary_ctx.as_ref().and_then(|ctx| ctx.compute().ok()));

    if let Some(frame) = stacked_frame {
        let camera_info = rt.block_on(async {
            let cameras = state.cameras.read().await;
            cameras.values().next().cloned()
        });
        if let Some(info) = camera_info {
            rt.block_on(storage::save_stacked_result(state, Some(frame), &info));
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_check_dimension_mismatch_no_context() {
        let frame = crate::frame::Frame::zeros(100, 100, 3).unwrap();
        assert!(!super::check_dimension_mismatch(
            &frame, &None, &None, &None
        ));
    }
}
