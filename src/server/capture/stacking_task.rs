use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, info};

use crate::frame::Frame;
use crate::server::state::{AppState, StackingType};
use crate::stacking::CometContext;
use crate::telemetry::metrics as telemetry_metrics;

use super::channel::{CapturedFrame, StackedFrame};
use super::context::{PlanetaryStackingContext, StackingCarryover, StackingContext};
use super::{pipeline, solving, storage};

/// Stacking pipeline running on a dedicated OS thread.
///
/// Receives captured frames, runs star detection, registration, and
/// accumulation. Sends the resulting display frame to the render channel.
/// Owns all stacking contexts exclusively — no shared mutable state.
///
/// `carryover` seeds those contexts from a capture that ended unexpectedly, so
/// a session resumed after a reconnect keeps the integration it had already
/// built rather than starting from one frame.
pub fn run_stacking_task(
    state: Arc<AppState>,
    stacking_rx: mpsc::Receiver<CapturedFrame>,
    render_tx: mpsc::SyncSender<StackedFrame>,
    rt: tokio::runtime::Handle,
    carryover: Option<StackingCarryover>,
) {
    debug!(resumed = carryover.is_some(), "Stacking task started");

    let carryover = carryover.unwrap_or(StackingCarryover {
        stacking: None,
        comet: None,
        planetary: None,
    });
    let mut stacking_ctx: Option<StackingContext> = carryover.stacking;
    let mut comet_ctx: Option<Box<dyn CometContext>> = carryover.comet;
    let mut planetary_ctx: Option<PlanetaryStackingContext> = carryover.planetary;
    let mut stacking_failed = false;
    let mut was_stacking_enabled = false;
    let mut last_stacking_type = StackingType::DeepSky;

    while let Ok(msg) = stacking_rx.recv() {
        let CapturedFrame {
            frame: raw_frame,
            frame_number,
            settings,
            camera_info,
        } = msg;

        // Perform debayering/conversion in the stacking task to keep the camera thread responsive
        let frame = {
            let _span = tracing::info_span!("frame_conversion").entered();
            match raw_frame.to_frame(&camera_info.info) {
                Ok(f) => Arc::new(f),
                Err(e) => {
                    tracing::warn!(error = %e, "Frame conversion failed");
                    rt.block_on(state.frame_rejected(format!("Conversion failed: {}", e)));
                    continue;
                }
            }
        };

        // Detect when stacking is toggled on or stacking type changes — reset context
        let stacking_enabled = settings.stacking && settings.stacking_type.supports_stacking();

        let _iter_span = tracing::info_span!(
            "stacking_iteration",
            frame_number,
            stacking_type = ?settings.stacking_type,
            stacking_enabled,
        )
        .entered();
        let _timer = telemetry_metrics::time_stage(telemetry_metrics::FrameStage::Stack);
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
        let dimension_mismatch =
            check_dimension_mismatch(&frame, &stacking_ctx, &comet_ctx, &planetary_ctx);
        if dimension_mismatch {
            info!("Frame dimensions changed (likely due to binning change), resetting stack");
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            rt.block_on(state.reset_counters());
        }

        // Process frame through stacking pipeline
        let registration_succeeded;
        let mut display_frame = if stacking_enabled && !stacking_failed {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                "Processing frame through stacking pipeline"
            );

            // The pipeline functions expect &Frame — Arc<Frame> derefs transparently
            let (res_frame, matched) = match settings.stacking_type {
                StackingType::Comet => rt.block_on(pipeline::process_frame_with_comet_stacking(
                    &frame,
                    &settings,
                    &mut comet_ctx,
                    &mut stacking_failed,
                )),
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
            Arc::new(res_frame)
        } else {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                stacking_failed = stacking_failed,
                "Stacking disabled or failed, using raw frame"
            );
            registration_succeeded = false;
            Arc::clone(&frame)
        };

        // Fallback to raw frame for live view when registration fails
        if stacking_enabled && !registration_succeeded {
            debug!("Registration failed, falling back to raw frame for live view");
            display_frame = Arc::clone(&frame);
        }

        // Wanderer mode: reset stack if movement detected
        if settings.wanderer_mode && stacking_enabled && !registration_succeeded {
            info!("Wanderer mode: movement detected (registration failed), resetting stack");
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            rt.block_on(state.reset_counters());
            display_frame = Arc::clone(&frame);
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
            false
        };

        // Trigger plate solving asynchronously. Gated up front: without the
        // Push-To plugin the solve is a no-op, and spawning it would keep a
        // second handle on the frame alive long enough to make the render
        // task's `Arc::try_unwrap` fail and copy instead.
        if solving::plate_solve_available(&state) {
            rt.spawn({
                let state = Arc::clone(&state);
                let solve_frame = Arc::clone(&display_frame);
                async move {
                    solving::try_plate_solve(&state, solve_frame).await;
                }
            });
        }

        // Update frame counters
        rt.block_on(state.frame_captured(was_stacked));

        // Release our handle on the captured frame before handing the display
        // frame downstream. On the raw-fallback paths the two are the same
        // allocation, and holding this binding until the end of the iteration
        // would leave the render task looking at a shared `Arc` — making it
        // copy the very frame this indirection exists to avoid.
        drop(frame);

        // Send to render channel (non-blocking — skip if render is busy)
        let render_msg = StackedFrame {
            display_frame,
            was_stacked,
            frame_number,
            settings,
        };
        if let Err(mpsc::TrySendError::Disconnected(_)) = render_tx.try_send(render_msg) {
            debug!("Render channel disconnected, stopping stacking task");
            break;
        }
    }

    // Save stacked result before exiting
    save_stacked_result(&state, &stacking_ctx, &comet_ctx, &planetary_ctx, &rt);

    // Park the accumulators in case this capture is about to be resumed after a
    // reconnect. A fresh start or a clean stop clears them; see
    // `CaptureService::start_capture` and `stop_capture`.
    *state
        .stacking_carryover
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(StackingCarryover {
        stacking: stacking_ctx,
        comet: comet_ctx,
        planetary: planetary_ctx,
    });

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
