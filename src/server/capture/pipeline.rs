use tracing::{debug, info, instrument, warn};

use super::context::{PlanetaryStackingContext, StackingContext};
use crate::background::{subtract_background_with_config, BackgroundConfig};
use crate::frame::Frame;
use crate::server::state::CaptureSettings;
use crate::stacking::{CometContext, COMET_PLUGIN};

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
) -> (Frame, bool) {
    // Initialize stacking context on first frame
    if stacking_ctx.is_none() {
        let ctx = StackingContext::new(frame.width(), frame.height(), frame.channels(), settings);
        if ctx.is_none() {
            warn!("Failed to create stacking context, falling back to single-frame mode");
            *stacking_failed = true;
            return (frame.clone(), false);
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
                return (frame.clone(), false);
            }
        }
        return (frame.clone(), true); // First frame is always "successful"
    }

    // Add frame to stack
    let frame_added = match ctx.add_frame(frame) {
        Ok(true) => {
            info!(frame_count = ctx.frame_count(), "Frame added to stack");
            true
        }
        Ok(false) => {
            info!(
                frame_count = ctx.frame_count(),
                "Frame registration failed, not added to stack"
            );
            false
        }
        Err(e) => {
            warn!(error = %e, "Error adding frame to stack");
            false
        }
    };

    // Return the current stacked result for display (raw, background subtraction applied in preview)
    match ctx.compute() {
        Ok(stacked) => (stacked, frame_added),
        Err(e) => {
            warn!(error = %e, "Failed to compute stack, using raw frame");
            (frame.clone(), false)
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
) -> (Frame, bool) {
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
            return (frame.clone(), false);
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
                return (frame.clone(), false);
            }
        }
        return (frame.clone(), true); // First frame is success
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
        Ok(stacked) => (stacked, frame_added),
        Err(e) => {
            warn!(error = %e, "Failed to compute comet stack, using raw frame");
            (frame.clone(), false)
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
) -> (Frame, bool) {
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
            return (frame.clone(), false);
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
                return (frame.clone(), false);
            }
        }
        return (frame.clone(), true); // First frame is success
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
        Ok(stacked) => (stacked, frame_added),
        Err(e) => {
            warn!(error = %e, "Failed to compute planetary stack, using raw frame");
            (frame.clone(), false)
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
    if pipeline_config.background_subtraction && frame.channels() == 3 {
        let _span0 = tracing::info_span!("background_neutralization").entered();
        match crate::render::compute_white_balance_grid(frame, 16, 25.0) {
            Ok(multipliers) => {
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

                let scale_lut = crate::render::stretch::cached_scale_lut(
                    pipeline_config.stretch_config.tone_mapping,
                    res.stretch_factor,
                    contrast_for_lut,
                );

                if can_fuse_contrast {
                    pipeline_config.contrast = false;
                }

                Some(crate::server::state::StretchResult {
                    black_point: res.black_point,
                    scale_lut,
                    color_intensity: pipeline_config.stretch_config.color_intensity,
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

/// Helper to get background configuration from capture settings
pub fn get_background_config(settings: &CaptureSettings) -> BackgroundConfig {
    BackgroundConfig::from_stretch_profile(settings.stretch_aggressiveness)
        .with_algorithm(settings.background_extraction_algorithm)
}

/// Helper to get a full render pipeline configuration from capture settings
/// How much of the eyepiece intensity slider's range actually reaches the
/// stretch. The slider is a comfort control, not a full remap of the tone curve.
const EYEPIECE_INTENSITY_SCALE: f32 = 0.4;

/// Sky level the eyepiece view aims for at full intensity.
const EYEPIECE_TARGET_BACKGROUND: f32 = 0.01;

/// Black-point factor the eyepiece view aims for at full intensity. Higher than
/// any stretch profile's default: trading faint-tail detail for a smoother sky
/// is the whole point of the eyepiece view.
const EYEPIECE_BLACK_POINT_SIGMA: f32 = 3.0;


pub fn get_render_pipeline_config(
    settings: &CaptureSettings,
    for_fits: bool,
) -> crate::render::RenderPipelineConfig {
    use crate::render::{AutoStretchConfig, RenderPipelineConfig};

    // Set configuration first, then explicit toggle last to override the config's auto-enable
    let mut config = RenderPipelineConfig::new()
        .with_background_config(get_background_config(settings))
        .with_background_subtraction(settings.background_subtraction);

    if settings.stacking_type == crate::stacking::StackingType::DeepSky
        || settings.stacking_type == crate::stacking::StackingType::Comet
    {
        config = config.with_scnr(true).with_scnr_amount(1.0);
    }

    if !for_fits {
        let use_aggressive_stretch = settings.stacking_type.uses_aggressive_stretch();
        let stretch_config = AutoStretchConfig::from_profile(
            !use_aggressive_stretch,
            settings.stretch_aggressiveness,
        )
        .with_color_intensity(1.0 + settings.auto_stretch_intensity);
        let saturation_config = settings.saturation_boost_config();

        // Similarly for auto-stretch and saturation boost: set config first, then explicit toggle
        config = config
            .with_stretch_config(stretch_config)
            .with_auto_stretch(settings.auto_stretch)
            .with_saturation_config(saturation_config)
            .with_saturation_boost(settings.saturation_boost)
            .with_contrast(settings.auto_stretch);

        // The 8-bit conversion is not a pipeline stage; the encoders apply it
        // where they write output bytes. It is set unconditionally because a
        // zero pedestal with dithering off reproduces a plain conversion.
        config.display = crate::render::DisplayOutput::default()
            .with_pedestal(settings.eyepiece.black_floor)
            .with_dither(settings.eyepiece.dither);

        // Apply eyepiece dark background enhancement
        let intensity = settings.eyepiece.intensity.clamp(0.0, 1.0) * EYEPIECE_INTENSITY_SCALE;
        if intensity > 0.0 && config.auto_stretch {
            // Interpolate target_background down for a darker sky
            config.stretch_config.target_background =
                config.stretch_config.target_background * (1.0 - intensity)
                    + EYEPIECE_TARGET_BACKGROUND * intensity;

            // Interpolate black_point_sigma *up*, which is what actually clips
            // noise: the black point is `mode - sigma * black_point_sigma`, so a
            // larger factor puts more of the sky's noise below black. This used
            // to interpolate down toward 1.0 under a comment claiming it clipped
            // noise, which had the opposite effect — at full intensity it left
            // more grain visible (9.7 output levels against 6.0) *and* clamped
            // more sky pixels to pure black (9.7 % against 1.8 %).
            config.stretch_config.black_point_sigma = (config.stretch_config.black_point_sigma
                * (1.0 - intensity)
                + EYEPIECE_BLACK_POINT_SIGMA * intensity)
                .clamp(0.5, 5.0);

            // Enhance contrast to make objects pop
            config.contrast = true;
            config.contrast_config.strength =
                config.contrast_config.strength * (1.0 - intensity) + 1.0 * intensity;
        }
    }

    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::BackgroundExtractionAlgorithm;
    use crate::frame::Frame;

    #[test]
    fn test_get_render_pipeline_config_respects_toggles() {
        let mut settings = CaptureSettings::default();

        // Test 1: Both enabled
        settings.background_subtraction = true;
        settings.auto_stretch = true;
        let config = get_render_pipeline_config(&settings, false);
        assert!(config.background_subtraction);
        assert!(config.auto_stretch);

        // Test 2: Both disabled
        settings.background_subtraction = false;
        settings.auto_stretch = false;
        let config = get_render_pipeline_config(&settings, false);
        assert!(!config.background_subtraction);
        assert!(!config.auto_stretch);

        // Test 3: Mixed
        settings.background_subtraction = true;
        settings.auto_stretch = false;
        let config = get_render_pipeline_config(&settings, false);
        assert!(config.background_subtraction);
        assert!(!config.auto_stretch);
    }

    #[test]
    fn test_eyepiece_intensity_interpolation() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = true;

        // Base config
        settings.eyepiece.intensity = 0.0;
        let base_config = get_render_pipeline_config(&settings, false);

        // Max intensity config (slider at 1.0, internal intensity 0.4)
        settings.eyepiece.intensity = 1.0;
        let max_config = get_render_pipeline_config(&settings, false);

        let blend = |base: f32, target: f32| base * 0.6 + target * 0.4;
        let expected_bg = blend(
            base_config.stretch_config.target_background,
            EYEPIECE_TARGET_BACKGROUND,
        );
        let expected_sigma = blend(
            base_config.stretch_config.black_point_sigma,
            EYEPIECE_BLACK_POINT_SIGMA,
        );
        let expected_contrast = blend(base_config.contrast_config.strength, 1.0);

        // Target background falls: a darker sky.
        assert!(
            max_config.stretch_config.target_background
                < base_config.stretch_config.target_background
        );
        assert!((max_config.stretch_config.target_background - expected_bg).abs() < 1e-5);

        // Black point sigma *rises*. The black point is `mode - sigma * factor`,
        // so a larger factor pushes more of the sky's noise below black — which
        // is what "clip noise" means. This assertion used to run the other way
        // and pinned a slider that made the eyepiece view grainier the further
        // it was pushed.
        assert!(
            max_config.stretch_config.black_point_sigma
                > base_config.stretch_config.black_point_sigma,
            "eyepiece intensity must raise black_point_sigma, got {} from {}",
            max_config.stretch_config.black_point_sigma,
            base_config.stretch_config.black_point_sigma
        );
        assert!((max_config.stretch_config.black_point_sigma - expected_sigma).abs() < 1e-5);

        // Contrast should increase
        assert!(max_config.contrast);
        assert!((max_config.contrast_config.strength - expected_contrast).abs() < 1e-5);

        // Half intensity config (slider at 0.5, internal intensity 0.2)
        settings.eyepiece.intensity = 0.5;
        let half_config = get_render_pipeline_config(&settings, false);

        let expected_half_bg =
            base_config.stretch_config.target_background * 0.8 + EYEPIECE_TARGET_BACKGROUND * 0.2;
        assert!((half_config.stretch_config.target_background - expected_half_bg).abs() < 1e-5);
    }

    /// The slider must move monotonically toward a smoother sky across its whole
    /// range, not just at the endpoints.
    #[test]
    fn eyepiece_intensity_monotonically_raises_the_black_point_factor() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = true;

        let mut previous = f32::MIN;
        for step in 0..=10 {
            settings.eyepiece.intensity = step as f32 / 10.0;
            let sigma = get_render_pipeline_config(&settings, false)
                .stretch_config
                .black_point_sigma;
            assert!(
                sigma >= previous,
                "black_point_sigma fell from {previous} to {sigma} at intensity {}",
                settings.eyepiece.intensity
            );
            previous = sigma;
        }
    }

    /// `black_point_sigma` is written directly rather than through
    /// `with_black_point_sigma`, so it carries its own clamp; a profile starting
    /// near the ceiling must not be pushed out of the solver's supported range.
    #[test]
    fn eyepiece_black_point_factor_stays_in_range() {
        let mut settings = CaptureSettings::default();
        settings.auto_stretch = true;
        for step in 0..=10 {
            settings.eyepiece.intensity = step as f32 / 10.0;
            let sigma = get_render_pipeline_config(&settings, false)
                .stretch_config
                .black_point_sigma;
            assert!(
                (0.5..=5.0).contains(&sigma),
                "black_point_sigma {sigma} outside the solver's range"
            );
        }
    }

    /// The display transform has to reach the encoders through the pipeline
    /// config — it is the only channel between the settings and the fused
    /// f32-to-u8 kernels.
    #[test]
    fn eyepiece_display_settings_reach_the_pipeline_config() {
        let mut settings = CaptureSettings::default();
        settings.eyepiece.black_floor = 0.05;
        settings.eyepiece.dither = true;

        let config = get_render_pipeline_config(&settings, false);
        assert!((config.display.pedestal - 0.05).abs() < 1e-6);
        assert!(config.display.dither);

        settings.eyepiece.black_floor = 0.0;
        settings.eyepiece.dither = false;
        let plain = get_render_pipeline_config(&settings, false);
        assert!(
            plain.display.is_plain(),
            "both settings off must reproduce a plain conversion"
        );
    }

    /// FITS is 32-bit linear data; the display transform is a property of the
    /// 8-bit conversion and must not follow the frame onto disk.
    #[test]
    fn fits_output_never_carries_the_display_transform() {
        let mut settings = CaptureSettings::default();
        settings.eyepiece.black_floor = 0.05;
        settings.eyepiece.dither = true;

        let config = get_render_pipeline_config(&settings, true);
        assert!(config.display.is_plain());
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
