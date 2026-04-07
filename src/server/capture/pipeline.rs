use tracing::{debug, info, warn};

use super::context::{PlanetaryStackingContext, StackingContext};
use crate::background::{subtract_background_with_config, BackgroundConfig};
use crate::frame::Frame;
use crate::server::state::CaptureSettings;
use crate::stacking::{CometContext, COMET_PLUGIN};

/// Process a frame through the stacking pipeline
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
pub async fn process_frame_with_comet_stacking(
    frame: &Frame,
    settings: &CaptureSettings,
    comet_ctx: &mut Option<Box<dyn CometContext>>,
    stacking_failed: &mut bool,
) -> (Frame, bool) {
    // Initialize comet stacking context on first frame using plugin
    if comet_ctx.is_none() {
        let plugin = COMET_PLUGIN.get();
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

/// Process a frame for preview display using the unified render pipeline
pub fn process_preview_frame(
    frame: &mut Frame,
    settings: &CaptureSettings,
) -> crate::error::Result<()> {
    use crate::render::RenderPipeline;

    let _span = tracing::info_span!("process_preview_frame").entered();

    let pipeline_config = get_render_pipeline_config(settings, false);
    let pipeline = RenderPipeline::new(pipeline_config);
    let _ = pipeline.process(frame)?;

    Ok(())
}

/// Helper to get background configuration from capture settings
pub fn get_background_config(settings: &CaptureSettings) -> BackgroundConfig {
    BackgroundConfig::from_stretch_profile(settings.stretch_aggressiveness)
        .with_algorithm(settings.background_extraction_algorithm)
}

/// Helper to get a full render pipeline configuration from capture settings
pub fn get_render_pipeline_config(
    settings: &CaptureSettings,
    for_fits: bool,
) -> crate::render::RenderPipelineConfig {
    use crate::render::{AutoStretchConfig, RenderPipelineConfig};

    // Set configuration first, then explicit toggle last to override the config's auto-enable
    let mut config = RenderPipelineConfig::new()
        .with_background_config(get_background_config(settings))
        .with_background_subtraction(settings.background_subtraction);

    if !for_fits {
        let use_aggressive_stretch = settings.stacking_type.uses_aggressive_stretch();
        let stretch_config = AutoStretchConfig::from_profile(
            !use_aggressive_stretch,
            settings.stretch_aggressiveness,
        );
        let saturation_config = settings.saturation_boost_config();

        // Similarly for auto-stretch and saturation boost: set config first, then explicit toggle
        config = config
            .with_stretch_config(stretch_config)
            .with_auto_stretch(settings.auto_stretch)
            .with_saturation_config(saturation_config)
            .with_saturation_boost(settings.saturation_boost)
            .with_contrast(settings.auto_stretch);
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
