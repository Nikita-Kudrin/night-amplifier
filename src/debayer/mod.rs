//! Debayering (demosaicing): converts single-channel Bayer CFA data from colour
//! astronomy cameras into full RGB, since each sensor pixel has only one colour
//! filter. The four 2x2 patterns (named by top-left arrangement): RGGB (most common
//! in astro cameras), BGGR, GRBG, GBRG.
//!
//! Algorithms: **Bilinear** (fast neighbour averaging, for live stacking), **VNG**
//! (edge-aware, better for final output), **Superpixel** (one RGB pixel per 2x2 quad,
//! no interpolation, half resolution). Submodules: `pattern`, `detection` (auto
//! pattern detection), `algorithms`.

mod algorithms;
mod detection;
mod pattern;

pub use detection::{detect_cfa_pattern, PatternDetectionResult};
pub use pattern::CfaPattern;

use crate::error::{Result, StackError};
use crate::frame::Frame;
use tracing::instrument;

use algorithms::{debayer_bilinear, debayer_bilinear_to_rgb8, debayer_superpixel, debayer_vng};

/// Debayering algorithm selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DebayerAlgorithm {
    /// Simple bilinear interpolation - fast, suitable for live preview
    Bilinear,

    /// Variable Number of Gradients - higher quality, edge-aware
    #[default]
    Vng,

    /// One RGB pixel per 2x2 CFA quad - half resolution, no interpolation
    ///
    /// Interpolates nothing, so it invents no chroma noise and keeps a surviving
    /// hot sample inside one output pixel. Halves both dimensions, which is free
    /// on a sensor that oversamples the eyepiece screen and a real loss on one
    /// that does not.
    Superpixel,
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

        // Timed here rather than in each camera provider: every colour camera
        // and the simulator funnel through this one call.
        let _timer =
            crate::telemetry::metrics::time_stage(crate::telemetry::metrics::FrameStage::Debayer);

        match self.config.algorithm {
            DebayerAlgorithm::Bilinear => debayer_bilinear(frame, self.config.pattern),
            DebayerAlgorithm::Vng => debayer_vng(frame, self.config.pattern),
            DebayerAlgorithm::Superpixel => debayer_superpixel(frame, self.config.pattern),
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

/// Fast path: debayer with bilinear interpolation straight to interleaved RGB8,
/// skipping the intermediate f32 `Frame`.
///
/// # Errors
/// `ChannelMismatch` unless the frame is single-channel — not redundant with
/// [`Debayerer::debayer`]'s check, since this bypasses `Debayerer` entirely and a
/// 3-channel frame would otherwise be silently demosaiced out of its red plane with
/// nothing faulting (`frame.data()` starts at plane 0, a Bayer walk over `width *
/// height` stays in bounds). No in-tree production caller as of 2026-08 (the
/// streaming encoder debayers at capture instead); kept as public API and because
/// `layout_tests` uses it for interleaved-write coverage its f32 sibling lacks.
pub fn debayer_bilinear_to_rgb8_fast(frame: &Frame, pattern: CfaPattern) -> Result<Vec<u8>> {
    if frame.channels() != 1 {
        return Err(StackError::ChannelMismatch {
            expected: 1,
            actual: frame.channels(),
        });
    }
    debayer_bilinear_to_rgb8(frame, pattern)
}

/// Debayer a frame with automatic CFA pattern detection: analyzes the raw Bayer data
/// to detect the pattern, then debayers with it. Returns (RGB frame, detection result).
///
/// ```
/// use night_amplifier::{Frame, PixelFormat};
/// use night_amplifier::debayer::debayer_auto;
///
/// let raw_data = vec![0u8; 100 * 100 * 2];
/// let frame = Frame::from_raw(&raw_data, 100, 100, 1, PixelFormat::Bayer16).unwrap();
/// let (rgb_frame, detection) = debayer_auto(&frame).unwrap();
/// println!("Used pattern {:?} with {:.0}% confidence", detection.pattern, detection.confidence * 100.0);
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

    #[test]
    fn test_debayer_pure_white_frame() {
        let data = vec![1.0f32; 16];
        let frame = Frame::from_f32_vec(data, 4, 4, 1).unwrap();
        let result = debayer(&frame).unwrap();
        for &v in result.data() {
            assert!((v - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn test_debayer_pure_black_frame() {
        let data = vec![0.0f32; 16];
        let frame = Frame::from_f32_vec(data, 4, 4, 1).unwrap();
        let result = debayer(&frame).unwrap();
        for &v in result.data() {
            assert!(v.abs() < 1e-6);
        }
    }

    #[test]
    fn test_debayer_1x1_frame() {
        let data = vec![0.5f32; 1];
        let frame = Frame::from_f32_vec(data, 1, 1, 1).unwrap();
        let result = debayer(&frame).unwrap();
        assert_eq!(result.width(), 1);
        assert_eq!(result.height(), 1);
        assert_eq!(result.channels(), 3);
        // Fallback to scalar duplication
        for &v in result.data() {
            assert!((v - 0.5).abs() < 1e-6);
        }
    }
}
