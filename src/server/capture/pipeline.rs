//! Running a captured frame through the stacking and preview stages.
//!
//! The settings-to-configuration half lives in [`super::stage_config`] and is
//! re-exported here, so `pipeline::build_cfa_pipeline` and friends still resolve
//! for the capture and stacking tasks that call them.

use tracing::{info, instrument, warn};

use super::context::{PlanetaryStackingContext, StackingContext};
use super::frame_gate::RejectionReason;
use crate::background::subtract_background_with_config;
use crate::frame::Frame;
use crate::server::state::CaptureSettings;
use crate::stacking::{CometContext, COMET_PLUGIN};

pub use super::stage_config::{
    build_cfa_pipeline, convert_captured_frame, debayer_algorithm, get_background_config,
    get_render_pipeline_config,
};

/// What one pass through a stacking pipeline produced.
///
/// `showing_stack` and `frame_added` answer different questions and must not be
/// collapsed into one flag: a frame that fails registration leaves the
/// accumulated stack untouched but perfectly displayable, so the live view keeps
/// showing it (`showing_stack: true`) while the counters record a rejection
/// (`frame_added: false`).
///
/// Substituting the raw sub on a registration failure — as the stacking task
/// used to — is what made the preview alternate between a deep stack and a
/// single noisy frame: the auto-stretch re-solves against a completely
/// different histogram each time the display swaps.
pub struct StackingOutcome {
    /// The frame to display: the accumulated stack, or a single sub when there
    /// is no stack to show yet.
    pub display_frame: Frame,
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
}

impl StackingOutcome {
    /// A single sub standing in for a stack that does not exist yet.
    fn single_frame(frame: &Frame, frame_added: bool) -> Self {
        Self {
            display_frame: frame.clone(),
            showing_stack: false,
            frame_added,
            stack_reset: false,
            rejected_because: None,
        }
    }

    /// The accumulated stack.
    fn stacked(display_frame: Frame, frame_added: bool) -> Self {
        Self {
            display_frame,
            showing_stack: true,
            frame_added,
            stack_reset: false,
            rejected_because: None,
        }
    }
}

/// Process a frame through the stacking pipeline
#[instrument(skip_all, fields(
    width = frame.width(),
    height = frame.height(),
    channels = frame.channels(),
))]
pub async fn process_frame_with_stacking(
    frame: &Frame,
    settings: &CaptureSettings,
    stacking_ctx: &mut Option<StackingContext>,
    stacking_failed: &mut bool,
) -> StackingOutcome {
    // Initialize stacking context on first frame
    if stacking_ctx.is_none() {
        let ctx = StackingContext::new(frame.width(), frame.height(), frame.channels(), settings);
        if ctx.is_none() {
            warn!("Failed to create stacking context, falling back to single-frame mode");
            *stacking_failed = true;
            return StackingOutcome::single_frame(frame, false);
        }
        *stacking_ctx = ctx;
    }

    let ctx = stacking_ctx.as_mut().unwrap();
    ctx.update_from_settings(settings);

    // Initialize with reference frame if not yet done
    if !ctx.is_initialized {
        match ctx.initialize_with_reference(frame) {
            Ok(star_count) => {
                info!(
                    star_count = star_count,
                    "Stacking initialized with reference frame"
                );
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize stacking, falling back to single-frame mode");
                *stacking_failed = true;
                return StackingOutcome::single_frame(frame, false);
            }
        }
        return StackingOutcome::single_frame(frame, true); // First frame is always "successful"
    }

    // Add frame to stack
    let admission = match ctx.add_frame(frame) {
        Ok(admission) => {
            match admission.rejected_because {
                None => info!(
                    frame_count = ctx.frame_count(),
                    matched_stars = admission.matched_stars,
                    // Debug, not the bare f32: NaN/inf are legitimate sentinels here
                    // (see FrameAdmission::mean_residual) but OTel exports them as a
                    // double attribute, and Jaeger's query API 500s trying to JSON-encode
                    // a non-finite float — taking down every trace search that touches
                    // one, not just this span. Recording via Debug makes it a string
                    // attribute instead, which is immune.
                    residual = ?admission.mean_residual,
                    "Frame added to stack"
                ),
                Some(reason) => info!(
                    frame_count = ctx.frame_count(),
                    matched_stars = admission.matched_stars,
                    residual = ?admission.mean_residual,
                    reason = reason.describe(),
                    "Frame not added to stack"
                ),
            }
            admission
        }
        Err(e) => {
            warn!(error = %e, "Error adding frame to stack");
            return StackingOutcome::single_frame(frame, false);
        }
    };

    // Return the current stacked result for display (raw, background subtraction applied in preview)
    match ctx.compute() {
        Ok(stacked) => StackingOutcome {
            stack_reset: admission.rebased,
            rejected_because: admission.rejected_because,
            ..StackingOutcome::stacked(stacked, admission.added)
        },
        Err(e) => {
            warn!(error = %e, "Failed to compute stack, using raw frame");
            StackingOutcome::single_frame(frame, false)
        }
    }
}

/// Process a frame through the comet stacking pipeline
#[instrument(skip_all, fields(
    width = frame.width(),
    height = frame.height(),
    channels = frame.channels(),
))]
pub async fn process_frame_with_comet_stacking(
    frame: &Frame,
    settings: &CaptureSettings,
    comet_ctx: &mut Option<Box<dyn CometContext>>,
    stacking_failed: &mut bool,
) -> StackingOutcome {
    // Initialize comet stacking context on first frame using plugin
    if comet_ctx.is_none() {
        let plugin = crate::license::pro_plugin(&COMET_PLUGIN);
        if let Some(plugin) = plugin {
            let ctx =
                plugin.create_context(frame.width(), frame.height(), frame.channels(), settings);
            *comet_ctx = Some(ctx);
        } else {
            warn!(
                "Comet stacking plugin not found (Pro feature), falling back to single-frame mode"
            );
            *stacking_failed = true;
            return StackingOutcome::single_frame(frame, false);
        }
    }

    let ctx = comet_ctx.as_mut().unwrap();
    ctx.update_from_settings(settings);

    // Check if ROI was updated in settings and update detector
    if let Some(new_roi) = settings.comet_roi {
        let current_roi = ctx.get_roi();
        if new_roi.x != current_roi.x
            || new_roi.y != current_roi.y
            || new_roi.width != current_roi.width
            || new_roi.height != current_roi.height
        {
            info!(
                x = new_roi.x,
                y = new_roi.y,
                width = new_roi.width,
                height = new_roi.height,
                "Comet ROI updated"
            );
            ctx.update_roi(new_roi);
        }
    }

    // Initialize with reference frame if not yet done
    if ctx.frame_count() == 0 {
        match ctx.initialize_with_reference(frame) {
            Ok(()) => {
                info!("Comet stacking initialized with reference frame");
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize comet stacking, falling back to single-frame mode");
                *stacking_failed = true;
                return StackingOutcome::single_frame(frame, false);
            }
        }
        return StackingOutcome::single_frame(frame, true); // First frame is success
    }

    // Add frame to stack
    let frame_added = match ctx.add_frame(frame) {
        Ok(true) => {
            info!(
                frame_count = ctx.frame_count(),
                "Frame added to comet stack"
            );
            true
        }
        Ok(false) => {
            info!(
                frame_count = ctx.frame_count(),
                "Comet alignment failed, frame not added to stack"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "Error adding frame to comet stack");
            false
        }
    };

    // Return the current stacked result for display (raw, background subtraction applied in preview)
    match ctx.compute() {
        Ok(stacked) => StackingOutcome::stacked(stacked, frame_added),
        Err(e) => {
            warn!(error = %e, "Failed to compute comet stack, using raw frame");
            StackingOutcome::single_frame(frame, false)
        }
    }
}

/// Process a frame through the planetary stacking pipeline
#[instrument(skip_all, fields(
    width = frame.width(),
    height = frame.height(),
    channels = frame.channels(),
))]
pub async fn process_frame_with_planetary_stacking(
    frame: &Frame,
    settings: &CaptureSettings,
    planetary_ctx: &mut Option<PlanetaryStackingContext>,
    stacking_failed: &mut bool,
) -> StackingOutcome {
    // Initialize planetary stacking context on first frame
    if planetary_ctx.is_none() {
        let ctx = PlanetaryStackingContext::new(
            frame.width(),
            frame.height(),
            frame.channels(),
            settings,
        );
        if ctx.is_none() {
            warn!("Failed to create planetary stacking context, falling back to single-frame mode");
            *stacking_failed = true;
            return StackingOutcome::single_frame(frame, false);
        }
        *planetary_ctx = ctx;
    }

    let ctx = planetary_ctx.as_mut().unwrap();
    ctx.update_from_settings(settings);

    // Initialize with reference frame if not yet done
    if !ctx.is_initialized {
        match ctx.initialize_with_reference(frame) {
            Ok(()) => {
                info!("Planetary stacking initialized with reference frame");
            }
            Err(e) => {
                warn!(error = %e, "Failed to initialize planetary stacking, falling back to single-frame mode");
                *stacking_failed = true;
                return StackingOutcome::single_frame(frame, false);
            }
        }
        return StackingOutcome::single_frame(frame, true); // First frame is success
    }

    // Add frame to stack
    let frame_added = match ctx.add_frame(frame, settings) {
        Ok(true) => {
            info!(
                frame_count = ctx.frame_count(),
                "Frame added to planetary stack"
            );
            true
        }
        Ok(false) => {
            info!(
                frame_count = ctx.frame_count(),
                "Planetary alignment failed, frame not added to stack"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "Error adding frame to planetary stack");
            false
        }
    };

    // Return the current stacked result for display (raw, background subtraction applied in preview)
    match ctx.compute() {
        Ok(stacked) => StackingOutcome::stacked(stacked, frame_added),
        Err(e) => {
            warn!(error = %e, "Failed to compute planetary stack, using raw frame");
            StackingOutcome::single_frame(frame, false)
        }
    }
}

/// Process a frame for preview display using the unified render pipeline.
/// Now returns a RenderReadyFrame instead of applying the non-linear stretch,
/// allowing the stretch to be fused into the downsampling pass.
pub fn process_preview_frame(
    frame: &mut Frame,
    settings: &CaptureSettings,
) -> crate::error::Result<(
    crate::render::RenderPipelineConfig,
    Option<crate::server::state::StretchResult>,
)> {
    use crate::background::subtract_background_with_config;
    use crate::render::autostretch::prepare_auto_stretch_frame;

    let _span = tracing::info_span!("process_preview_frame").entered();

    let mut pipeline_config = get_render_pipeline_config(settings, false);

    // Stage 0: Background Neutralization (Pre-subtraction)
    //
    // Split into `wb_grid` (the estimate) and `wb_apply` (the multiply) because they
    // scale with different things and only one of them is expensive: the grid reads the
    // frame to produce three numbers, the apply is one pass over every sample. A single
    // span here reported 97 ms with no way to tell which half owned it.
    if pipeline_config.background_subtraction && frame.channels() == 3 {
        let _span0 = tracing::info_span!("background_neutralization").entered();
        let multipliers = {
            let _span = tracing::info_span!("wb_grid").entered();
            crate::render::compute_white_balance_grid_with_config(
                frame,
                16,
                25.0,
                crate::render::WhiteBalanceConfig::preview(),
            )
        };
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
    if pipeline_config.background_subtraction {
        let _span1 = tracing::info_span!("background_subtraction").entered();
        if let Err(e) =
            subtract_background_with_config(frame, pipeline_config.background_config.clone())
        {
            warn!(error = %e, "Background subtraction failed");
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
    let stretch_result = if pipeline_config.auto_stretch {
        let _span2 = tracing::info_span!("prepare_auto_stretch").entered();
        match prepare_auto_stretch_frame(frame, pipeline_config.stretch_config) {
            Ok(res) => {
                // When saturation boost is off and contrast is enabled, fuse the
                // contrast S-curve into the scale LUT — the same optimization that
                // auto_stretch_frame used in the old RenderPipeline::process path.
                // This eliminates a separate per-pixel contrast pass in the encode
                // kernels. When saturation boost is on, contrast must run as a
                // separate pass because saturation sits between stretch and contrast.
                let can_fuse_contrast = pipeline_config.contrast
                    && frame.channels() == 3
                    && !pipeline_config.contrast_config.is_disabled()
                    && !pipeline_config.saturation_boost;

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
                let shadow_floor = pipeline_config.shadow_floor.resolve(
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
                        shadow_floor
                    } else {
                        crate::render::ShadowFloor::NONE
                    },
                );

                if can_fuse_contrast {
                    pipeline_config.contrast = false;
                }

                Some(crate::server::state::StretchResult {
                    black_point: res.black_point,
                    scale_lut,
                    color_intensity: pipeline_config.stretch_config.color_intensity,
                    deferred_shadow_floor: (!can_fuse_contrast && !shadow_floor.is_none())
                        .then(|| {
                            std::sync::Arc::new(crate::render::ShadowFloorTable::new(shadow_floor))
                        }),
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

    Ok((pipeline_config, stretch_result))
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
    #[tokio::test]
    async fn a_rejected_frame_still_leaves_the_stack_on_screen() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        let first = process_frame_with_stacking(&reference, &settings, &mut ctx, &mut failed).await;
        assert!(!failed, "reference frame should have initialised the stack");
        assert!(first.frame_added);

        // A blank frame has nothing to register against.
        let blank = Frame::filled(150, 150, 1, 0.02).unwrap();
        let outcome = process_frame_with_stacking(&blank, &settings, &mut ctx, &mut failed).await;

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
    #[tokio::test]
    async fn a_rejected_frame_reports_why() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        process_frame_with_stacking(&reference, &settings, &mut ctx, &mut failed).await;

        let blank = Frame::filled(150, 150, 1, 0.02).unwrap();
        let outcome = process_frame_with_stacking(&blank, &settings, &mut ctx, &mut failed).await;

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
    #[tokio::test]
    async fn a_soft_frame_is_not_the_telescope_being_moved() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        // Past the gate's warm-up, so it has a session to compare against.
        for i in 0..8 {
            let frame = starfield(150, 150, i as f32 * 0.25);
            let outcome =
                process_frame_with_stacking(&frame, &settings, &mut ctx, &mut failed).await;
            assert!(outcome.frame_added, "sharp frame {i} should have stacked");
        }

        // Same field, same star positions, stars twice as wide.
        let defocused = starfield_with_spread(150, 150, 0.0, 2.2);
        let outcome =
            process_frame_with_stacking(&defocused, &settings, &mut ctx, &mut failed).await;

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
    #[tokio::test]
    async fn a_stacked_frame_gives_wanderer_nothing_to_react_to() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        process_frame_with_stacking(&reference, &settings, &mut ctx, &mut failed).await;

        let shifted = starfield(150, 150, 2.0);
        let outcome = process_frame_with_stacking(&shifted, &settings, &mut ctx, &mut failed).await;

        assert!(outcome.frame_added);
        assert_eq!(outcome.rejected_because, None);
        assert!(!outcome.stack_reset);
    }

    #[tokio::test]
    async fn a_frame_that_registers_joins_the_stack() {
        let settings = CaptureSettings::default();
        let mut ctx = None;
        let mut failed = false;

        let reference = starfield(150, 150, 0.0);
        process_frame_with_stacking(&reference, &settings, &mut ctx, &mut failed).await;

        let shifted = starfield(150, 150, 2.0);
        let outcome = process_frame_with_stacking(&shifted, &settings, &mut ctx, &mut failed).await;

        assert!(outcome.showing_stack);
        assert!(
            outcome.frame_added,
            "a cleanly shifted starfield should register and stack"
        );
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
        process_preview_frame(&mut frame_bg, &settings).unwrap();

        // Check if the RenderPipeline used background subtraction
        // Since we reordered the calls, get_render_pipeline_config will now correctly
        // return a config with background_subtraction = true if settings say so.
        let config = get_render_pipeline_config(&settings, false);
        assert!(config.background_subtraction);
    }
}
