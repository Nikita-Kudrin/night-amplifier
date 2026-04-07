//! Non-linear stretch and tone mapping functions for image enhancement
//!
//! This module provides the core stretch/tone mapping functions used in astronomical
//! imaging to boost faint details while preserving bright stars.

pub mod asinh;
pub mod mtf;
pub mod saturation;

// Re-export public items to maintain API compatibility
pub use asinh::{asinh, asinh_stretch, asinh_stretch_color_preserving, asinh_stretch_frame};
pub use mtf::{mtf, mtf_stretch_color_preserving, mtf_stretch_frame, solve_mtf_midtone};
pub use saturation::{
    apply_shadow_saturation_boost, SaturationBoostConfig, SaturationPlugin, SATURATION_PLUGIN,
};

use crate::error::Result;
use crate::frame::Frame;
use serde::{Deserialize, Serialize};

/// Tone mapping algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToneMappingAlgorithm {
    /// Asinh (inverse hyperbolic sine) stretch - default for astrophotography
    #[default]
    Asinh,
    /// Midtones Transfer Function (Histogram Transformation)
    Mtf,
}

/// Apply tone mapping to a frame using the specified algorithm
///
/// # Arguments
/// * `frame` - Mutable reference to an RGB frame
/// * `algorithm` - Which tone mapping algorithm to use
/// * `strength` - Algorithm-specific strength parameter:
///   - For Asinh: stretch factor (typical: 1.0-20.0)
///   - For MTF: midtone parameter (typical: 0.1-0.4 for boost)
pub fn apply_tone_mapping(
    frame: &mut Frame,
    algorithm: ToneMappingAlgorithm,
    strength: f32,
) -> Result<()> {
    match algorithm {
        ToneMappingAlgorithm::Asinh => asinh_stretch_frame(frame, strength),
        ToneMappingAlgorithm::Mtf => mtf_stretch_frame(frame, strength),
    }
}

/// Estimate the strength parameter to achieve target background brightness
///
/// # Arguments
/// * `algorithm` - Which tone mapping algorithm
/// * `input_median` - Current median brightness of the image
/// * `target_output` - Desired output brightness (typically 0.15-0.25)
///
/// # Returns
/// Recommended strength parameter for the algorithm
pub fn estimate_tone_mapping_strength(
    algorithm: ToneMappingAlgorithm,
    input_median: f32,
    target_output: f32,
) -> f32 {
    match algorithm {
        ToneMappingAlgorithm::Asinh => asinh::estimate_stretch_factor(input_median, target_output),
        ToneMappingAlgorithm::Mtf => mtf::solve_mtf_midtone(input_median, target_output),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_tone_mapping_asinh() {
        let mut frame = Frame::filled(10, 10, 3, 0.2).unwrap();
        apply_tone_mapping(&mut frame, ToneMappingAlgorithm::Asinh, 5.0).unwrap();

        // Values should be boosted
        assert!(frame.get_pixel(5, 5, 0) > 0.2);
    }

    #[test]
    fn test_estimate_tone_mapping_strength() {
        let input = 0.1;
        let target = 0.2;

        let asinh_strength =
            estimate_tone_mapping_strength(ToneMappingAlgorithm::Asinh, input, target);

        // Both should produce reasonable positive values
        assert!(asinh_strength > 0.0);
    }

    #[test]
    fn test_tone_mapping_algorithm_default() {
        let algo = ToneMappingAlgorithm::default();
        assert_eq!(algo, ToneMappingAlgorithm::Asinh);
    }
}
