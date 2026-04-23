//! Tests for processing all fixture subdirectories.
//!
//! These are longer-running integration tests that process complete image sets.

use std::io::{self, Write};
use std::path::Path;

use night_amplifier::camera::CaptureConfig;
use night_amplifier::camera::{Camera, SimulatedCamera};
use night_amplifier::{
    auto_stretch_default, compute_image_stats, debayer_auto, detect_cfa_pattern,
    subtract_background, CfaPattern, DebayerAlgorithm, DebayerConfig, DetectionConfig, Frame,
    PipelineConfig, StackingPipeline, StarDetector,
};
use serial_test::serial;

use crate::integration::common::{
    find_fixture_sets, prepare_test_output_dir, FixtureSet, LoadedImage, MAX_STRETCH_FACTOR,
    MIN_ACCEPTABLE_SNR, MIN_FRAMES_FOR_STACKING, MIN_STACKING_SUCCESS_RATE, MIN_STRETCH_FACTOR,
    STACKED_OUTPUT_DIR,
};
use crate::integration::image_loading::{
    load_image, load_images_from_paths, save_processed_frame_to_dir,
};

/// Process all fixture subdirectories and save results
/// This is the main test that processes each fixture set and outputs stacked/stretched images
///
/// TODO: This test can be used as a performance/benchmark test for future algorithm optimizations.
/// Consider adding timing measurements and comparison against baseline performance metrics.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_process_all_fixture_sets() {
    println!("\n=== Processing All Fixture Sets ===\n");

    // Ensure fixtures are downloaded from Google Drive
    crate::integration::common::ensure_fixtures_sync();

    // Prepare dedicated output directory (clears only this test's directory)
    let output_dir = match prepare_test_output_dir(STACKED_OUTPUT_DIR) {
        Ok(dir) => {
            println!("Output directory: {}", dir.display());
            dir
        }
        Err(e) => {
            eprintln!("Warning: Failed to prepare output directory: {}", e);
            return;
        }
    };

    let fixture_sets = find_fixture_sets();

    if fixture_sets.is_empty() {
        println!("No fixture subdirectories found in tests/fixtures.");
        println!("To run this test, create subdirectories with TIFF or FITS files.");
        println!("Skipping test.\n");
        return;
    }

    println!("Found {} fixture set(s) to process.\n", fixture_sets.len());

    let mut processed_count = 0;

    // Refresh fixture sets after downloading
    let fixture_sets = find_fixture_sets();

    for fixture_set in &fixture_sets {
        if process_fixture_set(fixture_set, &output_dir) {
            processed_count += 1;
        }
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "=== Processing Complete: {}/{} sets processed ===\n",
        processed_count,
        fixture_sets.len()
    );

    // VALIDATION: At least one fixture set should be successfully processed
    if !fixture_sets.is_empty() {
        assert!(
            processed_count > 0,
            "VALIDATION FAILED: No fixture sets were successfully processed out of {}. \
             Check star detection, registration, or image quality.",
            fixture_sets.len()
        );

        // Report overall success rate
        let overall_success_rate = processed_count as f64 / fixture_sets.len() as f64;
        println!(
            "Overall fixture set processing rate: {:.1}%",
            overall_success_rate * 100.0
        );
    }
}

/// Process a single fixture set. Returns true if successful.
fn process_fixture_set(fixture_set: &FixtureSet, output_dir: &Path) -> bool {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!(
        "Processing: {} ({} files)",
        fixture_set.name,
        fixture_set.files.len()
    );
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    let _ = io::stdout().flush();

    if fixture_set.files.len() < MIN_FRAMES_FOR_STACKING {
        println!(
            "  Only {} file(s), need at least {} for stacking. Skipping.\n",
            fixture_set.files.len(),
            MIN_FRAMES_FOR_STACKING
        );
        return false;
    }

    // Load images from this fixture set
    let images = load_images_from_paths(&fixture_set.files);

    if images.len() < MIN_FRAMES_FOR_STACKING {
        println!(
            "  Only {} image(s) loaded successfully, need at least {}. Skipping.\n",
            images.len(),
            MIN_FRAMES_FOR_STACKING
        );
        return false;
    }

    // Check if images need debayering
    let needs_debayer = images.iter().any(|img| img.is_bayer);

    // Detect CFA pattern once for all images (if needed)
    let detected_pattern: Option<CfaPattern> = if needs_debayer {
        images
            .iter()
            .filter(|img| img.is_bayer && img.frame.channels() == 1)
            .next()
            .and_then(|img| detect_cfa_pattern(&img.frame).ok())
            .map(|detection| {
                println!("\n  Auto-detected CFA pattern: {:?}", detection.pattern);
                detection.pattern
            })
    } else {
        None
    };

    // Helper to load and debayer a single frame (streaming approach)
    let load_frame = |img: &LoadedImage| -> Option<Frame> {
        if img.is_bayer && img.frame.channels() == 1 {
            let config = detected_pattern
                .map(|p| DebayerConfig::new(p).with_algorithm(DebayerAlgorithm::Bilinear))
                .unwrap_or_else(|| {
                    DebayerConfig::new(CfaPattern::Rggb).with_algorithm(DebayerAlgorithm::Bilinear)
                });
            night_amplifier::debayer_with_config(&img.frame, config).ok()
        } else {
            Some(img.frame.clone())
        }
    };

    // Load and process reference frame first
    let ref_frame = match load_frame(&images[0]) {
        Some(f) => f,
        None => {
            println!("  Failed to load reference frame. Skipping.\n");
            return false;
        }
    };

    let (ref_width, ref_height, ref_channels) =
        (ref_frame.width(), ref_frame.height(), ref_frame.channels());

    let step_offset = if needs_debayer { 1 } else { 0 };
    let total_steps = if needs_debayer { 7 } else { 6 };

    println!(
        "\n  All frames: {}x{} with {} channel(s)\n",
        ref_width, ref_height, ref_channels
    );

    // Initialize StackingPipeline with reference frame
    println!(
        "  [{}/{}] Initializing stacking pipeline...",
        1 + step_offset,
        total_steps
    );

    let pipeline_config = PipelineConfig::fast();

    let mut pipeline = match StackingPipeline::new(&ref_frame, pipeline_config) {
        Ok(p) => p,
        Err(e) => {
            println!("        Failed: {}. Skipping set.\n", e);
            return false;
        }
    };

    let ref_stars = pipeline.reference_stars();
    println!("        Found {} stars in reference", ref_stars.len());

    // Calculate and report average SNR
    let avg_snr: f32 = if ref_stars.is_empty() {
        0.0
    } else {
        ref_stars.iter().map(|s| s.snr).sum::<f32>() / ref_stars.len() as f32
    };
    println!("        Average star SNR: {:.1}", avg_snr);

    if avg_snr < MIN_ACCEPTABLE_SNR {
        println!(
            "        WARNING: Average SNR ({:.1}) is below minimum ({:.1})",
            avg_snr, MIN_ACCEPTABLE_SNR
        );
    }

    // Drop reference frame to free memory
    drop(ref_frame);

    // Process frames using parallel prefetcher + StackingPipeline
    let num_cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    println!(
        "  [{}/{}] Processing {} frames (parallel prefetch, {} CPUs)...",
        2 + step_offset,
        total_steps,
        images.len() - 1,
        num_cpus
    );

    let total_frames = images.len();
    let mut frames_processed = 0;

    // Create production simulated camera
    let mut camera =
        SimulatedCamera::new(fixture_set.path.clone()).expect("Failed to create simulated camera");

    // Use 1 microsecond exposure (per requirements)
    let capture_config = CaptureConfig::default().with_exposure_us(1);

    // Consume the first frame (which we already processed as reference)
    let _ = camera
        .capture(&capture_config)
        .expect("Failed to capture reference frame from simulated camera");

    // Process remainder of frames
    for _ in 0..(images.len() - 1) {
        let frame = match camera.capture(&capture_config) {
            Ok(f) => f,
            Err(e) => {
                println!("        Failed to capture frame: {}", e);
                break;
            }
        };

        // Process through the pipeline
        let _result = pipeline.process_frame(&frame);

        frames_processed += 1;

        // Progress indicator
        if frames_processed % 20 == 0 || frames_processed == images.len() - 1 {
            println!(
                "        Processed {}/{}",
                frames_processed,
                images.len() - 1
            );
        }
    }

    // Get statistics from pipeline
    let stats = pipeline.stats();
    println!(
        "        Successfully stacked {} of {} frames",
        stats.frames_stacked, total_frames
    );

    // VALIDATION: Check stacking success rate
    let stacking_rate = stats.success_rate() as f64 / 100.0;
    println!(
        "        Stacking success rate: {:.1}%",
        stats.success_rate()
    );
    if stacking_rate < MIN_STACKING_SUCCESS_RATE {
        println!(
            "        VALIDATION WARNING: Stacking rate ({:.1}%) below minimum ({:.1}%)",
            stats.success_rate(),
            MIN_STACKING_SUCCESS_RATE * 100.0
        );
    }

    // Compute stacked result
    println!(
        "  [{}/{}] Computing stacked result...",
        3 + step_offset,
        total_steps
    );
    let mut stacked = match pipeline.compute() {
        Ok(f) => f,
        Err(e) => {
            println!("        Failed: {}. Skipping set.\n", e);
            return false;
        }
    };
    println!("        Stack computed");

    // Background subtraction
    println!(
        "  [{}/{}] Subtracting background...",
        4 + step_offset,
        total_steps
    );
    if let Err(e) = subtract_background(&mut stacked) {
        println!("        Warning: Background subtraction failed: {}", e);
    } else {
        println!("        Background subtracted");
    }

    // Statistics
    println!(
        "  [{}/{}] Computing statistics and auto-stretch...",
        5 + step_offset,
        total_steps
    );
    let stats = compute_image_stats(&stacked);
    if let Ok(s) = &stats {
        println!("        Mean median: {:.6}", s.mean_median());
    }

    let stretch_result = auto_stretch_default(&mut stacked);
    match &stretch_result {
        Ok(r) => {
            println!(
                "        Stretch factor: {:.2}, Black point: {:.6}, Converged: {}",
                r.stretch_factor, r.black_point, r.converged
            );

            // VALIDATION: Check stretch factor bounds
            if r.stretch_factor < MIN_STRETCH_FACTOR {
                println!(
                    "        VALIDATION WARNING: Stretch factor ({:.2}) below minimum ({:.2})",
                    r.stretch_factor, MIN_STRETCH_FACTOR
                );
            }
            if r.stretch_factor > MAX_STRETCH_FACTOR {
                println!(
                    "        VALIDATION WARNING: Stretch factor ({:.2}) exceeds maximum ({:.2})",
                    r.stretch_factor, MAX_STRETCH_FACTOR
                );
            }
            if !r.converged {
                println!("        VALIDATION WARNING: Auto-stretch did not converge");
            }
        }
        Err(e) => {
            println!("        VALIDATION FAILED: Auto-stretch failed: {}", e);
        }
    }

    // Save result
    println!(
        "  [{}/{}] Saving processed result...",
        6 + step_offset,
        total_steps
    );
    match save_processed_frame_to_dir(&stacked, output_dir, &fixture_set.name) {
        Ok(output_path) => {
            let abs_path =
                std::fs::canonicalize(&output_path).unwrap_or_else(|_| output_path.clone());
            println!("\n  ✓ Processed file saved to: {}\n", abs_path.display());
            let _ = io::stdout().flush();
            true
        }
        Err(e) => {
            println!("        Failed to save: {}\n", e);
            let _ = io::stdout().flush();
            false
        }
    }
}

/// Diagnostic test to understand why star detection fails on real images
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn diagnose_star_detection() {
    println!("\n=== DIAGNOSTIC: Star Detection Analysis ===\n");

    let fixture_sets = find_fixture_sets();
    if fixture_sets.is_empty() {
        println!("No fixture sets found");
        return;
    }

    for fixture_set in &fixture_sets {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "Fixture: {} ({} files)",
            fixture_set.name,
            fixture_set.files.len()
        );
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

        // Load first image
        let first_file = &fixture_set.files[0];
        let img = match load_image(first_file) {
            Ok(img) => img,
            Err(e) => {
                println!("  Failed to load: {}", e);
                continue;
            }
        };

        println!(
            "  Raw image: {}x{}, {} channels, is_bayer={}",
            img.width,
            img.height,
            img.frame.channels(),
            img.is_bayer
        );

        // Analyze raw image statistics
        let data = img.frame.data();
        let min = data.iter().cloned().fold(f32::MAX, f32::min);
        let max = data.iter().cloned().fold(f32::MIN, f32::max);
        let sum: f32 = data.iter().sum();
        let mean = sum / data.len() as f32;

        let mut sorted: Vec<f32> = data.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];

        let mut deviations: Vec<f32> = sorted.iter().map(|&v| (v - median).abs()).collect();
        deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mad = deviations[deviations.len() / 2];
        let sigma = mad * 1.4826;

        println!("  RAW Statistics:");
        println!(
            "    Min: {:.6}, Max: {:.6}, Range: {:.6}",
            min,
            max,
            max - min
        );
        println!("    Mean: {:.6}, Median: {:.6}", mean, median);
        println!("    MAD: {:.6}, Sigma: {:.6}", mad, sigma);
        println!(
            "    Dynamic range (max/median): {:.2}x",
            max / median.max(0.0001)
        );

        // Try star detection on raw Bayer
        println!("\n  Star detection on RAW Bayer:");
        for sigma_thresh in [2.0f32, 3.0, 5.0, 7.0, 10.0] {
            let config = DetectionConfig::default()
                .with_sigma(sigma_thresh)
                .with_min_snr(2.0);
            let detector = StarDetector::new(config);
            match detector.detect(&img.frame) {
                Ok(stars) => println!("    sigma={:.1}: {} stars", sigma_thresh, stars.len()),
                Err(e) => println!("    sigma={:.1}: error - {}", sigma_thresh, e),
            }
        }

        // Debayer if needed and try again
        if img.is_bayer {
            println!("\n  After debayering:");
            if let Ok((debayered, pattern)) = debayer_auto(&img.frame) {
                println!("    Detected pattern: {:?}", pattern);
                println!(
                    "    Debayered: {}x{}, {} channels",
                    debayered.width(),
                    debayered.height(),
                    debayered.channels()
                );

                let db_data = debayered.data();
                let db_min = db_data.iter().cloned().fold(f32::MAX, f32::min);
                let db_max = db_data.iter().cloned().fold(f32::MIN, f32::max);
                let db_sum: f32 = db_data.iter().sum();
                let db_mean = db_sum / db_data.len() as f32;

                let mut db_sorted: Vec<f32> = db_data.to_vec();
                db_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let db_median = db_sorted[db_sorted.len() / 2];

                let mut db_deviations: Vec<f32> =
                    db_sorted.iter().map(|&v| (v - db_median).abs()).collect();
                db_deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
                let db_mad = db_deviations[db_deviations.len() / 2];
                let db_sigma = db_mad * 1.4826;

                println!("    Debayered Statistics:");
                println!("      Min: {:.6}, Max: {:.6}", db_min, db_max);
                println!("      Mean: {:.6}, Median: {:.6}", db_mean, db_median);
                println!("      MAD: {:.6}, Sigma: {:.6}", db_mad, db_sigma);

                println!("\n    Star detection on debayered:");
                for sigma_thresh in [2.0f32, 3.0, 5.0, 7.0, 10.0] {
                    let config = DetectionConfig::default()
                        .with_sigma(sigma_thresh)
                        .with_min_snr(2.0);
                    let detector = StarDetector::new(config);
                    match detector.detect(&debayered) {
                        Ok(stars) => {
                            let top_snrs: Vec<f32> = stars.iter().take(5).map(|s| s.snr).collect();
                            println!(
                                "      sigma={:.1}: {} stars, top SNRs: {:?}",
                                sigma_thresh,
                                stars.len(),
                                top_snrs
                            );
                        }
                        Err(e) => println!("      sigma={:.1}: error - {}", sigma_thresh, e),
                    }
                }
            }
        }

        println!();
    }

    println!("=== DIAGNOSTIC COMPLETE ===\n");
}
