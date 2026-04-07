//! Integration tests for background subtraction behavior.
//!
//! These tests specifically evaluate how background subtraction affects
//! extended objects like nebulae, ensuring the algorithm doesn't remove
//! legitimate signal along with the background.

use night_amplifier::{
    auto_stretch_default, compute_image_stats, debayer_auto, BackgroundConfig, BackgroundExtractor,
    DetectionConfig, Frame, ImageRegistration, RejectionMethod, Stacker, StackingConfig,
    StarDetector,
};
use serial_test::serial;
use std::path::Path;

use crate::integration::common::{
    find_image_files_in_dir, prepare_test_output_dir, MIN_STARS_FOR_REGISTRATION,
};
use crate::integration::image_loading::{load_images_from_paths, save_processed_frame_to_dir};

/// Test output directory for background tests
const BACKGROUND_OUTPUT_DIR: &str = "background";

/// Specific fixture set for nebula testing
const NEBULA_FIXTURE_PATH: &str = "tests/fixtures/20260224_204458";

/// Maximum number of frames to use for faster testing
const MAX_TEST_FRAMES: usize = 20;

/// Statistics about background subtraction impact
#[derive(Debug)]
#[allow(dead_code)]
struct BackgroundSubtractionStats {
    /// Mean of channel medians before background subtraction
    pre_bg_mean_median: f32,
    /// Mean of channel medians after background subtraction
    post_bg_mean_median: f32,
    /// Ratio of post/pre median (signal retention)
    signal_retention_ratio: f32,
    /// Standard deviation of pixel values before
    pre_bg_std_dev: f32,
    /// Standard deviation of pixel values after
    post_bg_std_dev: f32,
    /// Contrast enhancement ratio (post_std / pre_std normalized by median)
    contrast_ratio: f32,
}

impl BackgroundSubtractionStats {
    fn compute(pre_frame: &Frame, post_frame: &Frame) -> Result<Self, String> {
        let pre_stats = compute_image_stats(pre_frame)
            .map_err(|e| format!("Failed to compute pre-bg stats: {}", e))?;
        let post_stats = compute_image_stats(post_frame)
            .map_err(|e| format!("Failed to compute post-bg stats: {}", e))?;

        let pre_median = pre_stats.mean_median();
        let post_median = post_stats.mean_median();

        // Compute standard deviation for contrast measurement
        let pre_std = compute_std_dev(pre_frame);
        let post_std = compute_std_dev(post_frame);

        let signal_retention = if pre_median > 1e-6 {
            post_median / pre_median
        } else {
            0.0
        };

        // Contrast ratio: how much the relative variation increased
        let pre_relative_std = if pre_median > 1e-6 {
            pre_std / pre_median
        } else {
            0.0
        };
        let post_relative_std = if post_median > 1e-6 {
            post_std / post_median
        } else {
            0.0
        };
        let contrast_ratio = if pre_relative_std > 1e-6 {
            post_relative_std / pre_relative_std
        } else {
            1.0
        };

        Ok(Self {
            pre_bg_mean_median: pre_median,
            post_bg_mean_median: post_median,
            signal_retention_ratio: signal_retention,
            pre_bg_std_dev: pre_std,
            post_bg_std_dev: post_std,
            contrast_ratio,
        })
    }
}

/// Compute standard deviation of pixel values
fn compute_std_dev(frame: &Frame) -> f32 {
    let data = frame.data();
    if data.is_empty() {
        return 0.0;
    }

    let mean: f32 = data.iter().sum::<f32>() / data.len() as f32;
    let variance: f32 = data.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / data.len() as f32;
    variance.sqrt()
}

/// Process frames through the stacking pipeline up to (but not including) background subtraction
fn stack_frames_without_background(fixture_path: &str) -> Result<Frame, String> {
    let path = Path::new(fixture_path);
    if !path.exists() {
        return Err(format!("Fixture path does not exist: {}", fixture_path));
    }

    let mut files = find_image_files_in_dir(path);
    if files.is_empty() {
        return Err("No image files found in fixture directory".to_string());
    }

    // Limit frames for faster testing
    files.truncate(MAX_TEST_FRAMES);
    println!("  Using {} frames for testing", files.len());

    let images = load_images_from_paths(&files);
    if images.len() < 2 {
        return Err(format!("Need at least 2 images, got {}", images.len()));
    }

    let (ref_width, ref_height, ref_channels) = (
        images[0].width,
        images[0].height,
        images[0].frame.channels(),
    );

    // Star detection
    let detection_config = DetectionConfig::default()
        .with_sigma(5.0)
        .with_max_stars(200);
    let detector = StarDetector::new(detection_config);

    let ref_stars = detector
        .detect(&images[0].frame)
        .map_err(|e| format!("Failed to detect stars: {}", e))?;

    if ref_stars.len() < MIN_STARS_FOR_REGISTRATION {
        return Err(format!(
            "Too few stars detected ({}) for registration",
            ref_stars.len()
        ));
    }

    // Initialize stacker
    let stacking_config = StackingConfig::default()
        .with_rejection(RejectionMethod::SigmaClip)
        .with_sigma(2.5);

    let mut stacker = Stacker::new(ref_width, ref_height, ref_channels, stacking_config)
        .map_err(|e| format!("Failed to create Stacker: {}", e))?;

    stacker
        .add_reference(&images[0].frame)
        .map_err(|e| format!("Failed to add reference frame: {}", e))?;

    // Register and stack
    let registration = ImageRegistration::with_defaults();

    for img in images[1..].iter() {
        let target_stars = match detector.detect(&img.frame) {
            Ok(stars) => stars,
            Err(_) => continue,
        };

        if let Ok(transform) = registration.register(&ref_stars, &target_stars) {
            let _ = stacker.add_frame(&img.frame, &transform);
        }
    }

    println!("  Stacked {} frames", stacker.frame_count());

    // Compute stacked result
    let stacked_raw = stacker
        .compute()
        .map_err(|e| format!("Failed to compute stack: {}", e))?;

    // Debayer if needed
    let stacked = if stacked_raw.channels() == 1 {
        let (debayered, pattern) =
            debayer_auto(&stacked_raw).map_err(|e| format!("Failed to debayer: {}", e))?;
        println!("  Debayered with pattern: {:?}", pattern.pattern);
        debayered
    } else {
        stacked_raw
    };

    Ok(stacked)
}

/// Test background subtraction on nebula data and evaluate signal preservation.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_background_subtraction_preserves_nebula_signal() {
    println!("\n=== Background Subtraction Signal Preservation Test ===\n");

    // Prepare output directory
    let output_dir =
        prepare_test_output_dir(BACKGROUND_OUTPUT_DIR).expect("Failed to prepare output directory");
    println!("Output directory: {:?}\n", output_dir);

    // Stack frames
    println!("Phase 1: Stacking frames...");
    let stacked = match stack_frames_without_background(NEBULA_FIXTURE_PATH) {
        Ok(f) => f,
        Err(e) => {
            println!("Skipping test: {}", e);
            return;
        }
    };

    // Get pre-background stats
    let pre_stats = compute_image_stats(&stacked).expect("Failed to compute pre-bg stats");
    println!("\nPhase 2: Pre-background statistics:");
    println!("  Mean median: {:.6}", pre_stats.mean_median());
    if let (Some(r), Some(g), Some(b)) = (
        pre_stats.channel(0),
        pre_stats.channel(1),
        pre_stats.channel(2),
    ) {
        println!(
            "  Per-channel medians: R={:.6}, G={:.6}, B={:.6}",
            r.median, g.median, b.median
        );
    }

    // Save pre-background image
    let mut pre_bg_stretched = stacked.clone();
    let _ = auto_stretch_default(&mut pre_bg_stretched);
    save_processed_frame_to_dir(&pre_bg_stretched, &output_dir, "01_pre_background")
        .expect("Failed to save pre-bg image");

    // Test different background subtraction configurations
    println!("\nPhase 3: Testing background subtraction configurations...\n");

    let configs = [
        (
            "gradient_only_default",
            BackgroundConfig::default(), // gradient_only: true, aggressiveness: 1.0
        ),
        (
            "for_nebulae",
            BackgroundConfig::for_nebulae(), // Conservative preset for nebulae
        ),
        (
            "adaptive",
            BackgroundConfig::adaptive(), // Auto-detect aggressiveness
        ),
        (
            "gradient_only_coarse",
            BackgroundConfig::default().with_grid_size(8, 8),
        ),
        (
            "low_aggressiveness",
            BackgroundConfig::default().with_aggressiveness(0.3),
        ),
        (
            "full_subtraction",
            BackgroundConfig::default().with_gradient_only(false),
        ),
    ];

    let mut results: Vec<(&str, BackgroundSubtractionStats)> = Vec::new();

    for (name, config) in &configs {
        println!("--- Testing config: {} ---", name);
        println!("  Grid: {}x{}", config.grid_width, config.grid_height);
        println!("  Gradient only: {}", config.gradient_only);
        println!(
            "  Reference percentile: {:.0}%",
            config.reference_percentile * 100.0
        );
        println!(
            "  Aggressiveness: {}",
            if config.aggressiveness < 0.0 {
                "auto".to_string()
            } else {
                format!("{:.0}%", config.aggressiveness * 100.0)
            }
        );

        let mut test_frame = stacked.clone();
        let extractor = BackgroundExtractor::new(config.clone());

        if let Err(e) = extractor.subtract(&mut test_frame) {
            println!("  ERROR: {}", e);
            continue;
        }

        let stats = match BackgroundSubtractionStats::compute(&stacked, &test_frame) {
            Ok(s) => s,
            Err(e) => {
                println!("  ERROR computing stats: {}", e);
                continue;
            }
        };

        println!("  Post-bg median: {:.6}", stats.post_bg_mean_median);
        println!(
            "  Signal retention: {:.1}%",
            stats.signal_retention_ratio * 100.0
        );
        println!("  Contrast ratio: {:.2}x", stats.contrast_ratio);

        // Auto-stretch and save
        let mut stretched = test_frame.clone();
        if auto_stretch_default(&mut stretched).is_ok() {
            let output_name = format!("02_bg_{}", name);
            let _ = save_processed_frame_to_dir(&stretched, &output_dir, &output_name);
        }

        results.push((name, stats));
        println!();
    }

    // Validation
    println!("=== Validation Results ===\n");

    for (name, stats) in &results {
        println!("Config: {}", name);

        // Check signal retention - gradient_only should retain most signal
        if name.starts_with("gradient_only") {
            assert!(
                stats.signal_retention_ratio > 0.5,
                "{}: Signal retention too low ({:.1}%), expected >50% for gradient-only mode",
                name,
                stats.signal_retention_ratio * 100.0
            );
            println!(
                "  [PASS] Signal retention: {:.1}% (>50%)",
                stats.signal_retention_ratio * 100.0
            );
        }

        // Check that we're not destroying all signal
        assert!(
            stats.post_bg_mean_median > 1e-6,
            "{}: Post-background median is effectively zero ({:.9})",
            name,
            stats.post_bg_mean_median
        );
        println!(
            "  [PASS] Post-bg median: {:.6} (>0)",
            stats.post_bg_mean_median
        );

        // Check contrast improvement (should generally improve or stay similar)
        if stats.contrast_ratio < 0.5 {
            println!(
                "  [WARN] Contrast degraded significantly: {:.2}x",
                stats.contrast_ratio
            );
        } else {
            println!("  [PASS] Contrast ratio: {:.2}x", stats.contrast_ratio);
        }

        println!();
    }

    println!("=== Test Complete ===");
    println!("Check output images in: {:?}", output_dir);
}

/// Test that compares background model to image variance to detect extended objects.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_background_model_analysis() {
    println!("\n=== Background Model Analysis Test ===\n");

    // Stack frames
    println!("Phase 1: Stacking frames...");
    let stacked = match stack_frames_without_background(NEBULA_FIXTURE_PATH) {
        Ok(f) => f,
        Err(e) => {
            println!("Skipping test: {}", e);
            return;
        }
    };

    println!("\nPhase 2: Analyzing background model...\n");

    let config = BackgroundConfig::default().with_grid_size(16, 16);
    let extractor = BackgroundExtractor::new(config);

    let model = extractor
        .estimate(&stacked)
        .expect("Failed to estimate background");

    // Analyze grid values for each channel
    for c in 0..stacked.channels() {
        let grid_values = model.grid_values(c);
        let min = grid_values.iter().cloned().fold(f32::MAX, f32::min);
        let max = grid_values.iter().cloned().fold(f32::MIN, f32::max);
        let mean: f32 = grid_values.iter().sum::<f32>() / grid_values.len() as f32;
        let variance: f32 =
            grid_values.iter().map(|&v| (v - mean).powi(2)).sum::<f32>() / grid_values.len() as f32;
        let std_dev = variance.sqrt();

        let channel_name = match c {
            0 => "Red",
            1 => "Green",
            2 => "Blue",
            _ => "Unknown",
        };

        println!("{} channel background grid:", channel_name);
        println!("  Min: {:.6}, Max: {:.6}", min, max);
        println!("  Mean: {:.6}, StdDev: {:.6}", mean, std_dev);
        println!(
            "  Range: {:.6} ({:.1}% of mean)",
            max - min,
            (max - min) / mean * 100.0
        );
        println!("  Coefficient of variation: {:.1}%", std_dev / mean * 100.0);
        println!();
    }

    // The background model should have relatively low variation for good images
    // High variation might indicate nebulae or other extended objects
    let green_grid = model.grid_values(1);
    let mean: f32 = green_grid.iter().sum::<f32>() / green_grid.len() as f32;
    let min = green_grid.iter().cloned().fold(f32::MAX, f32::min);
    let max = green_grid.iter().cloned().fold(f32::MIN, f32::max);
    let range_ratio = (max - min) / mean;

    println!("Background uniformity assessment:");
    if range_ratio < 0.1 {
        println!(
            "  Background is very uniform (range {:.1}% of mean)",
            range_ratio * 100.0
        );
        println!("  -> Full background subtraction may be safe");
    } else if range_ratio < 0.3 {
        println!(
            "  Background has moderate gradient (range {:.1}% of mean)",
            range_ratio * 100.0
        );
        println!("  -> Gradient-only subtraction recommended");
    } else {
        println!(
            "  Background is highly non-uniform (range {:.1}% of mean)",
            range_ratio * 100.0
        );
        println!("  -> May contain extended objects, use caution with background subtraction");
    }
}
