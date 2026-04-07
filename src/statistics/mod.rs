//! Robust Image Statistics for Astrophotography Autostretch
//!
//! This module provides high-performance computation of robust statistics
//! (median and MAD) optimized for astronomical images on embedded platforms.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

mod channel;
mod compute;
mod config;
mod image;
mod ops;

pub use channel::ChannelStats;
pub use config::StatsConfig;
pub use image::ImageStats;
pub use ops::fast_median;

use compute::compute_channel_stats;
use ops::{compute_mad_in_place_simd, min_max_simd};

/// Compute robust image statistics (median and MAD) per channel
pub fn compute_image_stats(frame: &Frame) -> Result<ImageStats> {
    compute_image_stats_with_config(frame, StatsConfig::default())
}

/// Compute image statistics with custom configuration
pub fn compute_image_stats_with_config(frame: &Frame, config: StatsConfig) -> Result<ImageStats> {
    let width = frame.width();
    let height = frame.height();
    let channels = frame.channels();
    let total_pixels = width * height;

    if total_pixels < config.min_samples {
        return Err(StackError::InvalidConfiguration(format!(
            "Image too small for statistics: {} pixels, need at least {}",
            total_pixels, config.min_samples
        )));
    }

    // Determine sample count and step
    let sample_count = total_pixels.min(config.max_samples);
    let step = if sample_count >= total_pixels {
        1
    } else {
        total_pixels / sample_count
    };

    // Compute statistics for each channel in parallel
    let channel_stats: Vec<ChannelStats> = (0..channels)
        .into_par_iter()
        .map(|channel| compute_channel_stats(frame, channel, step))
        .collect();

    Ok(ImageStats {
        channels: channel_stats,
        sample_count,
    })
}

/// Compute statistics for a luminance image (single-pass for monochrome)
pub fn compute_luminance_stats(data: &[f32]) -> Result<ChannelStats> {
    if data.len() < 1000 {
        return Err(StackError::InvalidConfiguration(
            "Data too small for statistics".into(),
        ));
    }

    let config = StatsConfig::default();
    let step = data.len() / config.max_samples.min(data.len());
    let step = step.max(1);

    // For step=1, use contiguous access
    let mut samples = if step == 1 {
        data.to_vec()
    } else {
        // Batch sampling for better cache efficiency
        let estimated_size = data.len() / step + 1;
        let mut samples = Vec::with_capacity(estimated_size);

        // Collect samples in batches
        let batch_size = 256;
        let indices: Vec<usize> = (0..data.len()).step_by(step).collect();

        for batch in indices.chunks(batch_size) {
            for &idx in batch {
                samples.push(data[idx]);
            }
        }
        samples
    };

    if samples.is_empty() {
        return Ok(ChannelStats::new(0.0, 0.0, 0.0, 0.0));
    }

    // Use SIMD for min/max
    let (min_val, max_val) = min_max_simd(&samples);

    // Compute median
    let median = fast_median(&mut samples);

    // Compute MAD in-place using SIMD
    compute_mad_in_place_simd(&mut samples, median);
    let mad = fast_median(&mut samples);

    Ok(ChannelStats::new(median, mad, min_val, max_val))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_image_stats() {
        let frame = Frame::filled(256, 256, 3, 0.3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        assert_eq!(stats.channels.len(), 3);
        for ch in &stats.channels {
            assert!(
                (ch.median - 0.3).abs() < 0.01,
                "Median should be ~0.3, got {}",
                ch.median
            );
            assert!(ch.mad < 0.01, "MAD should be ~0, got {}", ch.mad);
            assert!(ch.sigma < 0.02, "Sigma should be ~0, got {}", ch.sigma);
        }
    }

    #[test]
    fn test_noisy_image_stats() {
        let mut data = vec![0.5f32; 256 * 256];
        let mut seed: u32 = 12345;
        for v in data.iter_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            let noise = ((seed >> 16) as f32 / 65536.0 - 0.5) * 0.1;
            *v += noise;
        }

        let frame = Frame::from_f32_vec(data, 256, 256, 1).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        let ch = &stats.channels[0];
        assert!((ch.median - 0.5).abs() < 0.02);
        assert!(ch.mad > 0.01);
        assert!(ch.mad < 0.1);
    }

    #[test]
    fn test_outlier_robustness() {
        let mut data = vec![0.1f32; 100 * 100];
        for i in 0..100 {
            data[i * 100] = 1.0;
        }

        let frame = Frame::from_f32_vec(data, 100, 100, 1).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        let ch = &stats.channels[0];
        assert!((ch.median - 0.1).abs() < 0.02);
        assert!(ch.mad < 0.05);
    }

    #[test]
    fn test_multichannel_independence() {
        let mut data = vec![0.0f32; 64 * 64 * 3];
        for i in 0..(64 * 64) {
            data[i * 3] = 0.2;
            data[i * 3 + 1] = 0.4;
            data[i * 3 + 2] = 0.6;
        }

        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        assert!((stats.channels[0].median - 0.2).abs() < 0.01);
        assert!((stats.channels[1].median - 0.4).abs() < 0.01);
        assert!((stats.channels[2].median - 0.6).abs() < 0.01);
    }

    #[test]
    fn test_min_max_tracking() {
        let mut data = vec![0.5f32; 100 * 100];
        data[0] = 0.1;
        data[9999] = 0.9;

        let frame = Frame::from_f32_vec(data, 100, 100, 1).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        let ch = &stats.channels[0];
        assert!((ch.min - 0.1).abs() < 0.01);
        assert!((ch.max - 0.9).abs() < 0.01);
    }

    #[test]
    fn test_mean_statistics() {
        let mut data = vec![0.0f32; 64 * 64 * 3];
        for i in 0..(64 * 64) {
            data[i * 3] = 0.3;
            data[i * 3 + 1] = 0.3;
            data[i * 3 + 2] = 0.3;
        }

        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();

        assert!((stats.mean_median() - 0.3).abs() < 0.01);
        assert!(stats.mean_sigma() < 0.02);
    }

    #[test]
    fn test_suggested_black_point() {
        let ch = ChannelStats::new(0.2, 0.01, 0.0, 1.0);
        let bp = ch.suggested_black_point(2.8);
        let expected = 0.2 - 2.8 * (0.01 * 1.4826);
        assert!((bp - expected).abs() < 0.001);
    }

    #[test]
    fn test_black_point_clamp() {
        let ch = ChannelStats::new(0.05, 0.1, 0.0, 1.0);
        let bp = ch.suggested_black_point(5.0);
        assert_eq!(bp, 0.0);
    }

    #[test]
    fn test_fast_median_odd() {
        let mut values = vec![1.0, 5.0, 3.0, 2.0, 4.0];
        let med = fast_median(&mut values);
        assert!((med - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_fast_median_even() {
        let mut values = vec![1.0, 5.0, 3.0, 2.0, 4.0, 6.0];
        let med = fast_median(&mut values);
        assert!((med - 3.5).abs() < 1e-6);
    }

    #[test]
    fn test_fast_median_single() {
        let mut values = vec![42.0];
        let med = fast_median(&mut values);
        assert!((med - 42.0).abs() < 1e-6);
    }

    #[test]
    fn test_fast_median_empty() {
        let mut values: Vec<f32> = vec![];
        let med = fast_median(&mut values);
        assert_eq!(med, 0.0);
    }

    #[test]
    fn test_luminance_stats() {
        let data = vec![0.25f32; 10000];
        let stats = compute_luminance_stats(&data).unwrap();
        assert!((stats.median - 0.25).abs() < 0.01);
        assert!(stats.mad < 0.01);
    }

    #[test]
    fn test_is_low_signal() {
        let mut high_signal_data = vec![0.1f32; 64 * 64];
        high_signal_data[0] = 0.9;
        let high_signal = Frame::from_f32_vec(high_signal_data, 64, 64, 1).unwrap();
        let stats = compute_image_stats(&high_signal).unwrap();
        assert!(!stats.is_low_signal());
    }

    #[test]
    fn test_config_full_precision() {
        let config = StatsConfig::default().full_precision();
        assert_eq!(config.max_samples, usize::MAX);
    }

    #[test]
    fn test_signal_range() {
        let ch = ChannelStats::new(0.2, 0.01, 0.0, 0.8);
        assert!((ch.signal_range() - 0.6).abs() < 1e-6);
    }

    #[test]
    fn test_global_min_max() {
        let mut data = vec![0.0f32; 64 * 64 * 3];
        for i in 0..(64 * 64) {
            data[i * 3] = 0.1;
            data[i * 3 + 1] = 0.5;
            data[i * 3 + 2] = 0.9;
        }
        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let stats = compute_image_stats(&frame).unwrap();
        assert!((stats.global_min() - 0.1).abs() < 0.01);
        assert!((stats.global_max() - 0.9).abs() < 0.01);
    }
}
