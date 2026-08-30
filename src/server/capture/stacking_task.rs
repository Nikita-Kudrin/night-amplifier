use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, info};

use crate::cfa::CfaPipeline;
use crate::debayer::DebayerAlgorithm;
use crate::frame::Frame;
use crate::server::state::{AppState, SensorCorrectionSettings, StackingType};
use crate::stacking::CometContext;
use crate::telemetry::metrics as telemetry_metrics;

use super::channel::{CapturedFrame, StackedFrame};
use super::frame_gate::RejectionReason;
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

    // The raw-CFA stage, rebuilt only when what it is derived from moves: a stage
    // may own precomputed state, so it must not be reconstructed per frame. The
    // key carries `stacking_type` as well as the correction settings because the
    // FPN stage is gated on it — switching to Planetary has to drop that stage
    // even though no sensor setting changed.
    let mut cfa_stage_key: Option<(SensorCorrectionSettings, StackingType)> = None;
    let mut cfa_pipeline = CfaPipeline::new();
    let mut debayer = DebayerAlgorithm::Bilinear;

    while let Ok(msg) = stacking_rx.recv() {
        let CapturedFrame {
            frame: raw_frame,
            frame_number,
            settings,
            camera_info,
        } = msg;

        let stage_key = (settings.sensor_correction.clone(), settings.stacking_type);
        if cfa_stage_key.as_ref() != Some(&stage_key) {
            cfa_stage_key = Some(stage_key);
            cfa_pipeline = pipeline::build_cfa_pipeline(&settings);
            debayer = pipeline::debayer_algorithm(&settings);
            info!(
                stages = ?cfa_pipeline.stage_names(),
                ?debayer,
                stacking_type = ?settings.stacking_type,
                "Raw-CFA stage configured"
            );
        }

        // Decode, correct on the mosaic, then debayer — all in the stacking task
        // so the camera thread stays free to start the next exposure.
        let frame = {
            let _span = tracing::info_span!("frame_conversion").entered();
            match pipeline::convert_captured_frame(
                &raw_frame,
                &camera_info.info,
                &cfa_pipeline,
                debayer,
            ) {
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
        let mut showing_stack;
        let stack_reset;
        let mut rejected_because;
        let mut display_frame = if stacking_enabled && !stacking_failed {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                "Processing frame through stacking pipeline"
            );

            // The pipeline functions expect &Frame — Arc<Frame> derefs transparently
            let outcome = match settings.stacking_type {
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
            registration_succeeded = outcome.frame_added;
            showing_stack = outcome.showing_stack;
            stack_reset = outcome.stack_reset;
            rejected_because = outcome.rejected_because;
            Arc::new(outcome.display_frame)
        } else {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                stacking_failed = stacking_failed,
                "Stacking disabled or failed, using raw frame"
            );
            registration_succeeded = false;
            showing_stack = false;
            stack_reset = false;
            rejected_because = None;
            Arc::clone(&frame)
        };

        // The stack restarted on a sharper reference, so the integration the
        // counters describe no longer exists.
        if stack_reset {
            rt.block_on(state.reset_counters());
        }

        // Note there is deliberately no raw-frame fallback for a frame that
        // merely failed to register — see `StackingOutcome`.

        // Wanderer mode: reset stack if movement detected
        if wanderer_detected_movement(
            settings.wanderer_mode,
            stacking_enabled,
            registration_succeeded,
            rejected_because,
        ) {
            info!(
                reason = rejected_because.map(|r| r.describe()).unwrap_or("registration failed"),
                "Wanderer mode: movement detected, resetting stack"
            );
            stacking_ctx = None;
            comet_ctx = None;
            planetary_ctx = None;
            rt.block_on(state.reset_counters());
            display_frame = Arc::clone(&frame);
            showing_stack = false;
            // The stack this frame failed against no longer exists, so the
            // verdict against it describes nothing the user can act on. Wanderer
            // treats a failed registration as the *signal*, not as a fault.
            rejected_because = None;
        }

        // Whether *this* frame joined the stack. Deriving it from the context's
        // frame count instead would report every frame after the first as
        // stacked, leaving the UI's rejection counter pinned at zero.
        let was_stacked = stacking_enabled && registration_succeeded;

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

        // Update frame counters. The reason rides on `frame_captured`, never on
        // `frame_rejected` — that one feeds the capture-abort burst detector and
        // is for a camera that failed to deliver a frame at all.
        rt.block_on(state.frame_captured(
            was_stacked,
            rejected_because.map(|reason| reason.describe()),
        ));

        // Release our handle on the captured frame before handing the display
        // frame downstream. On the raw-fallback paths the two are the same
        // allocation, and holding this binding until the end of the iteration
        // would leave the render task looking at a shared `Arc` — making it
        // copy the very frame this indirection exists to avoid.
        drop(frame);

        // Send to render channel (non-blocking — skip if render is busy)
        let render_msg = StackedFrame {
            display_frame,
            showing_stack,
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

/// Whether Wanderer mode should treat this frame as the user having moved the
/// telescope, and start the stack again.
///
/// Only a frame that could not be placed against the reference counts. Before
/// the frame gate existed every rejection meant exactly that, so the condition
/// was simply "did not stack"; the gate also rejects frames that aligned
/// perfectly well but were soft or loose, and resetting on those hands the user
/// a stack that restarts every time a cloud crosses.
///
/// A mode that reports no reason (comet, planetary) keeps the original
/// behaviour: not stacking is the only signal available.
fn wanderer_detected_movement(
    wanderer_mode: bool,
    stacking_enabled: bool,
    registration_succeeded: bool,
    rejected_because: Option<RejectionReason>,
) -> bool {
    if !wanderer_mode || !stacking_enabled || registration_succeeded {
        return false;
    }
    rejected_because.is_none_or(|reason| reason.means_the_sky_moved())
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
    use super::wanderer_detected_movement as moved;
    use super::RejectionReason;

    #[test]
    fn test_check_dimension_mismatch_no_context() {
        let frame = crate::frame::Frame::zeros(100, 100, 3).unwrap();
        assert!(!super::check_dimension_mismatch(
            &frame, &None, &None, &None
        ));
    }

    #[test]
    fn wanderer_resets_when_the_frame_cannot_be_placed_at_all() {
        for reason in [
            RejectionReason::NoStars,
            RejectionReason::TooFewStars,
            RejectionReason::RegistrationFailed,
            RejectionReason::TooFewCorrespondences,
        ] {
            assert!(
                moved(true, true, false, Some(reason)),
                "{reason:?} means the field no longer matches the reference"
            );
        }
    }

    /// The regression the frame gate introduced: it rejects frames that aligned
    /// perfectly well but were soft or loose, and Wanderer read every rejection
    /// as the user having swung the scope. A cloud crossing would restart the
    /// stack, which is the opposite of what the mode is for.
    #[test]
    fn wanderer_holds_the_stack_through_a_cloud() {
        for reason in [
            RejectionReason::ResidualTooHigh,
            RejectionReason::StarsTooLarge,
            RejectionReason::StackerError,
        ] {
            assert!(
                !moved(true, true, false, Some(reason)),
                "{reason:?} is a bad frame, not a new target"
            );
        }
    }

    #[test]
    fn wanderer_leaves_a_stacked_frame_alone() {
        assert!(!moved(true, true, true, None));
    }

    /// Comet and planetary report no reason, so "did not stack" stays the only
    /// signal available to them.
    #[test]
    fn a_mode_without_reasons_keeps_the_original_wanderer_behaviour() {
        assert!(moved(true, true, false, None));
    }

    #[test]
    fn wanderer_does_nothing_when_it_is_off_or_stacking_is_not_running() {
        assert!(!moved(false, true, false, None), "wanderer mode is off");
        assert!(!moved(true, false, false, None), "stacking is not running");
    }
}
