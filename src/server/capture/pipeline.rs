//! Running a captured frame through the stacking and preview stages.
//!
//! The settings-to-configuration half lives in [`super::stage_config`] and is
//! re-exported here, so `pipeline::build_cfa_pipeline` and friends still resolve
//! for the capture and stacking tasks that call them.

use tracing::{instrument, warn};

use super::analysis::{AnalysisContext, PreviewAnalysis};
use super::context::{create_live_stacker, LiveStacker, StackSettings};
use crate::plugins::Plugins;
use super::frame_gate::RejectionReason;
use crate::frame::Frame;
use crate::server::state::CaptureSettings;

pub use super::stage_config::{
    build_cfa_pipeline, convert_captured_frame, debayer_algorithm, get_background_config,
    get_render_pipeline_config,
};

/// What one pass through a stacking pipeline produced. `showing_stack` and
/// `frame_added` must not collapse into one flag: a frame that fails registration
/// leaves the accumulated stack untouched but displayable (`showing_stack: true`)
/// while the counters record a rejection (`frame_added: false`). Substituting the
/// raw sub on failure — the old behaviour — made the preview alternate between a
/// deep stack and a single noisy frame, re-solving auto-stretch against a
/// completely different histogram each swap.
pub struct StackingOutcome {
    /// The frame to display: the accumulated stack, or a single sub when there
    /// is no stack to show yet.
    ///
    /// `None` when the caller passed `want_display: false` — the render task already
    /// has a frame queued, so building another copy of the accumulator would be work
    /// whose only destination is `drain_to_latest`. The stack itself is unaffected;
    /// only this copy of it is skipped.
    pub display_frame: Option<Frame>,
    /// `display_frame` is the accumulated stack rather than a single sub.
    pub showing_stack: bool,
    /// This frame registered and was added to the stack.
    pub frame_added: bool,
    /// The stack was discarded and restarted during this pass, so the session
    /// counters no longer describe what is on screen.
    pub stack_reset: bool,
    /// Why the frame did not join the stack, when it did not. `None` when it
    /// did, or when the mode does not report a reason.
    pub rejected_because: Option<RejectionReason>,
    /// Frames in the accumulated stack after this pass. `0` when there is no stack.
    ///
    /// Carried so the render task can tell how much the statistics it caches have
    /// moved: they fall as `1/sqrt(N)`, so proportional growth in this number is what
    /// decides when they have to be measured again.
    pub stack_depth: u32,
    /// The stack's coverage map, when `display_frame` is an accumulated stack its subs did
    /// not all cover — see [`crate::frame::NoiseField`]. Taken from the same accumulator
    /// read as the display copy, so it costs one plane's block medians rather than
    /// another 434 MB pass.
    ///
    /// `None` wherever `display_frame` is, wherever every sub covered the whole frame
    /// (the common case), and wherever the mode keeps no `IncrementalPixel` accumulator
    /// — planetary, and comet, whose context is a Pro trait this does not reach through.
    pub noise: Option<crate::frame::NoiseField>,
}

impl StackingOutcome {
    /// A single sub standing in for a stack that does not exist yet.
    fn single_frame(frame: &Frame, frame_added: bool) -> Self {
        Self {
            display_frame: Some(frame.clone()),
            showing_stack: false,
            frame_added,
            stack_reset: false,
            rejected_because: None,
            stack_depth: 0,
            noise: None,
        }
    }

    /// The accumulated stack.
    fn stacked(display_frame: Frame, frame_added: bool, stack_depth: u32) -> Self {
        Self {
            display_frame: Some(display_frame),
            showing_stack: true,
            frame_added,
            stack_reset: false,
            rejected_because: None,
            stack_depth,
            noise: None,
        }
    }

    /// The frame joined the stack, but no copy of the accumulator was made.
    ///
    /// `showing_stack` stays true: it describes what the *live view* is showing, and the
    /// view keeps showing the stack it was already showing. The counters read
    /// `frame_added`, not this.
    fn stacked_not_displayed(frame_added: bool, stack_depth: u32) -> Self {
        Self {
            display_frame: None,
            showing_stack: true,
            frame_added,
            stack_reset: false,
            rejected_because: None,
            stack_depth,
            noise: None,
        }
    }
}

/// Offers one frame to the session's stack, creating the stack on the first frame.
///
/// `stacking_failed` latches when the mode cannot run at all (no plugin, no reference
/// stars), so the caller stops asking and shows raw subs until the stack is reset.
#[instrument(skip_all, fields(
    width = frame.width(),
    height = frame.height(),
    channels = frame.channels(),
    kind = ?settings.stacking_type,
))]
pub fn stack_frame(
    frame: &Frame,
    settings: &CaptureSettings,
    plugins: &Plugins,
    stacker: &mut Option<Box<dyn LiveStacker>>,
    stacking_failed: &mut bool,
    want_display: bool,
) -> StackingOutcome {
    let stack = StackSettings::of(settings, plugins);
    let stacker = match stacker {
        Some(stacker) => stacker,
        None => match create_live_stacker(settings.stacking_type, frame, &stack) {
            Ok(created) => stacker.insert(created),
            Err(e) => {
                warn!(error = %e, "Cannot stack, falling back to single-frame mode");
                *stacking_failed = true;
                return StackingOutcome::single_frame(frame, false);
            }
        },
    };
    stacker.apply_settings(&stack);

    if !stacker.has_reference() {
        if let Err(e) = stacker.set_reference(frame) {
            warn!(error = %e, "Failed to initialize stacking, falling back to single-frame mode");
            *stacking_failed = true;
            return StackingOutcome::single_frame(frame, false);
        }
        return StackingOutcome::single_frame(frame, true); // The reference always "joins"
    }

    let admission = match stacker.offer(frame, &stack) {
        Ok(admission) => admission,
        Err(e) => {
            warn!(error = %e, "Error adding frame to stack");
            return StackingOutcome::single_frame(frame, false);
        }
    };
    let depth = stacker.depth() as u32;
    let with_verdict = |outcome: StackingOutcome| StackingOutcome {
        stack_reset: admission.rebased,
        rejected_because: admission.rejected_because,
        ..outcome
    };

    // The frame is in the stack now. What follows is only the copy the live view needs,
    // and the render task already having one queued means this copy's only destination
    // is `drain_to_latest`.
    if !want_display {
        return with_verdict(StackingOutcome::stacked_not_displayed(admission.added, depth));
    }

    // The raw stack; background subtraction happens in the preview.
    match stacker.snapshot() {
        Ok((stacked, noise)) => StackingOutcome {
            noise,
            ..with_verdict(StackingOutcome::stacked(stacked, admission.added, depth))
        },
        Err(e) => {
            warn!(error = %e, "Failed to compute stack, using raw frame");
            StackingOutcome::single_frame(frame, false)
        }
    }
}

/// What one pass of the preview pipeline decided about a frame: the stage config and the
/// solved tone curve. A named type rather than a tuple so the four call sites say which
/// half they read.
pub struct PreviewRender {
    pub pipeline_config: crate::render::RenderPipelineConfig,
    pub stretch_result: Option<crate::render::display::StretchResult>,
}

/// Process a frame for preview display using the unified render pipeline.
/// Now returns a RenderReadyFrame instead of applying the non-linear stretch,
/// allowing the stretch to be fused into the downsampling pass.
///
/// Analyses the frame from scratch. The render task uses
/// [`process_preview_frame_with_analysis`] instead, which reuses the estimates a
/// previous frame of the same stack already produced.
pub fn process_preview_frame(
    frame: &mut Frame,
    settings: &CaptureSettings,
    plugins: &Plugins,
) -> crate::error::Result<PreviewRender> {
    process_preview_frame_with_analysis(
        frame,
        settings,
        plugins,
        AnalysisContext::ONE_SHOT,
        &mut PreviewAnalysis::new(),
    )
}

/// [`process_preview_frame`] reusing the estimates a previous frame of the same stack
/// already produced: white balance, background model, image statistics. These
/// describe the stack rather than this frame, and a stack moves by 1/N per render.
/// `analysis` decides per frame whether the stored set still applies; see
/// [`super::analysis`] for the four things that invalidate it. Everything that
/// touches pixels still runs every frame — only the measuring is reused.
pub fn process_preview_frame_with_analysis(
    frame: &mut Frame,
    settings: &CaptureSettings,
    plugins: &Plugins,
    ctx: AnalysisContext,
    analysis: &mut PreviewAnalysis,
) -> crate::error::Result<PreviewRender> {
    use crate::background::BackgroundExtractor;
    use crate::render::autostretch::prepare_auto_stretch_frame_with_stats;

    // `analysis_reused` is declared here, not just recorded below: `Span::record` on a
    // field the macro never declared is a silent no-op, so the one thing that says
    // whether the cache is working would have been invisible in every trace.
    let _span = tracing::info_span!(
        "process_preview_frame",
        analysis_reused = tracing::field::Empty
    )
    .entered();

    let mut pipeline_config = get_render_pipeline_config(settings, plugins, false);
    // How deep the stack is decides how much of its noise reduction the stretch
    // spends on a calmer sky (`render::autostretch::depth_grain_gain`). Set here
    // rather than in `get_render_pipeline_config`, which only sees settings.
    pipeline_config.stretch_config.stack_depth = ctx.stack_depth.max(1);

    let reused = analysis.begin_frame(
        ctx,
        (frame.width(), frame.height(), frame.channels()),
        pipeline_config.background_subtraction,
        &pipeline_config.background_config,
        pipeline_config.scnr,
        pipeline_config.scnr_amount,
        pipeline_config.auto_stretch,
    );
    tracing::Span::current().record("analysis_reused", reused);

    // Stage 0: Background Neutralization (Pre-subtraction)
    //
    // Split into `wb_grid` (the estimate) and `wb_apply` (the multiply) because they
    // scale with different things and only one of them is expensive: the grid reads the
    // frame to produce three numbers, the apply is one pass over every sample. A single
    // span here reported 97 ms with no way to tell which half owned it.
    if pipeline_config.background_subtraction && frame.channels() == 3 {
        let _span0 = tracing::info_span!("background_neutralization").entered();
        let multipliers = analysis.white_balance(|| {
            let _span = tracing::info_span!("wb_grid").entered();
            crate::render::compute_white_balance_grid_with_config(
                frame,
                16,
                25.0,
                crate::render::WhiteBalanceConfig::preview(),
            )
        });
        match multipliers {
            Ok(multipliers) => {
                let _span = tracing::info_span!("wb_apply").entered();
                if let Err(e) = crate::render::neutralize_background(frame, &multipliers) {
                    warn!(error = %e, "Background neutralization failed");
                }
            }
            Err(e) => warn!(error = %e, "Failed to compute grid white balance"),
        }
    }

    // Stage 1: Background subtraction (modifies linear data)
    //
    // The estimate and the subtraction are separated here, rather than going through
    // `subtract_background_with_config`, because only the estimate is reusable — the
    // model still has to be subtracted from every frame.
    if pipeline_config.background_subtraction {
        let _span1 = tracing::info_span!("background_subtraction").entered();
        let config = pipeline_config.background_config.clone();
        match analysis.background(|| {
            BackgroundExtractor::new(config)
                .with_plugins(plugins.clone())
                .estimate(frame)
        }) {
            Ok(model) => {
                let _span = tracing::info_span!("subtract_model").entered();
                model.subtract_from(frame);
            }
            Err(e) => warn!(error = %e, "Background subtraction failed"),
        }
    }

    // Stage 1.5: SCNR
    if pipeline_config.scnr && frame.channels() == 3 {
        let _span1_5 = tracing::info_span!("scnr").entered();
        if let Err(e) = crate::render::scnr::apply_scnr(frame, pipeline_config.scnr_amount) {
            warn!(error = %e, "SCNR failed");
        }
    }

    // Stage 2: Prepare auto-stretch (computes stats, subtracts black point, but does not stretch)
    //
    // The statistics are measured here, after the three stages above have run, which is
    // what makes them reusable: a cached set describes a frame that went through the
    // *same* cached white balance and background model, so the whole analysis is
    // internally consistent or none of it is.
    let stretch_result = if pipeline_config.auto_stretch {
        let _span2 = tracing::info_span!("prepare_auto_stretch").entered();
        // Not `?`: a frame too small for robust statistics (below `StatsConfig`'s
        // 1000-sample minimum) has always rendered unstretched rather than not at all,
        // and the error handling below is what keeps it that way. Propagating from here
        // would make the render task drop the whole frame.
        let prepared = analysis
            .stats(|| {
                let _span = tracing::info_span!("compute_image_stats").entered();
                crate::statistics::compute_image_stats(frame)
            })
            .and_then(|stats| {
                prepare_auto_stretch_frame_with_stats(
                    frame,
                    pipeline_config.stretch_config,
                    &stats,
                )
            });
        match prepared {
            Ok(res) => {
                // When saturation boost is off and contrast is enabled, fuse the
                // contrast S-curve into the scale LUT — the same optimization that
                // auto_stretch_frame used in the old RenderPipeline::process path.
                // This eliminates a separate per-pixel contrast pass in the encode
                // kernels. When saturation boost is on, contrast must run as a
                // separate pass because saturation sits between stretch and contrast —
                // and so does the AI denoiser, which reads the stretched image before
                // either (`render::denoise::ai`).
                let can_fuse_contrast = pipeline_config.contrast
                    && frame.channels() == 3
                    && !pipeline_config.contrast_config.is_disabled()
                    && !pipeline_config.saturation_boost
                    && !pipeline_config.denoise.ai.is_enabled();

                let contrast_for_lut = if can_fuse_contrast {
                    Some(&pipeline_config.contrast_config)
                } else {
                    None
                };

                // The floor anchors to where the sky ends up *after* contrast,
                // so the anchor asks whether contrast runs at all — not whether
                // it runs inside the table. Those differ exactly when saturation
                // boost is on, and using `contrast_for_lut` here would move the
                // slider's meaning between Community and Pro.
                let contrast_applied = (pipeline_config.contrast
                    && !pipeline_config.contrast_config.is_disabled())
                .then_some(&pipeline_config.contrast_config);
                let shadow = pipeline_config.shadow_floor.resolve(
                    crate::render::sky_level_after_contrast(
                        res.target_background,
                        contrast_applied,
                    ),
                );

                let scale_lut = crate::render::stretch::cached_scale_lut(
                    pipeline_config.stretch_config.tone_mapping,
                    res.stretch_factor,
                    contrast_for_lut,
                    // Only fused here when contrast was. Otherwise the row tail
                    // applies it after the separate contrast pass, so that the
                    // order is stretch -> saturation -> contrast -> floor either
                    // way.
                    if can_fuse_contrast {
                        shadow.floor
                    } else {
                        crate::render::ShadowFloor::NONE
                    },
                );

                if can_fuse_contrast {
                    pipeline_config.contrast = false;
                }

                Some(crate::render::display::StretchResult {
                    black_point: res.black_point,
                    scale_lut,
                    color_intensity: pipeline_config.stretch_config.color_intensity,
                    deferred_shadow_floor: (!can_fuse_contrast && !shadow.floor.is_none())
                        .then(|| {
                            std::sync::Arc::new(crate::render::ShadowFloorTable::new(shadow.floor))
                        }),
                    sky_shadow: shadow.sky,
                })
            }
            Err(e) => {
                warn!(error = %e, "Auto-stretch preparation failed");
                None
            }
        }
    } else {
        None
    };

    Ok(PreviewRender {
        pipeline_config,
        stretch_result,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundExtractionAlgorithm;

    /// A frame with enough well-separated stars for detection and registration.
    fn starfield(width: usize, height: usize, offset: f32) -> Frame {
        starfield_with_spread(width, height, offset, 1.0)
    }

    /// The same field with the stars `spread` times wider — defocus, or a thin
    /// cloud, without moving a single star.
    fn starfield_with_spread(width: usize, height: usize, offset: f32, spread: f32) -> Frame {
        let mut frame = Frame::filled(width, height, 1, 0.02).unwrap();
        let placements = [
            (18.0, 22.0, 0.9),
            (57.0, 31.0, 0.7),
            (91.0, 19.0, 0.8),
            (34.0, 68.0, 0.6),
            (76.0, 84.0, 0.85),
            (110.0, 55.0, 0.75),
            (25.0, 105.0, 0.65),
            (98.0, 112.0, 0.9),
            (63.0, 127.0, 0.7),
            (129.0, 92.0, 0.8),
        ];
        // Only the few pixels around each star carry signal, so stay inside a
        // small box rather than evaluating the profile over the whole frame.
        let reach = (6.0 * spread).ceil() as usize;
        let variance = 2.0 * spread * spread;
        for (sx, sy, peak) in placements {
            let (sx, sy) = (sx + offset, sy + offset);
            let (cx, cy) = (sx.round() as usize, sy.round() as usize);
            for y in cy.saturating_sub(reach)..(cy + reach).min(height) {
                for x in cx.saturating_sub(reach)..(cx + reach).min(width) {
                    let d2 = (x as f32 - sx).powi(2) + (y as f32 - sy).powi(2);
                    let v = peak * (-d2 / (2.0 * variance)).exp();
                    let current = frame.get_pixel(x, y, 0);
                    frame.set_pixel(x, y, 0, (current + v).min(1.0));
                }
            }
        }
        frame
    }

    /// The defect behind the reported flicker: when a frame could not be
    /// registered the caller used to be handed the raw sub, so the preview
    /// alternated between a deep stack and a single noisy frame. The stack is
    /// still there and still displayable — only the counter should change.
    #[test]
    fn a_rejected_frame_still_leaves_the_stack_on_screen() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        let first = stack_frame(&reference, &settings, &Plugins::none(), &mut ctx, &mut failed, true);
        assert!(!failed, "reference frame should have initialised the stack");
        assert!(first.frame_added);

        // A blank frame has nothing to register against.
        let blank = Frame::filled(150, 150, 1, 0.02).unwrap();
        let outcome = stack_frame(&blank, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        assert!(
            !outcome.frame_added,
            "a blank frame must not join the stack"
        );
        assert!(
            outcome.showing_stack,
            "the accumulated stack is still displayable and must stay on screen"
        );
    }

    /// The reason has to survive the pipeline, or the status bar can only say
    /// how many frames were dropped and never why.
    #[test]
    fn a_rejected_frame_reports_why() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        stack_frame(&reference, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        let blank = Frame::filled(150, 150, 1, 0.02).unwrap();
        let outcome = stack_frame(&blank, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        let reason = outcome
            .rejected_because
            .expect("a rejected frame must say why");
        assert!(
            !reason.describe().is_empty(),
            "the reason has to be renderable"
        );
        assert!(
            reason.means_the_sky_moved(),
            "a starless frame could not be placed against the reference at all, \
             which is exactly the signal Wanderer resets on: {reason:?}"
        );
    }

    /// Wanderer mode restarts the stack whenever a frame does not join it, so
    /// every verdict the gate can produce is a verdict Wanderer acts on. A soft
    /// frame — cloud, gust, a moment of bad seeing — has every star exactly where
    /// the reference has it; discarding the integration for one of those is the
    /// opposite of what the mode is for. Pinned against real pipeline output
    /// because it is the pipeline, not the classifier, that decides which verdict
    /// a soft frame gets.
    #[test]
    fn a_soft_frame_is_not_the_telescope_being_moved() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        // Past the gate's warm-up, so it has a session to compare against.
        for i in 0..8 {
            let frame = starfield(150, 150, i as f32 * 0.25);
            let outcome =
                stack_frame(&frame, &settings, &Plugins::none(), &mut ctx, &mut failed, true);
            assert!(outcome.frame_added, "sharp frame {i} should have stacked");
        }

        // Same field, same star positions, stars twice as wide.
        let defocused = starfield_with_spread(150, 150, 0.0, 2.2);
        let outcome =
            stack_frame(&defocused, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        assert_eq!(
            outcome.rejected_because,
            Some(RejectionReason::StarsTooLarge),
            "a field with every star in place but twice as wide is a soft frame"
        );
        assert!(
            !outcome
                .rejected_because
                .expect("just asserted")
                .means_the_sky_moved(),
            "Wanderer would have thrown away the whole stack for one soft frame"
        );
        assert!(
            outcome.showing_stack && !outcome.stack_reset,
            "the accumulated stack must survive a soft frame"
        );
    }

    /// The other half of the Wanderer contract, pinned against real pipeline
    /// output rather than a hand-made verdict: a frame that stacks reports no
    /// reason at all, so nothing can read it as movement.
    #[test]
    fn a_stacked_frame_gives_wanderer_nothing_to_react_to() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        stack_frame(&reference, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        let shifted = starfield(150, 150, 2.0);
        let outcome = stack_frame(&shifted, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        assert!(outcome.frame_added);
        assert_eq!(outcome.rejected_because, None);
        assert!(!outcome.stack_reset);
    }

    #[test]
    fn a_frame_that_registers_joins_the_stack() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        stack_frame(&reference, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        let shifted = starfield(150, 150, 2.0);
        let outcome = stack_frame(&shifted, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        assert!(outcome.showing_stack);
        assert!(
            outcome.frame_added,
            "a cleanly shifted starfield should register and stack"
        );
    }

    /// A frame too small for robust statistics renders unstretched, not not-at-all.
    ///
    /// `StatsConfig` refuses anything under 1000 pixels, and the render task treats an
    /// error out of `process_preview_frame` as "drop this frame". Splitting the
    /// statistics out so the analysis cache could hold them briefly turned that refusal
    /// into a propagating `?`, which stopped the preview dead on a small ROI — caught by
    /// the render-task tests, which build 10x10 fixtures.
    #[test]
    fn a_frame_too_small_for_statistics_still_renders() {
        let mut frame = Frame::filled(10, 10, 3, 0.2).unwrap();
        let settings = CaptureSettings::default();

        let stretch = process_preview_frame(&mut frame, &settings, &Plugins::none()).map(|r| r.stretch_result)
            .expect("a frame too small to measure must still render");
        assert!(
            stretch.is_none(),
            "there is no stretch to report without statistics"
        );
    }

    /// Skipping the display copy must skip *only* the copy.
    ///
    /// The whole point of `want_display` is that it is a rendering optimisation, not a
    /// stacking one: the frame still registers, still joins the accumulator, and still
    /// moves the frame count. Getting this wrong would silently throw away integration
    /// time whenever the render thread fell behind — which is exactly the condition the
    /// flag exists to detect, so it would bite hardest on the slowest machines.
    #[test]
    fn skipping_the_display_copy_still_stacks_the_frame() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        stack_frame(&reference, &settings, &Plugins::none(), &mut ctx, &mut failed, true);

        let shifted = starfield(150, 150, 2.0);
        let outcome =
            stack_frame(&shifted, &settings, &Plugins::none(), &mut ctx, &mut failed, false);

        assert!(
            outcome.display_frame.is_none(),
            "want_display: false must not build the copy"
        );
        assert!(
            outcome.frame_added,
            "the frame must still join the stack — the flag is about the preview only"
        );
        assert!(
            outcome.showing_stack,
            "the live view is still showing the stack it was already showing"
        );

        // The accumulator moved: a later frame that does ask for the copy gets one, and
        // it carries the integration this iteration contributed.
        let third = starfield(150, 150, 4.0);
        let outcome =
            stack_frame(&third, &settings, &Plugins::none(), &mut ctx, &mut failed, true);
        assert!(outcome.display_frame.is_some());
        assert_eq!(
            ctx.as_ref().expect("context").depth(),
            3,
            "all three frames must be in the stack, including the undisplayed one"
        );
    }

    /// Comet reports no reasons, so a frame the nucleus could not be found in has to
    /// come through the shared driver exactly as a deep-sky rejection does: counted as
    /// not stacked, with the accumulated comet still on screen.
    #[test]
    fn a_comet_frame_that_fails_to_align_keeps_the_comet_on_screen() {
        use crate::server::capture::context::{CometStacker, StubComet};

        let mut settings = CaptureSettings::default();
        settings.stacking_type = crate::stacking::StackingType::Comet;
        let stub = StubComet::new(32, 32, 1).answering(vec![Ok(true), Ok(false)]);
        let mut stacker: Option<Box<dyn LiveStacker>> =
            Some(Box::new(CometStacker::from_context(Box::new(stub))));
        let mut failed = false;
        let sub = Frame::filled(32, 32, 1, 0.5).unwrap();

        let first = stack_frame(&sub, &settings, &Plugins::none(), &mut stacker, &mut failed, true);
        assert!(first.frame_added && !first.showing_stack, "the reference shows as itself");

        let aligned = stack_frame(&sub, &settings, &Plugins::none(), &mut stacker, &mut failed, true);
        assert!(aligned.frame_added && aligned.showing_stack);
        assert_eq!(aligned.stack_depth, 2);

        let lost = stack_frame(&sub, &settings, &Plugins::none(), &mut stacker, &mut failed, true);
        assert!(!lost.frame_added);
        assert!(lost.showing_stack, "the comet stack stays on screen");
        assert_eq!(lost.rejected_because, None);
        let shown = lost.display_frame.expect("a display copy was asked for");
        assert!((shown.get_pixel(0, 0, 0) - 0.02).abs() < 1e-6, "the stack, not the sub");
        assert!(!failed);
    }

    /// A mode that cannot run here — comet without the Pro plugin — latches the
    /// session onto single frames instead of retrying the plugin every frame.
    #[test]
    fn a_mode_that_cannot_run_falls_back_to_single_frames() {
        let mut settings = CaptureSettings::default();
        settings.stacking_type = crate::stacking::StackingType::Comet;
        let mut stacker = None;
        let mut failed = false;
        let sub = Frame::filled(32, 32, 1, 0.5).unwrap();

        let outcome = stack_frame(&sub, &settings, &Plugins::none(), &mut stacker, &mut failed, true);

        assert!(failed, "the session must stop asking for a stack it cannot build");
        assert!(stacker.is_none());
        assert!(!outcome.frame_added && !outcome.showing_stack);
        assert!(outcome.display_frame.is_some(), "the raw sub is still shown");
    }

    #[test]
    fn test_process_preview_frame_background_subtraction_flag() {
        let mut settings = CaptureSettings::default();
        settings.background_subtraction = true;
        settings.background_extraction_algorithm = BackgroundExtractionAlgorithm::GridBilinear;

        let mut data = vec![0.0f32; 64 * 64 * 1];
        for y in 0..64 {
            for x in 0..64 {
                data[y * 64 + x] = 0.1 + (x as f32 / 63.0) * 0.4;
            }
        }
        let frame = Frame::from_f32_vec(data, 64, 64, 1).unwrap();

        // Process with background subtraction enabled
        let mut frame_bg = frame.clone();
        process_preview_frame(&mut frame_bg, &settings, &Plugins::none()).unwrap();

        // Check if the RenderPipeline used background subtraction
        // Since we reordered the calls, get_render_pipeline_config will now correctly
        // return a config with background_subtraction = true if settings say so.
        let config = get_render_pipeline_config(&settings, &Plugins::none(), false);
        assert!(config.background_subtraction);
    }
}
