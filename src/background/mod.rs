//! Background extraction and light pollution subtraction
//!
//! This module estimates and removes the sky background gradient caused by
//! light pollution. The algorithm:
//! 1. Overlays a boundary-hugging grid of sample nodes on the image
//! 2. Extracts a star-free background estimate per node via iterative sigma clipping
//! 3. Prunes nodes that landed on nebulosity (global + local neighbor rejection)
//! 4. Inpaints rejected nodes via iterative 4-connected averaging
//! 5. Interpolates a smooth 2D background surface using bilinear delta-stepping
//! 6. Subtracts the background from the image

mod config;
mod extractor;
mod model;

pub use config::{BackgroundConfig, BackgroundExtractionAlgorithm};
pub use extractor::BackgroundExtractor;
pub use model::BackgroundModel;

use crate::error::Result;
use crate::frame::Frame;
use std::sync::OnceLock;

/// Plugin trait for advanced background algorithms (implemented in Pro version)
pub trait BackgroundAlgorithmPlugin: Send + Sync {
    fn estimate_rbf(&self, frame: &Frame, config: &BackgroundConfig) -> Result<BackgroundModel>;
}

/// Global registry for the background plugin
pub static BACKGROUND_PLUGIN: OnceLock<Box<dyn BackgroundAlgorithmPlugin>> = OnceLock::new();

/// Convenience function for background subtraction
pub fn subtract_background(frame: &mut Frame) -> Result<()> {
    BackgroundExtractor::with_defaults().subtract(frame)
}

/// Convenience function for background subtraction with custom config
pub fn subtract_background_with_config(frame: &mut Frame, config: BackgroundConfig) -> Result<()> {
    BackgroundExtractor::new(config).subtract(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_background() {
        // Create a uniform frame - background should be uniform
        let frame = Frame::filled(256, 256, 3, 0.3).unwrap();
        let extractor = BackgroundExtractor::with_defaults();
        let model = extractor.estimate(&frame).unwrap();

        // Check that all grid values are close to 0.3
        for c in 0..3 {
            for v in model.grid_values(c) {
                assert!((v - 0.3).abs() < 0.01, "Grid value {} differs from 0.3", v);
            }
        }
    }

    #[test]
    fn test_gradient_background() {
        // Create a frame with a horizontal gradient
        let mut frame = Frame::zeros(256, 256, 1).unwrap();
        for y in 0..256 {
            for x in 0..256 {
                let value = x as f32 / 255.0 * 0.5; // 0.0 to 0.5 gradient
                frame.set_pixel(x, y, 0, value);
            }
        }

        let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(8, 8));
        let model = extractor.estimate(&frame).unwrap();

        // Left side should be lower than right side
        let left_val = model.get_background(0, 128, 0);
        let right_val = model.get_background(255, 128, 0);
        assert!(left_val < right_val, "Gradient not detected");
        assert!(left_val < 0.1, "Left side should be near 0");
        assert!(right_val > 0.4, "Right side should be near 0.5");
    }

    #[test]
    fn test_star_rejection() {
        // Create a uniform background with a bright "star"
        let mut frame = Frame::filled(64, 64, 1, 0.2).unwrap();

        // Add a bright star in the center
        frame.set_pixel(32, 32, 0, 1.0);
        frame.set_pixel(31, 32, 0, 0.8);
        frame.set_pixel(33, 32, 0, 0.8);
        frame.set_pixel(32, 31, 0, 0.8);
        frame.set_pixel(32, 33, 0, 0.8);

        let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(4, 4));
        let model = extractor.estimate(&frame).unwrap();

        // The background should still be close to 0.2 despite the star
        let center_bg = model.get_background(32, 32, 0);
        assert!(
            (center_bg - 0.2).abs() < 0.05,
            "Star not rejected, background is {}",
            center_bg
        );
    }

    #[test]
    fn test_background_subtraction_full() {
        // Test full background subtraction mode (gradient_only: false)
        let mut frame = Frame::filled(128, 128, 1, 0.4).unwrap();

        // Add a "star" at 0.9
        frame.set_pixel(64, 64, 0, 0.9);

        let config = BackgroundConfig::default().with_gradient_only(false);
        let extractor = BackgroundExtractor::new(config);
        extractor.subtract(&mut frame).unwrap();

        // Background should be near 0 after full subtraction
        let corner_val = frame.get_pixel(0, 0, 0);
        assert!(
            corner_val < 0.05,
            "Background not removed, got {}",
            corner_val
        );

        // Star should still be visible (above background)
        let star_val = frame.get_pixel(64, 64, 0);
        assert!(star_val > 0.3, "Star was removed, got {}", star_val);
    }

    #[test]
    fn test_background_subtraction_gradient_only() {
        // Test gradient-only mode (default): uniform background should be preserved
        let mut frame = Frame::filled(128, 128, 1, 0.4).unwrap();

        // Add a "star" at 0.9
        frame.set_pixel(64, 64, 0, 0.9);

        let extractor = BackgroundExtractor::with_defaults();
        extractor.subtract(&mut frame).unwrap();

        // With uniform background and gradient_only=true, background should be preserved
        let corner_val = frame.get_pixel(0, 0, 0);
        assert!(
            (corner_val - 0.4).abs() < 0.05,
            "Uniform background should be preserved in gradient-only mode, got {}",
            corner_val
        );

        // Star should still be at 0.9
        let star_val = frame.get_pixel(64, 64, 0);
        assert!(
            (star_val - 0.9).abs() < 0.05,
            "Star should be preserved, got {}",
            star_val
        );
    }

    #[test]
    fn test_gradient_subtraction() {
        // Test that gradient-only mode removes gradients while preserving base level
        let mut frame = Frame::zeros(256, 256, 1).unwrap();
        for y in 0..256 {
            for x in 0..256 {
                // Gradient from 0.1 to 0.6
                let value = 0.1 + x as f32 / 255.0 * 0.5;
                frame.set_pixel(x, y, 0, value);
            }
        }

        let extractor = BackgroundExtractor::new(BackgroundConfig::default().with_grid_size(8, 8));
        extractor.subtract(&mut frame).unwrap();

        // After gradient-only subtraction, the image should be relatively uniform
        // at approximately the minimum background level (0.1)
        let left_val = frame.get_pixel(10, 128, 0);
        let right_val = frame.get_pixel(245, 128, 0);

        // Values should be close to each other (gradient removed)
        assert!(
            (left_val - right_val).abs() < 0.1,
            "Gradient not removed: left={}, right={}",
            left_val,
            right_val
        );

        // Values should be near the original minimum (~0.1)
        assert!(left_val > 0.05, "Signal level too low: {}", left_val);
    }

    #[test]
    fn test_no_negative_values() {
        // Create a frame with low values
        let mut frame = Frame::filled(64, 64, 1, 0.1).unwrap();

        // Set some pixels even lower
        frame.set_pixel(32, 32, 0, 0.05);

        let extractor = BackgroundExtractor::with_defaults();
        extractor.subtract(&mut frame).unwrap();

        // Ensure no negative values
        for &v in frame.data() {
            assert!(v >= 0.0, "Negative value found: {}", v);
        }
    }

    #[test]
    fn test_bilinear_interpolation() {
        // Create a simple 2x2 grid model and check interpolation
        let model = BackgroundModel::new(
            vec![vec![0.0, 1.0, 0.0, 1.0]], // 2x2 grid: TL=0, TR=1, BL=0, BR=1
            2,
            2,
            100,
            100,
            1,
            true,
            0.1,
            1.0,
        );

        // Center should be average of all corners
        let center = model.get_background(50, 50, 0);
        assert!(
            (center - 0.5).abs() < 0.1,
            "Center interpolation wrong: {}",
            center
        );

        // Far left should be low
        let left = model.get_background(0, 50, 0);
        assert!(left < 0.3, "Left interpolation wrong: {}", left);

        // Far right should be high
        let right = model.get_background(99, 50, 0);
        assert!(right > 0.7, "Right interpolation wrong: {}", right);
    }

    #[test]
    fn test_to_frame() {
        let frame = Frame::filled(64, 64, 3, 0.5).unwrap();
        let extractor = BackgroundExtractor::with_defaults();
        let model = extractor.estimate(&frame).unwrap();

        let bg_frame = model.to_frame().unwrap();
        assert_eq!(bg_frame.width(), 64);
        assert_eq!(bg_frame.height(), 64);
        assert_eq!(bg_frame.channels(), 3);
    }

    #[test]
    fn test_multichannel() {
        // Create a frame with different backgrounds per channel
        let mut data = vec![0.0f32; 64 * 64 * 3];
        for y in 0..64 {
            for x in 0..64 {
                let idx = (y * 64 + x) * 3;
                data[idx] = 0.1; // R
                data[idx + 1] = 0.3; // G
                data[idx + 2] = 0.5; // B
            }
        }
        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();

        let extractor = BackgroundExtractor::with_defaults();
        let model = extractor.estimate(&frame).unwrap();

        let r = model.get_background(32, 32, 0);
        let g = model.get_background(32, 32, 1);
        let b = model.get_background(32, 32, 2);

        assert!((r - 0.1).abs() < 0.02, "R channel wrong: {}", r);
        assert!((g - 0.3).abs() < 0.02, "G channel wrong: {}", g);
        assert!((b - 0.5).abs() < 0.02, "B channel wrong: {}", b);
    }

    #[test]
    fn test_convenience_function() {
        // Create a frame with a gradient
        let mut frame = Frame::zeros(64, 64, 1).unwrap();
        for y in 0..64 {
            for x in 0..64 {
                let value = 0.1 + x as f32 / 63.0 * 0.3; // 0.1 to 0.4 gradient
                frame.set_pixel(x, y, 0, value);
            }
        }

        subtract_background(&mut frame).unwrap();

        // After gradient-only subtraction, the gradient should be flattened
        let left_val = frame.get_pixel(5, 32, 0);
        let right_val = frame.get_pixel(58, 32, 0);

        // Values should be close to each other (gradient removed)
        assert!(
            (left_val - right_val).abs() < 0.15,
            "Gradient not removed: left={}, right={}",
            left_val,
            right_val
        );
    }

    #[test]
    fn test_boundary_hugging_grid() {
        // Verify that the grid nodes span the full image boundaries
        let frame = Frame::filled(256, 256, 1, 0.3).unwrap();
        let config = BackgroundConfig::default().with_grid_size(8, 8);
        let extractor = BackgroundExtractor::new(config);
        let model = extractor.estimate(&frame).unwrap();

        // The model should produce valid background values at all corners
        let tl = model.get_background(0, 0, 0);
        let tr = model.get_background(255, 0, 0);
        let bl = model.get_background(0, 255, 0);
        let br = model.get_background(255, 255, 0);

        for (name, val) in [("TL", tl), ("TR", tr), ("BL", bl), ("BR", br)] {
            assert!(
                (val - 0.3).abs() < 0.02,
                "{} corner should be ~0.3, got {}",
                name,
                val
            );
        }
    }

    #[test]
    fn test_nebulosity_pruning_with_inpaint() {
        // Create a frame with uniform background + bright nebula region
        let mut frame = Frame::filled(256, 256, 1, 0.1).unwrap();

        // Paint a large "nebula" in the center (80x80 bright region)
        for y in 88..168 {
            for x in 88..168 {
                frame.set_pixel(x, y, 0, 0.5);
            }
        }

        let config = BackgroundConfig::default().with_grid_size(12, 12);
        let extractor = BackgroundExtractor::new(config);
        let model = extractor.estimate(&frame).unwrap();

        // Background at the corner should be close to 0.1
        let corner = model.get_background(5, 5, 0);
        assert!(
            (corner - 0.1).abs() < 0.05,
            "Corner should be ~0.1, got {}",
            corner
        );

        // Background in the nebula region should also be close to 0.1
        // because nebula nodes were pruned and inpainted from neighbors
        let center = model.get_background(128, 128, 0);
        assert!(
            (center - 0.1).abs() < 0.1,
            "Center should be ~0.1 (nebula pruned + inpainted), got {}",
            center
        );
    }

    #[test]
    fn test_flat_field_fallback() {
        // Create a frame that is mostly bright (simulating a tightly cropped nebula)
        let frame = Frame::filled(64, 64, 1, 0.8).unwrap();

        // With a uniform bright frame, most/all nodes will have similar high values.
        // The pruning should not crash even if aggressive pruning triggers the fallback.
        let config = BackgroundConfig::default().with_grid_size(4, 4);
        let extractor = BackgroundExtractor::new(config);
        let result = extractor.estimate(&frame);

        assert!(result.is_ok(), "Should not crash on all-bright frame");

        let model = result.unwrap();
        // The model should produce some background value (either original or fallback)
        let bg = model.get_background(32, 32, 0);
        assert!(bg > 0.0, "Background should be positive, got {}", bg);
    }

    #[test]
    fn test_no_double_processing_at_boundaries() {
        // Create a uniform frame and verify delta-stepping doesn't create grid-line artifacts
        let mut frame = Frame::filled(128, 128, 1, 0.5).unwrap();

        let config = BackgroundConfig::default()
            .with_grid_size(8, 8)
            .with_gradient_only(false);
        let extractor = BackgroundExtractor::new(config);
        extractor.subtract(&mut frame).unwrap();

        // After subtracting a uniform background, all pixels should be ~0.0
        // If boundaries were processed twice, those pixels would be negative (clamped to 0)
        // while interior pixels would be slightly positive — creating visible lines.
        let data = frame.data();
        let max_val = data.iter().cloned().fold(0.0f32, f32::max);
        let min_val = data.iter().cloned().fold(f32::MAX, f32::min);

        // All values should be very close to each other (uniform after uniform subtraction)
        assert!(
            max_val - min_val < 0.02,
            "Non-uniform result suggests double-processing: min={}, max={}",
            min_val,
            max_val
        );
    }
}
