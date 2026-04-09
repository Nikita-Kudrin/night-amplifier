//! Non-linear stretching and final rendering for display
//!
//! This module provides the final step in the EAA pipeline: converting the
//! 32-bit floating-point stacked image into a visually pleasing 8-bit output.

mod autostretch;
mod black_point;
mod output;
pub mod pipeline;
pub mod simd;
mod stretch;
mod white_balance;

// Re-export all public items from submodules
pub use autostretch::{
    auto_stretch_default, auto_stretch_frame, compute_auto_stretch, solve_stretch_factor,
    solve_stretch_factor_newton, AutoStretchConfig, AutoStretchResult, StretchAggressiveness,
};
pub use black_point::{
    calculate_black_point, calculate_black_points, calculate_luminance_black_point,
    subtract_black_point, subtract_black_point_auto, subtract_black_point_uniform,
    BlackPointConfig,
};
pub use output::{
    apply_contrast_frame, apply_s_curve, downsample, finalize_for_display, frame_to_rgb8,
    frame_to_rgb8_simple, frame_to_rgb8_with_contrast, ContrastConfig, OutputConfig,
};
pub use stretch::{
    apply_shadow_saturation_boost, apply_tone_mapping, asinh, asinh_stretch,
    asinh_stretch_color_preserving, asinh_stretch_frame, estimate_tone_mapping_strength,
    SaturationBoostConfig, SaturationPlugin, ToneMappingAlgorithm, SATURATION_PLUGIN,
};
pub use white_balance::{
    compute_neutralization_multipliers, compute_white_balance_grid, neutralize_background,
    neutralize_background_auto,
};

// Re-export pipeline types for convenient access
pub use pipeline::{
    process_frame_deep_sky, process_frame_default, process_frame_planetary, PipelineResult,
    RenderPipeline, RenderPipelineConfig,
};

use crate::error::Result;
use crate::frame::Frame;

/// Convenience function: render with auto-stretch
pub fn render_with_auto_stretch(frame: &Frame) -> Result<Vec<u8>> {
    let pipeline = RenderPipeline::with_defaults();
    let (stretched, _) = pipeline.process_copy(frame)?;
    frame_to_rgb8_simple(&stretched)
}

/// Convenience function: render with custom asinh stretch
pub fn render_with_stretch(frame: &Frame, stretch: f32) -> Result<Vec<u8>> {
    let config = RenderPipelineConfig::default()
        .with_stretch_config(AutoStretchConfig::default().with_target_background(stretch));
    let pipeline = RenderPipeline::new(config);
    let (stretched, _) = pipeline.process_copy(frame)?;
    frame_to_rgb8_simple(&stretched)
}

/// Convenience function: render with no additional stretch
pub fn render_to_rgb8(frame: &Frame) -> Result<Vec<u8>> {
    frame_to_rgb8_simple(frame)
}
