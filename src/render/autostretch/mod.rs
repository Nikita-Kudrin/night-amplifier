//! Automatic stretch factor calculation
//!
//! This module provides the autostretch solver that calculates the optimal stretch factor
//! to map the image's background median to a target brightness.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::statistics::compute_image_stats;

mod config;
mod logic;
pub mod solver;
mod stats;

pub use config::{AutoStretchConfig, StretchAggressiveness};
pub use logic::{compute_auto_stretch, compute_auto_stretch_with_algorithm};
pub use solver::{solve_stretch_factor, solve_stretch_factor_newton};
pub use stats::{estimate_signal_fraction, AutoStretchResult};

use super::black_point::{
    calculate_black_points, subtract_black_point, subtract_black_point_uniform, BlackPointConfig,
};
use super::stretch::{apply_tone_mapping, ToneMappingAlgorithm};

/// Automatically stretch a frame to target background level
#[tracing::instrument(skip(frame, contrast_config))]
pub fn auto_stretch_frame(
    frame: &mut Frame,
    config: AutoStretchConfig,
    contrast_config: Option<&crate::render::output::ContrastConfig>,
) -> Result<AutoStretchResult> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "auto_stretch_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    let stats = {
        let _span = tracing::info_span!("compute_image_stats").entered();
        compute_image_stats(frame)?
    };
    let result = compute_auto_stretch_with_algorithm(frame, &stats, config, config.tone_mapping);

    if channels == 3 && config.per_channel_black_point {
        let bp_config = BlackPointConfig::new(config.black_point_sigma);
        let black_points = {
            let _span = tracing::info_span!("calculate_black_points").entered();
            calculate_black_points(frame, &stats, bp_config)?
        };
        subtract_black_point(frame, &black_points)?;

        if config.tone_mapping == ToneMappingAlgorithm::Mtf {
            let _span = tracing::info_span!("mtf_stretch_frame").entered();
            crate::render::stretch::mtf_stretch_frame(frame, result.midtones)?;
            if let Some(c_cfg) = contrast_config {
                let _span = tracing::info_span!("apply_contrast_frame").entered();
                crate::render::output::apply_contrast_frame(frame, c_cfg)?;
            }
        } else {
            let _span = tracing::info_span!("apply_fused_stretch_frame").entered();
            crate::render::stretch::apply_fused_stretch_frame(
                frame,
                0.0,
                config.tone_mapping,
                result.stretch_factor,
                config.color_intensity,
                contrast_config,
            )?;
        }
    } else if channels == 3 {
        if config.tone_mapping == ToneMappingAlgorithm::Mtf {
            subtract_black_point_uniform(frame, result.black_point)?;
            let _span = tracing::info_span!("mtf_stretch_frame").entered();
            crate::render::stretch::mtf_stretch_frame(frame, result.midtones)?;
            if let Some(c_cfg) = contrast_config {
                let _span = tracing::info_span!("apply_contrast_frame").entered();
                crate::render::output::apply_contrast_frame(frame, c_cfg)?;
            }
        } else {
            let _span = tracing::info_span!("apply_fused_stretch_frame").entered();
            crate::render::stretch::apply_fused_stretch_frame(
                frame,
                result.black_point,
                config.tone_mapping,
                result.stretch_factor,
                config.color_intensity,
                contrast_config,
            )?;
        }
    } else {
        subtract_black_point_uniform(frame, result.black_point)?;
        let _span = tracing::info_span!("apply_tone_mapping").entered();
        if config.tone_mapping == ToneMappingAlgorithm::Mtf {
            crate::render::stretch::mtf_stretch_frame(frame, result.midtones)?;
        } else {
            apply_tone_mapping(
                frame,
                config.tone_mapping,
                result.stretch_factor,
                config.color_intensity,
            )?;
        }
    }

    Ok(result)
}

/// Compute autostretch parameters and subtract black point, but do NOT stretch the frame.
/// Used for deferred rendering in the Mega-Kernel architecture.
#[tracing::instrument(skip(frame))]
pub fn prepare_auto_stretch_frame(
    frame: &mut Frame,
    config: AutoStretchConfig,
) -> Result<AutoStretchResult> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "prepare_auto_stretch_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    let stats = {
        let _span = tracing::info_span!("compute_image_stats").entered();
        compute_image_stats(frame)?
    };
    let mut result =
        compute_auto_stretch_with_algorithm(frame, &stats, config, config.tone_mapping);

    if channels == 3 && config.per_channel_black_point {
        let bp_config = BlackPointConfig::new(config.black_point_sigma);
        let black_points = {
            let _span = tracing::info_span!("calculate_black_points").entered();
            calculate_black_points(frame, &stats, bp_config)?
        };
        subtract_black_point(frame, &black_points)?;
        // Set black point to 0 in the result since we already subtracted it per-channel
        result.black_point = 0.0;
    } else {
        subtract_black_point_uniform(frame, result.black_point)?;
        // Set black point to 0 in the result since we already subtracted it uniformly
        result.black_point = 0.0;
    }

    Ok(result)
}

/// Automatically stretch a frame with default configuration
pub fn auto_stretch_default(frame: &mut Frame) -> Result<AutoStretchResult> {
    auto_stretch_frame(frame, AutoStretchConfig::default(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autostretch_config_defaults() {
        let config = AutoStretchConfig::default();
        assert!((config.target_background - 0.10).abs() < 1e-6);
    }

    #[test]
    fn test_auto_stretch_frame_end_to_end() {
        let background = 0.03;
        // `set_pixel`, not index arithmetic: a fixture that encodes the layout cannot
        // detect a layout bug. The identical fixture in `server::capture::storage` was
        // converted during the planar migration and this one was missed.
        let mut frame = Frame::zeros(64, 64, 3).unwrap();

        let mut seed: u32 = 54321;
        for y in 0..64 {
            for x in 0..64 {
                for c in 0..3 {
                    seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
                    let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.005;
                    frame.set_pixel(x, y, c, background + noise);
                }
            }
        }
        let config = AutoStretchConfig::new().with_target_background(0.15);
        let result = auto_stretch_frame(&mut frame, config, None).unwrap();

        assert!(result.converged);
        let bg = frame.get_pixel(0, 0, 0);
        assert!(bg > 0.05 && bg < 0.30);
    }

    #[test]
    fn test_auto_stretch_frame_preserves_colors() {
        let mut data = vec![0.0f32; 32 * 32 * 3];

        let plane = 32 * 32;
        for i in 0..plane {
            data[i] = 0.04;
            data[plane + i] = 0.05;
            data[plane * 2 + i] = 0.06;
        }

        let idx = 16 * 32 + 16;
        data[idx] = 0.8;
        data[plane + idx] = 0.3;
        data[plane * 2 + idx] = 0.2;

        let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();
        auto_stretch_frame(&mut frame, AutoStretchConfig::default(), None).unwrap();

        let star_r = frame.get_pixel(16, 16, 0);
        let star_g = frame.get_pixel(16, 16, 1);
        let star_b = frame.get_pixel(16, 16, 2);

        assert!(star_r > star_g && star_r > star_b);
        assert!(star_g > star_b);
    }

    #[test]
    fn test_auto_stretch_frame_wrong_channels() {
        let mut frame = Frame::filled(10, 10, 2, 0.5).unwrap();
        let result = auto_stretch_frame(&mut frame, AutoStretchConfig::default(), None);
        assert!(matches!(result, Err(StackError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_auto_stretch_default_convenience() {
        let data = vec![0.05f32; 32 * 32 * 3];
        let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();

        let result = auto_stretch_default(&mut frame).unwrap();
        assert!(result.converged);
    }
}
