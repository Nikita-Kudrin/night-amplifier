use std::sync::mpsc;
use std::sync::Arc;
use tracing::{debug, info};

use night_amplifier_core::cfa::CfaPipeline;
use night_amplifier_core::cfa::SensorCorrectionSettings;
use night_amplifier_core::debayer::DebayerAlgorithm;
use night_amplifier_core::frame::Frame;
use crate::state::{AppState, CameraRole, StackingType};
use night_amplifier_core::telemetry::metrics as telemetry_metrics;

use super::channel::{CapturedFrame, QueueDepth, StackedFrame};
use super::field_solve;
use super::context::{LiveStacker, StackingCarryover};
use super::stack_crop::{self, StackCrop};
use super::wanderer::Wanderer;
use super::{pipeline, solving, storage};

/// The channel ends the stacking task owns, with the depth counters that shadow them.
///
/// Grouped rather than passed one by one: a channel and its counter are only correct
/// together — incremented before the send, given back when the send did not happen — so
/// keeping them apart invites exactly the desync [`QueueDepth`] documents. `CaptureTask`
/// takes its pair the same way.
pub struct StackingChannels {
    /// Incoming captured frames.
    pub stacking_rx: mpsc::Receiver<CapturedFrame>,
    /// Depth of the channel behind `stacking_rx`, decremented per message taken.
    pub stacking_depth: QueueDepth,
    /// Outgoing display frames.
    pub render_tx: mpsc::SyncSender<StackedFrame>,
    /// Depth of the channel in front of `render_tx`. Read as well as written: it is
    /// what `want_display` gates on.
    pub render_depth: QueueDepth,
    /// Slots in that channel, so its depth can be reported as a fraction.
    pub render_capacity: usize,
}

/// Stacking pipeline running on a dedicated OS thread. Receives captured frames, runs
/// star detection, registration, and accumulation, and sends the resulting display
/// frame to the render channel. Owns all stacking contexts exclusively — no shared
/// mutable state.
///
/// `carryover` seeds those contexts from a capture that ended unexpectedly, so a
/// session resumed after a reconnect keeps the integration it had already built
/// rather than starting from one frame.
pub fn run_stacking_task(
    state: Arc<AppState>,
    channels: StackingChannels,
    rt: tokio::runtime::Handle,
    carryover: Option<StackingCarryover>,
) {
    let StackingChannels {
        stacking_rx,
        stacking_depth,
        render_tx,
        render_depth,
        render_capacity,
    } = channels;

    debug!(resumed = carryover.is_some(), "Stacking task started");

    let mut stacker: Option<Box<dyn LiveStacker>> = carryover.map(|carried| carried.stacker);
    let (mut was_stacking_enabled, mut last_stacking_type) =
        reset_detector_start(stacker.as_deref());
    let mut stacking_failed = false;
    let mut wanderer = Wanderer::default();
    let mut stack_crop = StackCrop::new();

    // The raw-CFA stage, rebuilt only when what it is derived from moves: a stage
    // may own precomputed state, so it must not be reconstructed per frame. The
    // key carries `stacking_type` as well as the correction settings because the
    // FPN stage is gated on it — switching to Planetary has to drop that stage
    // even though no sensor setting changed.
    let mut cfa_stage_key: Option<(SensorCorrectionSettings, StackingType)> = None;
    let mut cfa_pipeline = CfaPipeline::new();
    let mut debayer = DebayerAlgorithm::Bilinear;

    while let Ok(msg) = stacking_rx.recv() {
        // One per message taken, before any work on it: the reported depth is what the
        // capture thread is waiting behind, not what it was when this frame arrived.
        stacking_depth.taken();

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
                    state.frame_rejected(settings.stacking, format!("Conversion failed: {}", e));
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

        if must_reset_stack(stacking_enabled, was_stacking_enabled, stacking_type_changed) {
            stacker = None;
            stacking_failed = false;
            wanderer.forget();
            state.reset_counters();
            info!(
                stacking_type = ?settings.stacking_type,
                "Live stacking enabled/changed, resetting context and counters"
            );
        }
        was_stacking_enabled = stacking_enabled;
        last_stacking_type = settings.stacking_type;

        // Check frame dimension mismatch (e.g. after binning change)
        if check_dimension_mismatch(&frame, stacker.as_deref()) {
            info!("Frame dimensions changed (likely due to binning change), resetting stack");
            stacker = None;
            wanderer.forget();
            state.reset_counters();
        }

        // Process frame through stacking pipeline
        let registration_succeeded;
        let mut showing_stack;
        let stack_reset;
        let mut rejected_because;
        let mut stack_depth;
        let noise;
        let mut display_frame = if stacking_enabled && !stacking_failed {
            debug!(
                stacking = settings.stacking,
                stacking_type = ?settings.stacking_type,
                "Processing frame through stacking pipeline"
            );

            // Build the display copy only when the render task has nothing queued.
            // Checked here rather than inside the pipeline functions so all three modes
            // answer it the same way, and read once per iteration so the decision cannot
            // change under the pipeline mid-frame.
            let want_display = render_depth.pending() == 0;

            // A stack about to be built from nothing is a new field as far as the
            // background's catalog protection knows.
            if stacker.is_none() {
                state.field_solve.reset();
            }
            let outcome = pipeline::stack_frame(
                &frame,
                &settings,
                &state.plugins,
                &mut stacker,
                &mut stacking_failed,
                want_display,
            );
            registration_succeeded = outcome.frame_added;
            showing_stack = outcome.showing_stack;
            stack_reset = outcome.stack_reset;
            rejected_because = outcome.rejected_because();
            stack_depth = outcome.stack_depth;
            noise = outcome.noise;
            outcome.display_frame.map(Arc::new)
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
            stack_depth = 0;
            noise = None;
            Some(Arc::clone(&frame))
        };

        // The stack restarted on a sharper reference, so the integration the
        // counters describe no longer exists.
        if stack_reset {
            state.reset_counters();
            state.field_solve.reset();
        }

        // Note there is deliberately no raw-frame fallback for a frame that
        // merely failed to register — see `StackingOutcome`.

        // Wanderer mode: reset stack if movement is confirmed
        if wanderer.confirms_movement(
            settings.wanderer_mode,
            stacking_enabled,
            registration_succeeded,
            rejected_because,
        ) {
            info!(
                reason = rejected_because.map(|r| r.describe()).unwrap_or("registration failed"),
                "Wanderer mode: movement confirmed, resetting stack"
            );
            stacker = None;
            state.reset_counters();
            // Always displayed, even when the compute above was skipped: the stack this
            // frame was measured against no longer exists, so the view must stop showing
            // it. The raw sub is a handle clone, not a copy.
            display_frame = Some(Arc::clone(&frame));
            showing_stack = false;
            stack_depth = 0;
            // The stack this frame failed against no longer exists, so the
            // verdict against it describes nothing the user can act on. Wanderer
            // treats a failed registration as the *signal*, not as a fault.
            rejected_because = None;
        }

        // Whether *this* frame joined the stack. Deriving it from the context's
        // frame count instead would report every frame after the first as
        // stacked, leaving the UI's rejection counter pinned at zero.
        let was_stacked = stacking_enabled && registration_succeeded;

        // Background protection's one plate solve of this stack (Pro). Deep-sky stacks
        // only: a planetary or comet stack is not registered on the stars.
        if let (true, Some(stack)) = (
            showing_stack
                && settings.background_subtraction
                && settings.stacking_type == StackingType::DeepSky,
            display_frame.as_ref(),
        ) {
            field_solve::offer(&state, &rt, stack, stack_depth as usize);
        }

        // Trigger plate solving asynchronously. Gated up front: without the Push-To
        // plugin the solve is a no-op, and spawning it would keep a second handle on
        // the frame alive long enough to make the render task's `Arc::try_unwrap` fail
        // and copy instead. Skipped along with the display copy on an iteration that
        // made none — the solve wants the stack, not a single sub, and `solve_frame` is
        // rate-limited by `MIN_SOLVE_ATTEMPT_INTERVAL` anyway. Declines outright while a
        // guide camera is connected: that camera is the solve source then, and offers
        // frames far more often than an imaging sub arrives.
        if let (true, Some(frame_to_solve)) = (
            solving::plate_solve_available(&state, solving::SolveSource::Main),
            display_frame.as_ref(),
        ) {
            solving::offer_plate_solve(
                &state,
                &rt,
                Arc::clone(frame_to_solve),
                solving::SolveSource::Main,
            );
        }

        // Update frame counters. The reason rides on `frame_captured`, never on
        // `frame_rejected` — that one feeds the capture-abort burst detector and
        // is for a camera that failed to deliver a frame at all.
        //
        // Spanned for the same reason as the render task's `publish_state`. Lock-free
        // since the counters became atomics; it once was an async lock reached by
        // `rt.block_on`, part of 8.7 ms of `stacking_iteration` self time with no name.
        {
            let _span = tracing::info_span!("publish_state").entered();
            state.frame_captured(
                was_stacked,
                settings.stacking,
                rejected_because.map(|reason| reason.describe()),
            );
        }

        // Release our handle on the captured frame before handing the display
        // frame downstream. On the raw-fallback paths the two are the same
        // allocation, and holding this binding until the end of the iteration
        // would leave the render task looking at a shared `Arc` — making it
        // copy the very frame this indirection exists to avoid.
        drop(frame);

        // Send to render channel (non-blocking — skip if render is busy). No frame at
        // all means this iteration deliberately made no display copy; the render task
        // still has the previous one.
        let Some(display_frame) = display_frame else {
            continue;
        };
        // Wanderer mode's reset above swapped the stack for a raw sub, so a map
        // measured on the stack no longer describes what is on screen.
        let noise = showing_stack.then_some(noise).flatten();
        let crop = display_crop(&mut stack_crop, &settings, showing_stack, stack_depth, noise.as_ref());
        let render_msg = StackedFrame {
            display_frame,
            showing_stack,
            was_stacked,
            frame_number,
            settings,
            stack_depth,
            noise,
            crop,
        };
        // Publish the count *before* the message, undone on arms that didn't send.
        // `try_send` makes the frame visible instantly, so counting after leaves a
        // window where the receiver wakes, runs `taken()` against zero, saturates,
        // and the sender's `sent()` lands on an already-empty channel — permanently
        // one above truth: `want_display` is `pending() == 0`, so the stacking task
        // would stop building display frames and the live view would freeze for the
        // rest of the session. Over-counting here costs at most one skipped display
        // copy; under-counting costs the session.
        render_depth.sent();
        match render_tx.try_send(render_msg) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                render_depth.taken();
                debug!("Render channel disconnected, stopping stacking task");
                break;
            }
            // Full: the frame is dropped, so give back the slot we just claimed. The
            // depth still reflects the messages the render task really has.
            Err(mpsc::TrySendError::Full(_)) => render_depth.taken(),
        }
        telemetry_metrics::record_pipeline_queue_depth(
            "stacking_to_render",
            render_depth.pending() as u64,
            render_capacity as u64,
        );
    }

    // Save stacked result before exiting
    save_stacked_result(&state, stacker.as_deref(), &mut stack_crop);

    // Park the accumulator in case this capture is about to be resumed after a
    // reconnect. A fresh start or a clean stop clears it; see
    // `CaptureService::start_capture` and `stop_capture`.
    state.resume.park_stack(stacker.map(|stacker| StackingCarryover { stacker }));

    debug!("Stacking task ended");
}

/// Where the stack-reset detector starts: whether stacking counts as already on, and
/// with which type.
///
/// A resumed session carries a stack, which counts as running in its own mode. Starting
/// from "off" made its first frame read as stacking being switched on, which discarded
/// the carried stack and zeroed its counters — every reconnect resumed from one frame.
/// Its own mode rather than the settings' one: resumed under another mode, the carried
/// stack cannot take the new frames, and the first of them has to restart it.
fn reset_detector_start(carried: Option<&dyn LiveStacker>) -> (bool, StackingType) {
    match carried {
        Some(stacker) => (true, stacker.kind()),
        None => (false, StackingType::DeepSky),
    }
}

/// Whether this frame starts a new stack: stacking was just switched on, or its type
/// changed under a running stack.
fn must_reset_stack(stacking_enabled: bool, was_stacking_enabled: bool, type_changed: bool) -> bool {
    stacking_enabled && (!was_stacking_enabled || type_changed)
}

/// Whether a running stack was built for a different frame geometry than `frame`.
fn check_dimension_mismatch(frame: &Frame, stacker: Option<&dyn LiveStacker>) -> bool {
    stacker.is_some_and(|stacker| {
        stacker.geometry() != (frame.width(), frame.height(), frame.channels())
    })
}

/// The crop a display frame is sent with: `auto_crop`'s, for a frame that shows the stack.
/// A raw sub standing in for it (live view, Wanderer's reset) is never cut.
fn display_crop(
    stack_crop: &mut StackCrop,
    settings: &crate::state::CaptureSettings,
    showing_stack: bool,
    stack_depth: u32,
    coverage: Option<&night_amplifier_core::frame::NoiseField>,
) -> Option<night_amplifier_core::stacking::CropRect> {
    stack_crop.decide(settings.auto_crop && showing_stack, stack_depth as usize, coverage)
}

/// The stack a session ends with, and what its export needs beside the pixels.
struct FinalStack {
    frame: Frame,
    depth: usize,
    /// The deep-sky stack's coverage map, when its subs did not all cover it.
    coverage: Option<night_amplifier_core::frame::NoiseField>,
    /// Where `frame` was cut from the stack, when `auto_crop` cropped it.
    crop: Option<night_amplifier_core::stacking::CropRect>,
}

/// The session's stack, if it holds one, cropped as the live view last showed it when
/// `auto_crop` is on.
///
/// Depth and coverage travel with the frame, from the stack that holds all three,
/// because the saved PNG is rendered with both as the live view renders them: the tone
/// curve spends the depth (`render::autostretch::depth_grain_gain`) and the filters read
/// the coverage. Re-deriving the depth from the session's `stacked_count` would let the
/// export and the live view disagree about the same stack — by the reference frame, and
/// by anything a mid-session reset did to the counters. The crop comes from the session's
/// own tracker for the same reason, and is cut before the FITS's background subtraction
/// and the PNG's render alike.
fn final_stack(stacker: Option<&dyn LiveStacker>, stack_crop: &mut StackCrop, auto_crop: bool) -> Option<FinalStack> {
    let stacker = stacker?;
    let (mut frame, mut coverage) = stacker.snapshot().ok()?;
    let depth = stacker.depth();
    let crop = stack_crop.decide(auto_crop, depth, coverage.as_ref());
    let crop = crop.filter(|rect| match stack_crop::crop_stack(&mut frame, coverage.as_ref(), rect) {
        Ok(cropped) => {
            coverage = cropped;
            true
        }
        Err(e) => {
            tracing::warn!(error = %e, "Auto crop of the saved stack failed, saving it whole");
            false
        }
    });
    Some(FinalStack { frame, depth, coverage, crop })
}

/// Save the final stacked result at the end of a capture session.
fn save_stacked_result(
    state: &Arc<AppState>,
    stacker: Option<&dyn LiveStacker>,
    stack_crop: &mut StackCrop,
) {
    let auto_crop = state.settings.snapshot().auto_crop;
    if let Some(stack) = final_stack(stacker, stack_crop, auto_crop) {
        // The imaging camera specifically: it is the one whose frames are in this
        // stack, and with a guide camera connected an arbitrary map entry could name
        // the wrong instrument in the FITS header.
        let camera_info = state.camera_in_role(CameraRole::Main);
        if let Some(info) = camera_info {
            storage::save_stacked_result(
                state,
                Some(stack.frame),
                stack.depth as u32,
                stack.coverage,
                stack.crop,
                &info,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{check_dimension_mismatch, must_reset_stack, reset_detector_start, StackCrop};
    use crate::capture::context::{
        CometStacker, LiveStacker, PlanetaryStackingContext, StackSettings, StackingContext,
    };
    use crate::state::{CaptureSettings, StackingType};

    fn deep_sky(width: usize, height: usize, channels: usize) -> Box<dyn LiveStacker> {
        let settings = StackSettings::of(&CaptureSettings::default(), &night_amplifier_core::plugins::Plugins::none());
        Box::new(StackingContext::new(width, height, channels, &settings).expect("context"))
    }

    /// A reconnect hands the stacking task the stack it had built. Its first frame must
    /// carry on with it, not restart the integration.
    #[test]
    fn a_resumed_stack_survives_its_first_frame() {
        let carried = deep_sky(16, 16, 1);
        let (was_enabled, last_type) = reset_detector_start(Some(carried.as_ref()));
        assert_eq!((was_enabled, last_type), (true, StackingType::DeepSky));
        assert!(!must_reset_stack(true, was_enabled, StackingType::DeepSky != last_type));
    }

    /// A deep-sky stack cannot take planetary frames: resumed under another mode, the
    /// carried stack has to make way for that mode's own.
    #[test]
    fn a_stack_resumed_under_another_mode_restarts() {
        let carried = deep_sky(16, 16, 1);
        let (was_enabled, last_type) = reset_detector_start(Some(carried.as_ref()));
        assert!(must_reset_stack(true, was_enabled, StackingType::Planetary != last_type));
    }

    #[test]
    fn a_fresh_session_starts_its_stack_on_the_first_frame() {
        let (was_enabled, last_type) = reset_detector_start(None);
        assert!(must_reset_stack(true, was_enabled, StackingType::Planetary != last_type));
        assert!(!must_reset_stack(false, was_enabled, false), "stacking off resets nothing");
    }

    /// Each mode reports its own kind, which is what the reset detector compares against.
    #[test]
    fn every_mode_reports_its_own_kind() {
        let settings = StackSettings::of(&CaptureSettings::default(), &night_amplifier_core::plugins::Plugins::none());
        let planetary = PlanetaryStackingContext::new(16, 16, 1, &settings).expect("context");
        let comet = CometStacker::from_context(Box::new(
            crate::capture::context::StubComet::new(16, 16, 1),
        ));
        assert_eq!(deep_sky(16, 16, 1).kind(), StackingType::DeepSky);
        assert_eq!(planetary.kind(), StackingType::Planetary);
        assert_eq!(comet.kind(), StackingType::Comet);
    }

    #[test]
    fn changing_the_stacking_type_restarts_a_running_stack() {
        assert!(must_reset_stack(true, true, true));
        assert!(!must_reset_stack(true, true, false));
    }

    /// The export renders the deep-sky stack with the coverage map the live view carries,
    /// so the session's final stack has to hand it over: here a drifting session whose
    /// left strip only the reference reached.
    #[test]
    fn the_final_deep_sky_stack_carries_its_coverage_map() {
        let ctx = drifted_stack(32, 3);
        let stack = super::final_stack(Some(&ctx), &mut StackCrop::new(), false).expect("a stack");
        assert_eq!(stack.depth, 4);
        let coverage = stack.coverage.expect("a thin strip is something to say");
        let strip = coverage.sample_coverage(2, 16);
        assert!((strip - 0.25).abs() < 1e-6, "the strip read {strip}");

        // Every sub covered all of it: nothing to carry, as the live view carries nothing.
        let settings = StackSettings::of(&CaptureSettings::default(), &night_amplifier_core::plugins::Plugins::none());
        let mut even = StackingContext::new(32, 32, 1, &settings).expect("context");
        even.stacker.add_reference(&night_amplifier_core::frame::Frame::filled(32, 32, 1, 0.3).unwrap()).unwrap();
        assert!(super::final_stack(Some(&even), &mut StackCrop::new(), false).unwrap().coverage.is_none());
        assert!(super::final_stack(None, &mut StackCrop::new(), true).is_none(), "no stack, nothing to save");
    }

    /// A `size`² mono stack of a reference and `subs` subs whose leftmost 8 px fell outside
    /// the warp: that strip holds `1 / (subs + 1)` of the stack.
    fn drifted_stack(size: usize, subs: usize) -> StackingContext {
        use night_amplifier_core::frame::Frame;
        use night_amplifier_core::registration::AffineTransform;

        let settings = StackSettings::of(&CaptureSettings::default(), &night_amplifier_core::plugins::Plugins::none());
        let sky = || Frame::filled(size, size, 1, 0.3).unwrap();
        let mut drifted = sky();
        for y in 0..size {
            for x in 0..8 {
                drifted.set_pixel(x, y, 0, 0.0); // the warp border's value
            }
        }
        let mut ctx = StackingContext::new(size, size, 1, &settings).expect("context");
        ctx.stacker.add_reference(&sky()).unwrap();
        for _ in 0..subs {
            ctx.stacker.add_frame(&drifted, &AffineTransform::identity()).unwrap();
        }
        ctx
    }

    /// Subs on top of the reference that make a stack deep enough to crop.
    const DEEP: usize = night_amplifier_core::stacking::MIN_CROP_DEPTH - 1;

    /// Off, nothing a display frame carries is ever cut; on, only a frame showing the stack.
    #[test]
    fn only_a_stack_shown_with_the_setting_on_is_cropped() {
        let ctx = drifted_stack(128, DEEP);
        let (_, coverage) = ctx.snapshot().unwrap();
        let coverage = coverage.as_ref();
        let off = CaptureSettings::default();
        let on = CaptureSettings { auto_crop: true, ..CaptureSettings::default() };
        let mut crop = StackCrop::new();

        for depth in 2..40 {
            assert_eq!(super::display_crop(&mut crop, &off, true, depth, coverage), None, "depth {depth}");
        }
        let shallow = night_amplifier_core::stacking::MIN_CROP_DEPTH as u32 - 1;
        assert_eq!(super::display_crop(&mut crop, &on, true, shallow, coverage), None, "a shallow stack was cut");
        assert!(super::display_crop(&mut crop, &on, true, 40, coverage).is_some());
        assert_eq!(super::display_crop(&mut crop, &on, false, 0, None), None, "a raw sub was cut");
        assert_eq!(super::display_crop(&mut crop, &on, false, 41, coverage), None, "Wanderer's raw sub was cut");
    }

    /// With `auto_crop` off the saved stack is the accumulator's, untouched: every sample,
    /// its size and its coverage map, however thin its border.
    #[test]
    fn the_saved_stack_is_never_cropped_with_the_setting_off() {
        let ctx = drifted_stack(128, DEEP);
        let (whole, map) = ctx.snapshot().unwrap();
        let stack = super::final_stack(Some(&ctx), &mut StackCrop::new(), false).unwrap();
        assert_eq!(stack.crop, None);
        assert_eq!((stack.frame.width(), stack.frame.height()), (128, 128));
        assert!(stack.frame.data() == whole.data(), "the saved samples changed");
        assert_eq!(stack.coverage.unwrap().coverage(), map.unwrap().coverage());
    }

    #[test]
    fn the_saved_stack_is_cropped_with_the_setting_on() {
        let ctx = drifted_stack(128, DEEP);
        let (whole, _) = ctx.snapshot().unwrap();
        let stack = super::final_stack(Some(&ctx), &mut StackCrop::new(), true).unwrap();
        let rect = night_amplifier_core::stacking::CropRect { x: 8, y: 0, width: 120, height: 128 };
        assert_eq!(stack.crop, Some(rect));
        assert_eq!((stack.frame.width(), stack.frame.height()), (120, 128));
        assert_eq!(stack.frame.get_pixel(0, 5, 0), whole.get_pixel(8, 5, 0));
        assert!(stack.coverage.is_none(), "what is left is complete, so it carries no map");
    }

    #[test]
    fn test_check_dimension_mismatch_no_context() {
        let frame = night_amplifier_core::frame::Frame::zeros(100, 100, 3).unwrap();
        assert!(!check_dimension_mismatch(&frame, None));
    }

    /// A binning change or a colour/mono swap: the running stack cannot take the frame.
    #[test]
    fn a_frame_of_another_geometry_does_not_fit_the_running_stack() {
        let stack = deep_sky(100, 100, 3);
        let fits = night_amplifier_core::frame::Frame::zeros(100, 100, 3).unwrap();
        let binned = night_amplifier_core::frame::Frame::zeros(50, 50, 3).unwrap();
        let mono = night_amplifier_core::frame::Frame::zeros(100, 100, 1).unwrap();
        assert!(!check_dimension_mismatch(&fits, Some(stack.as_ref())));
        assert!(check_dimension_mismatch(&binned, Some(stack.as_ref())));
        assert!(check_dimension_mismatch(&mono, Some(stack.as_ref())));
    }
}

#[cfg(test)]
#[path = "stacking_task_export_tests.rs"]
mod export_tests;
