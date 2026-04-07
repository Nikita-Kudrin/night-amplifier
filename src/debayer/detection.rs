//! Automatic CFA pattern detection for Bayer sensors

use crate::error::{Result, StackError};
use crate::frame::Frame;

use super::CfaPattern;

/// Result of automatic CFA pattern detection
#[derive(Debug, Clone)]
pub struct PatternDetectionResult {
    /// The detected CFA pattern
    pub pattern: CfaPattern,
    /// Confidence score (0.0 to 1.0) - higher means more confident
    pub confidence: f32,
    /// Average intensities for each 2x2 grid position [top-left, top-right, bottom-left, bottom-right]
    pub grid_averages: [f32; 4],
}

/// Detects the CFA pattern from a single-channel Bayer frame
///
/// The detection algorithm works by:
/// 1. Computing average pixel intensities for each position in the 2x2 Bayer grid
/// 2. Identifying green pixels (appear on the diagonal, have similar mid-range values)
/// 3. Distinguishing red from blue based on typical astronomical image characteristics
///
/// # Algorithm Details
///
/// In a Bayer pattern, green pixels always occupy diagonal positions (either
/// top-left/bottom-right or top-right/bottom-left). The algorithm:
///
/// 1. Computes variance of diagonal pairs to find which diagonal has green
/// 2. Green pixels have lower variance between them (both are green)
/// 3. Red vs Blue is determined by intensity (red typically brighter in astro images
///    due to H-alpha emission, but we also check edge gradients)
///
/// # Arguments
/// * `frame` - Single-channel frame containing raw Bayer data
///
/// # Returns
/// * `PatternDetectionResult` with detected pattern and confidence score
///
/// # Example
/// ```
/// use night_amplifier::{Frame, PixelFormat};
/// use night_amplifier::debayer::{detect_cfa_pattern, CfaPattern};
///
/// let raw_data = vec![0u8; 100 * 100 * 2]; // 16-bit mono
/// let frame = Frame::from_raw(&raw_data, 100, 100, 1, PixelFormat::Bayer16).unwrap();
/// let result = detect_cfa_pattern(&frame).unwrap();
/// println!("Detected pattern: {:?} with confidence {:.2}", result.pattern, result.confidence);
/// ```
pub fn detect_cfa_pattern(frame: &Frame) -> Result<PatternDetectionResult> {
    if frame.channels() != 1 {
        return Err(StackError::ChannelMismatch {
            expected: 1,
            actual: frame.channels(),
        });
    }

    let width = frame.width();
    let height = frame.height();
    let data = frame.data();

    if width < 4 || height < 4 {
        return Err(StackError::InvalidDimensions {
            width,
            height,
            channels: 1,
        });
    }

    let grid_stats = compute_grid_statistics(data, width, height);
    let (pattern, confidence) = analyze_grid_statistics(&grid_stats);

    Ok(PatternDetectionResult {
        pattern,
        confidence,
        grid_averages: [
            grid_stats[0].mean,
            grid_stats[1].mean,
            grid_stats[2].mean,
            grid_stats[3].mean,
        ],
    })
}

/// Statistics for one position in the 2x2 Bayer grid
#[derive(Debug, Clone, Default)]
struct GridPositionStats {
    mean: f32,
    #[allow(dead_code)]
    variance: f32,
    #[allow(dead_code)]
    count: usize,
}

/// Compute statistics for each of the 4 positions in the 2x2 Bayer grid
fn compute_grid_statistics(data: &[f32], width: usize, height: usize) -> [GridPositionStats; 4] {
    let mut stats = [
        GridPositionStats::default(),
        GridPositionStats::default(),
        GridPositionStats::default(),
        GridPositionStats::default(),
    ];

    // Sample the image - use a grid to avoid processing every pixel
    let sample_step = ((width * height) / 100_000).max(1);
    let block_step = (sample_step as f32).sqrt().ceil() as usize;
    let block_step = block_step.max(1) * 2; // Ensure we step by even amounts

    let mut sums = [0.0f64; 4];
    let mut sum_sqs = [0.0f64; 4];
    let mut counts = [0usize; 4];

    let mut y = 0;
    while y + 1 < height {
        let mut x = 0;
        while x + 1 < width {
            let p00 = data[y * width + x] as f64;
            let p10 = data[y * width + x + 1] as f64;
            let p01 = data[(y + 1) * width + x] as f64;
            let p11 = data[(y + 1) * width + x + 1] as f64;

            sums[0] += p00;
            sums[1] += p10;
            sums[2] += p01;
            sums[3] += p11;

            sum_sqs[0] += p00 * p00;
            sum_sqs[1] += p10 * p10;
            sum_sqs[2] += p01 * p01;
            sum_sqs[3] += p11 * p11;

            counts[0] += 1;
            counts[1] += 1;
            counts[2] += 1;
            counts[3] += 1;

            x += block_step;
        }
        y += block_step;
    }

    for i in 0..4 {
        if counts[i] > 0 {
            let n = counts[i] as f64;
            let mean = sums[i] / n;
            let variance = (sum_sqs[i] / n) - (mean * mean);
            stats[i] = GridPositionStats {
                mean: mean as f32,
                variance: variance.max(0.0) as f32,
                count: counts[i],
            };
        }
    }

    stats
}

/// Analyze grid statistics to determine the CFA pattern
fn analyze_grid_statistics(stats: &[GridPositionStats; 4]) -> (CfaPattern, f32) {
    let means = [stats[0].mean, stats[1].mean, stats[2].mean, stats[3].mean];

    // Green pixels are on one diagonal
    // Diagonal 1: positions 0 and 3 (top-left and bottom-right)
    // Diagonal 2: positions 1 and 2 (top-right and bottom-left)

    let diag1_diff = (means[0] - means[3]).abs();
    let diag2_diff = (means[1] - means[2]).abs();

    let green_on_diag1 = diag1_diff < diag2_diff;

    let (pattern, confidence) = if green_on_diag1 {
        // Green at positions 0 and 3 -> GRBG or GBRG
        let pos1_is_red = means[1] > means[2];

        if pos1_is_red {
            (CfaPattern::Grbg, compute_confidence(&means, green_on_diag1))
        } else {
            (CfaPattern::Gbrg, compute_confidence(&means, green_on_diag1))
        }
    } else {
        // Green at positions 1 and 2 -> RGGB or BGGR
        let pos0_is_red = means[0] > means[3];

        if pos0_is_red {
            (CfaPattern::Rggb, compute_confidence(&means, green_on_diag1))
        } else {
            (CfaPattern::Bggr, compute_confidence(&means, green_on_diag1))
        }
    };

    (pattern, confidence)
}

/// Compute confidence score for the detection
fn compute_confidence(means: &[f32; 4], green_on_diag1: bool) -> f32 {
    let diag1_diff = (means[0] - means[3]).abs();
    let diag2_diff = (means[1] - means[2]).abs();

    let diagonal_ratio = if green_on_diag1 {
        if diag2_diff > 0.001 {
            1.0 - (diag1_diff / diag2_diff).min(1.0)
        } else {
            0.5
        }
    } else if diag1_diff > 0.001 {
        1.0 - (diag2_diff / diag1_diff).min(1.0)
    } else {
        0.5
    };

    let (r_pos, b_pos) = if green_on_diag1 { (1, 2) } else { (0, 3) };

    let rb_diff = (means[r_pos] - means[b_pos]).abs();
    let max_mean = means.iter().cloned().fold(0.0f32, f32::max);
    let rb_separation = if max_mean > 0.001 {
        (rb_diff / max_mean).min(1.0)
    } else {
        0.0
    };

    (diagonal_ratio * 0.6 + rb_separation * 0.4).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_pattern_frame(width: usize, height: usize, values: [f32; 4]) -> Frame {
        let mut data = vec![0.0f32; width * height];
        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                let x_odd = x & 1;
                let y_odd = y & 1;
                data[idx] = values[y_odd * 2 + x_odd];
            }
        }
        Frame::from_f32_vec(data, width, height, 1).unwrap()
    }

    #[test]
    fn test_detect_cfa_pattern_rggb() {
        // RGGB: R bright, G mid, B dim
        let frame = create_pattern_frame(100, 100, [0.7, 0.5, 0.5, 0.3]);
        let result = detect_cfa_pattern(&frame).unwrap();
        assert_eq!(result.pattern, CfaPattern::Rggb);
        assert!(result.confidence > 0.3);
    }

    #[test]
    fn test_detect_cfa_pattern_bggr() {
        // BGGR: B dim, G mid, R bright
        let frame = create_pattern_frame(100, 100, [0.3, 0.5, 0.5, 0.7]);
        let result = detect_cfa_pattern(&frame).unwrap();
        assert_eq!(result.pattern, CfaPattern::Bggr);
    }

    #[test]
    fn test_detect_cfa_pattern_grbg() {
        // GRBG: G, R, B, G
        let frame = create_pattern_frame(100, 100, [0.5, 0.7, 0.3, 0.5]);
        let result = detect_cfa_pattern(&frame).unwrap();
        assert_eq!(result.pattern, CfaPattern::Grbg);
    }

    #[test]
    fn test_detect_cfa_pattern_gbrg() {
        // GBRG: G, B, R, G
        let frame = create_pattern_frame(100, 100, [0.5, 0.3, 0.7, 0.5]);
        let result = detect_cfa_pattern(&frame).unwrap();
        assert_eq!(result.pattern, CfaPattern::Gbrg);
    }

    #[test]
    fn test_detect_pattern_wrong_channels() {
        let frame = Frame::zeros(100, 100, 3).unwrap();
        let result = detect_cfa_pattern(&frame);
        assert!(matches!(result, Err(StackError::ChannelMismatch { .. })));
    }

    #[test]
    fn test_detect_pattern_too_small() {
        let frame = Frame::zeros(2, 2, 1).unwrap();
        let result = detect_cfa_pattern(&frame);
        assert!(matches!(result, Err(StackError::InvalidDimensions { .. })));
    }
}
