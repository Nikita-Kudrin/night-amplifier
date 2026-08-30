//! Mapping `CaptureSettings` onto the configuration each capture stage takes.
//!
//! One direction only: settings in, stage configuration out. Nothing here
//! touches a frame except [`convert_captured_frame`], which is the one call that
//! ties the raw-CFA stage together and so belongs with the builders that decide
//! what is in it.
//!
//! Split out of `pipeline.rs`, which had grown past the size where the two
//! halves — "what should this stage be configured as" and "run a frame through
//! the stack" — could still be read as one thing.

use crate::background::BackgroundConfig;
use crate::camera::{CameraInfo, CameraResult, RawFrame};
use crate::cfa::{CfaPipeline, FpnFilter, HotPixelConfig, HotPixelFilter};
use crate::debayer::DebayerAlgorithm;
use crate::frame::Frame;
use crate::server::state::CaptureSettings;

/// The raw-CFA stage for the current settings.
///
/// Built when settings change rather than per frame — a stage may own
/// precomputed state, and a master dark will be the first that does.
pub fn build_cfa_pipeline(settings: &CaptureSettings) -> CfaPipeline {
    let correction = &settings.sensor_correction;
    let mut pipeline = CfaPipeline::new();

    // Hot pixels first: a column carrying hundreds of them would otherwise drag
    // its own median, and the FPN correction would spread that across the column.
    //
    // Kept for planetary, unlike the two stages below. A hot pixel is in the
    // same place in all 5000 frames of a lucky-imaging run, so it survives the
    // stack exactly as it survives a deep-sky one — and the filter is one-sided
    // and isolation-gated, so it cannot bite the disc it is imaging. The cost
    // that once argued against it is now a per-frame sweep only: the noise
    // estimate is cached across frames inside the stage.
    if correction.hot_pixel_rejection {
        pipeline = pipeline.with_stage(Box::new(HotPixelFilter::new(HotPixelConfig {
            sigma: correction.hot_pixel_sigma,
            ..HotPixelConfig::default()
        })));
    }
    // Not for planetary: the correction assumes each sensor line is mostly sky,
    // so its level measures readout rather than signal. A lunar or planetary
    // disc fills enough of a line to move that level, and flattening it would
    // carve bands across the disc.
    if correction.fpn_removal && settings.stacking_type != crate::stacking::StackingType::Planetary
    {
        pipeline = pipeline.with_stage(Box::new(FpnFilter));
    }
    pipeline
}

/// The demosaic the raw stage ends with.
///
/// Superpixel is skipped for planetary for the same reason `cfa::fpn` and the
/// denoisers are: it halves both dimensions, and resolution at the diffraction
/// limit is the entire product of lucky imaging. Leaving the setting to apply
/// there would quietly throw away three quarters of what the mode exists to
/// capture, on a toggle the observer set for deep sky.
pub fn debayer_algorithm(settings: &CaptureSettings) -> DebayerAlgorithm {
    if settings.sensor_correction.superpixel_debayer
        && settings.stacking_type != crate::stacking::StackingType::Planetary
    {
        DebayerAlgorithm::Superpixel
    } else {
        DebayerAlgorithm::Bilinear
    }
}

/// Decode a captured buffer, run the pre-debayer corrections, and demosaic.
///
/// The whole raw-CFA stage in one call, so the probe frame that sizes the
/// pipeline's channels and the frames that flow through them are produced the
/// same way — with `superpixel_debayer` on they differ by 4x in memory.
pub fn convert_captured_frame(
    raw: &RawFrame,
    info: &CameraInfo,
    cfa_pipeline: &CfaPipeline,
    algorithm: DebayerAlgorithm,
) -> CameraResult<Frame> {
    let mut cfa = raw.to_cfa_frame(info)?;
    {
        let _timer = crate::telemetry::metrics::time_stage(
            crate::telemetry::metrics::FrameStage::CfaCorrection,
        );
        cfa_pipeline.apply(&mut cfa);
    }
    cfa.debayer(algorithm)
        .map_err(|e| crate::camera::CameraError::ImageReadFailed(e.to_string()))
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

/// Map the denoise settings onto the config the encoders read.
///
/// Skipped for planetary the same way `cfa::fpn` is: lucky imaging exists to
/// recover the fine detail these filters remove, and a lunar disc is exactly the
/// low-contrast large-scale structure a wavelet threshold flattens.
fn denoise_config(settings: &CaptureSettings) -> crate::render::DenoiseConfig {
    use crate::render::{ChromaDenoiseConfig, DenoiseConfig, LumaDenoiseConfig};

    if settings.stacking_type == crate::stacking::StackingType::Planetary {
        return DenoiseConfig::OFF;
    }

    let d = &settings.denoise;
    DenoiseConfig {
        chroma: ChromaDenoiseConfig {
            enabled: d.chroma,
            strength: d.chroma_strength.clamp(0.0, 1.0),
            ..ChromaDenoiseConfig::default()
        },
        luma: LumaDenoiseConfig {
            enabled: d.luma,
            strength: d
                .luma_strength
                .clamp(0.0, crate::render::MAX_LUMA_STRENGTH),
            k: LumaDenoiseConfig::thresholds_for_star_protection(d.star_protection),
        },
    }
}

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

        config.denoise = denoise_config(settings);

        // Apply eyepiece dark background enhancement
        let intensity = settings.eyepiece.intensity.clamp(0.0, 1.0) * EYEPIECE_INTENSITY_SCALE;
        if intensity > 0.0 && config.auto_stretch {
            // Interpolate target_background down for a darker sky
            config.stretch_config.target_background = config.stretch_config.target_background
                * (1.0 - intensity)
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
    use crate::server::state::SensorCorrectionSettings;
    use crate::stacking::StackingType;

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

    fn settings_with(
        correction: SensorCorrectionSettings,
        stacking_type: StackingType,
    ) -> CaptureSettings {
        CaptureSettings {
            sensor_correction: correction,
            stacking_type,
            ..CaptureSettings::default()
        }
    }

    #[test]
    fn the_default_stage_list_corrects_hot_pixels_then_flattens_lines() {
        let settings = settings_with(SensorCorrectionSettings::default(), StackingType::DeepSky);
        assert_eq!(
            build_cfa_pipeline(&settings).stage_names(),
            vec!["hot_pixels", "row_column_fpn"]
        );
    }

    #[test]
    fn disabling_both_corrections_leaves_the_pre_debayer_seam_empty() {
        let settings = settings_with(
            SensorCorrectionSettings {
                hot_pixel_rejection: false,
                fpn_removal: false,
                ..SensorCorrectionSettings::default()
            },
            StackingType::DeepSky,
        );
        assert!(build_cfa_pipeline(&settings).is_empty());
    }

    #[test]
    fn planetary_keeps_hot_pixel_rejection_but_not_line_flattening() {
        let settings = settings_with(SensorCorrectionSettings::default(), StackingType::Planetary);
        assert_eq!(
            build_cfa_pipeline(&settings).stage_names(),
            vec!["hot_pixels"]
        );
    }

    #[test]
    fn superpixel_is_opt_in() {
        let settings = settings_with(SensorCorrectionSettings::default(), StackingType::DeepSky);
        assert_eq!(debayer_algorithm(&settings), DebayerAlgorithm::Bilinear);

        let settings = settings_with(
            SensorCorrectionSettings {
                superpixel_debayer: true,
                ..SensorCorrectionSettings::default()
            },
            StackingType::DeepSky,
        );
        assert_eq!(debayer_algorithm(&settings), DebayerAlgorithm::Superpixel);
    }

    /// Halving both dimensions is the opposite of what lucky imaging is for, so
    /// the setting does not follow the observer into planetary.
    #[test]
    fn superpixel_does_not_apply_to_planetary() {
        let settings = settings_with(
            SensorCorrectionSettings {
                superpixel_debayer: true,
                ..SensorCorrectionSettings::default()
            },
            StackingType::Planetary,
        );
        assert_eq!(debayer_algorithm(&settings), DebayerAlgorithm::Bilinear);
    }
}
