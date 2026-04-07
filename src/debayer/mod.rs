//! Debayering (demosaicing) module for converting CFA raw data to RGB
//!
//! This module handles conversion of single-channel Bayer pattern data from
//! color astronomy cameras into full RGB images. Most modern astronomy cameras
//! use a Color Filter Array (CFA) where each pixel has only one color filter,
//! requiring interpolation to reconstruct the full color information.
//!
//! # Bayer Patterns
//!
//! The four standard 2x2 Bayer patterns are named by their top-left 2x2 arrangement:
//! - RGGB: Red-Green / Green-Blue (most common in astronomy cameras)
//! - BGGR: Blue-Green / Green-Red
//! - GRBG: Green-Red / Blue-Green
//! - GBRG: Green-Blue / Red-Green
//!
//! # Algorithms
//!
//! - **Bilinear**: Fast, simple averaging of neighbors. Good for live stacking.
//! - **VNG** (Variable Number of Gradients): Higher quality, uses edge detection
//!   to avoid interpolating across sharp boundaries. Better for final output.
//!
//! # Module Structure
//!
//! - `pattern` - CFA pattern definitions
//! - `detection` - Automatic pattern detection
//! - `algorithms` - Debayering algorithm implementations

mod algorithms;
mod detection;
mod pattern;

pub use detection::{detect_cfa_pattern, PatternDetectionResult};
pub use pattern::CfaPattern;

use crate::error::{Result, StackError};
use crate::frame::Frame;
use tracing::instrument;

use algorithms::{debayer_bilinear, debayer_vng};

/// Debayering algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebayerAlgorithm {
    /// Simple bilinear interpolation - fast, suitable for live preview
    Bilinear,

    /// Variable Number of Gradients - higher quality, edge-aware
    #[default]
    Vng,
}

/// Configuration for debayering operations
#[derive(Debug, Clone)]
pub struct DebayerConfig {
    /// The CFA pattern of the sensor
    pub pattern: CfaPattern,
    /// The debayering algorithm to use
    pub algorithm: DebayerAlgorithm,
}

impl Default for DebayerConfig {
    fn default() -> Self {
        Self {
            pattern: CfaPattern::Rggb,
            algorithm: DebayerAlgorithm::Bilinear,
        }
    }
}

impl DebayerConfig {
    /// Create a new debayer config with the specified pattern
    pub fn new(pattern: CfaPattern) -> Self {
        Self {
            pattern,
            algorithm: DebayerAlgorithm::Bilinear,
        }
    }

    /// Set the debayering algorithm
    pub fn with_algorithm(mut self, algorithm: DebayerAlgorithm) -> Self {
        self.algorithm = algorithm;
        self
    }
}

/// Debayerer for converting CFA raw data to RGB
pub struct Debayerer {
    config: DebayerConfig,
}

impl Debayerer {
    /// Create a new debayerer with the given configuration
    pub fn new(config: DebayerConfig) -> Self {
        Self { config }
    }

    /// Create a debayerer with default settings (RGGB, bilinear)
    pub fn with_defaults() -> Self {
        Self::new(DebayerConfig::default())
    }

    /// Create a debayerer for a specific CFA pattern
    pub fn with_pattern(pattern: CfaPattern) -> Self {
        Self::new(DebayerConfig::new(pattern))
    }

    /// Debayer a single-channel frame to RGB
    ///
    /// # Arguments
    /// * `frame` - Single-channel (grayscale) frame containing raw Bayer data
    ///
    /// # Returns
    /// A new 3-channel RGB frame with interpolated color values
    #[instrument(skip(self, frame), fields(
        resolution = %format!("{}x{}", frame.width(), frame.height()),
        pattern = ?self.config.pattern,
        algorithm = ?self.config.algorithm
    ))]
    pub fn debayer(&self, frame: &Frame) -> Result<Frame> {
        if frame.channels() != 1 {
            return Err(StackError::ChannelMismatch {
                expected: 1,
                actual: frame.channels(),
            });
        }

        match self.config.algorithm {
            DebayerAlgorithm::Bilinear => debayer_bilinear(frame, self.config.pattern),
            DebayerAlgorithm::Vng => debayer_vng(frame, self.config.pattern),
        }
    }
}

/// Convenience function to debayer a frame with default settings (RGGB, bilinear)
pub fn debayer(frame: &Frame) -> Result<Frame> {
    Debayerer::with_defaults().debayer(frame)
}

/// Convenience function to debayer with a specific CFA pattern
pub fn debayer_with_pattern(frame: &Frame, pattern: CfaPattern) -> Result<Frame> {
    Debayerer::with_pattern(pattern).debayer(frame)
}

/// Convenience function to debayer with full configuration
pub fn debayer_with_config(frame: &Frame, config: DebayerConfig) -> Result<Frame> {
    Debayerer::new(config).debayer(frame)
}

/// Debayer a frame with automatic CFA pattern detection
///
/// This function first analyzes the raw Bayer data to detect the CFA pattern,
/// then applies debayering with the detected pattern.
///
/// # Arguments
/// * `frame` - Single-channel frame containing raw Bayer data
///
/// # Returns
/// A tuple of (RGB frame, detection result with pattern and confidence)
///
/// # Example
/// ```
/// use night_amplifier::{Frame, PixelFormat};
/// use night_amplifier::debayer::debayer_auto;
///
/// let raw_data = vec![0u8; 100 * 100 * 2];
/// let frame = Frame::from_raw(&raw_data, 100, 100, 1, PixelFormat::Bayer16).unwrap();
/// let (rgb_frame, detection) = debayer_auto(&frame).unwrap();
/// println!("Used pattern {:?} with {:.0}% confidence",
///          detection.pattern, detection.confidence * 100.0);
/// ```
pub fn debayer_auto(frame: &Frame) -> Result<(Frame, PatternDetectionResult)> {
    let detection = detect_cfa_pattern(frame)?;
    let rgb = debayer_with_pattern(frame, detection.pattern)?;
    Ok((rgb, detection))
}

/// Debayer with automatic pattern detection and specified algorithm
///
/// # Arguments
/// * `frame` - Single-channel frame containing raw Bayer data
/// * `algorithm` - The debayering algorithm to use
///
/// # Returns
/// A tuple of (RGB frame, detection result)
pub fn debayer_auto_with_algorithm(
    frame: &Frame,
    algorithm: DebayerAlgorithm,
) -> Result<(Frame, PatternDetectionResult)> {
    let detection = detect_cfa_pattern(frame)?;
    let config = DebayerConfig::new(detection.pattern).with_algorithm(algorithm);
    let rgb = debayer_with_config(frame, config)?;
    Ok((rgb, detection))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debayer_creates_rgb_output() {
        let mut data = vec![0.5f32; 16];
        data[0] = 0.8;
        data[5] = 0.3;

        let frame = Frame::from_f32_vec(data, 4, 4, 1).unwrap();
        let result = debayer(&frame).unwrap();

        assert_eq!(result.channels(), 3);
        assert_eq!(result.width(), 4);
        assert_eq!(result.height(), 4);
    }

    #[test]
    fn test_debayer_channel_mismatch_error() {
        let frame = Frame::zeros(4, 4, 3).unwrap();
        let result = debayer(&frame);
        assert!(matches!(result, Err(StackError::ChannelMismatch { .. })));
    }

    #[test]
    fn test_debayer_vng_algorithm() {
        let data = vec![0.5f32; 64];
        let frame = Frame::from_f32_vec(data, 8, 8, 1).unwrap();

        let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);
        let result = debayer_with_config(&frame, config).unwrap();

        assert_eq!(result.channels(), 3);
    }

    #[test]
    fn test_pure_red_pattern() {
        let mut data = vec![0.0f32; 16];
        data[0] = 1.0;
        data[2] = 1.0;
        data[8] = 1.0;
        data[10] = 1.0;

        let frame = Frame::from_f32_vec(data, 4, 4, 1).unwrap();
        let result = debayer(&frame).unwrap();

        assert!(result.get_pixel(0, 0, 0) > 0.5);
        assert!(result.get_pixel(0, 0, 2) < 0.5);
    }

    #[test]
    fn test_debayer_preserves_dimensions() {
        let frame = Frame::zeros(1920, 1080, 1).unwrap();
        let result = debayer(&frame).unwrap();

        assert_eq!(result.width(), 1920);
        assert_eq!(result.height(), 1080);
        assert_eq!(result.channels(), 3);
    }

    #[test]
    fn test_debayer_auto() {
        let mut data = vec![0.0f32; 64 * 64];
        for y in 0..64 {
            for x in 0..64 {
                let idx = y * 64 + x;
                data[idx] = match ((y & 1), (x & 1)) {
                    (0, 0) => 0.8,
                    (0, 1) => 0.5,
                    (1, 0) => 0.5,
                    (1, 1) => 0.2,
                    _ => unreachable!(),
                };
            }
        }

        let frame = Frame::from_f32_vec(data, 64, 64, 1).unwrap();
        let (rgb, detection) = debayer_auto(&frame).unwrap();

        assert_eq!(rgb.channels(), 3);
        assert_eq!(detection.pattern, CfaPattern::Rggb);
    }

    #[test]
    fn test_vng_odd_dimensions() {
        // Test VNG with odd dimensions that don't divide evenly by 4
        // This exercises the SIMD remainder handling
        for (width, height) in [(17, 17), (33, 33), (65, 65), (127, 127)] {
            let data = vec![0.3f32; width * height];
            let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();

            let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);
            let result = debayer_with_config(&frame, config).unwrap();

            assert_eq!(result.width(), width);
            assert_eq!(result.height(), height);
            assert_eq!(result.channels(), 3);

            // Verify no NaN or infinite values
            for val in result.data() {
                assert!(val.is_finite(), "Found non-finite value in VNG output");
            }
        }
    }

    #[test]
    fn test_vng_small_images() {
        // Test VNG with images smaller than SIMD width (< 8 pixels wide)
        // These should fall back entirely to scalar/bilinear processing
        for (width, height) in [(4, 4), (5, 5), (6, 6), (7, 7), (8, 8)] {
            let data = vec![0.4f32; width * height];
            let frame = Frame::from_f32_vec(data, width, height, 1).unwrap();

            let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);
            let result = debayer_with_config(&frame, config).unwrap();

            assert_eq!(result.width(), width);
            assert_eq!(result.height(), height);
            assert_eq!(result.channels(), 3);
        }
    }

    #[test]
    fn test_vng_output_valid() {
        // Verify VNG produces valid output (no NaN, Inf, or out-of-range values)
        let mut data = vec![0.0f32; 64 * 64];
        // Create a realistic Bayer pattern with some variation
        for y in 0..64 {
            for x in 0..64 {
                let base = 0.1 + 0.01 * (x as f32 + y as f32) / 128.0;
                data[y * 64 + x] = base + 0.05 * ((x * 7 + y * 11) as f32 * 0.1).sin().abs();
            }
        }

        let frame = Frame::from_f32_vec(data, 64, 64, 1).unwrap();
        let config = DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Vng);
        let result = debayer_with_config(&frame, config).unwrap();

        // Check that all output values are finite and in valid range
        for (i, val) in result.data().iter().enumerate() {
            assert!(val.is_finite(), "Non-finite value at index {}: {}", i, val);
            assert!(*val >= 0.0, "Negative value at index {}: {}", i, val);
            assert!(*val <= 1.5, "Value too large at index {}: {}", i, val); // Allow slight overshoot from interpolation
        }
    }
}
