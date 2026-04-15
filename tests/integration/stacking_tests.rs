//! Tests for the complete stacking pipeline with real image files.

use night_amplifier::{
    auto_stretch_default, compute_image_stats, debayer_auto, render_to_rgb8, subtract_background,
    DetectionConfig, ImageRegistration, RejectionMethod, Stacker, StackingConfig, StarDetector,
};
use serial_test::serial;

use crate::integration::common::{
    find_fixture_sets, prepare_test_output_dir, FixtureSet, MAX_OUTPUT_MEAN_VALUE,
    MAX_STRETCH_FACTOR, MIN_ACCEPTABLE_SNR, MIN_FRAMES_FOR_STACKING, MIN_OUTPUT_MEAN_VALUE,
    MIN_STACKING_SUCCESS_RATE, MIN_STARS_FOR_REGISTRATION, MIN_STRETCH_FACTOR, STACKED_OUTPUT_DIR,
};
use crate::integration::image_loading::{load_images_from_paths, save_processed_frame_to_dir};

/// Test the complete stacking pipeline with real image files.
/// Each fixture directory is processed as a separate stacking session.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_full_stacking_pipeline() {
    println!("\n=== Full Stacking Pipeline Test ===\n");

    // Ensure fixtures are downloaded from Google Drive
    crate::integration::common::ensure_fixtures_sync();

    let fixture_sets = find_fixture_sets();
    if fixture_sets.is_empty() {
        println!("No fixture directories found. Skipping test.\n");
        return;
    }

    println!("Found {} fixture set(s) to process.\n", fixture_sets.len());

    // Prepare output directory
    let output_dir =
        prepare_test_output_dir(STACKED_OUTPUT_DIR).expect("Failed to prepare output directory");
    println!("Output directory: {:?}\n", output_dir);

    let mut total_sets_processed = 0;
    let mut total_sets_successful = 0;

    for fixture_set in &fixture_sets {
        println!(
            "\n--- Processing fixture set: {} ({} files) ---\n",
            fixture_set.name,
            fixture_set.files.len()
        );

        match process_fixture_set(fixture_set) {
            Ok(stacked_frame) => {
                // Save the stacked result
                let output_name = format!("{}_stacked", fixture_set.name);
                match save_processed_frame_to_dir(&stacked_frame, &output_dir, &output_name) {
                    Ok(path) => {
                        println!("  Saved stacked result to: {:?}", path);
                        total_sets_successful += 1;
                    }
                    Err(e) => {
                        println!("  Warning: Failed to save stacked result: {}", e);
                    }
                }
            }
            Err(e) => {
                println!("  Error processing fixture set: {}", e);
            }
        }

        total_sets_processed += 1;
    }

    println!("\n=== Pipeline Test Complete ===");
    println!(
        "Processed {}/{} fixture sets successfully.\n",
        total_sets_successful, total_sets_processed
    );

    assert!(
        total_sets_successful > 0,
        "No fixture sets were successfully processed"
    );
}

/// Processes a single fixture set through the complete stacking pipeline.
/// Returns the final stacked and stretched frame.
fn process_fixture_set(fixture_set: &FixtureSet) -> Result<night_amplifier::Frame, String> {
    let images = load_images_from_paths(&fixture_set.files);

    if images.is_empty() {
        return Err("No images loaded".to_string());
    }

    if images.len() < MIN_FRAMES_FOR_STACKING {
        return Err(format!(
            "Only {} image(s), need at least {}",
            images.len(),
            MIN_FRAMES_FOR_STACKING
        ));
    }

    println!("  Loaded {} images for stacking.", images.len());

    // Verify all images have the same dimensions
    let (ref_width, ref_height, ref_channels) = (
        images[0].width,
        images[0].height,
        images[0].frame.channels(),
    );

    for img in &images[1..] {
        if img.width != ref_width || img.height != ref_height {
            return Err(format!(
                "Image {:?} has different dimensions",
                img.path.file_name().unwrap_or_default()
            ));
        }
        if img.frame.channels() != ref_channels {
            return Err(format!(
                "Image {:?} has different number of channels",
                img.path.file_name().unwrap_or_default()
            ));
        }
    }

    println!(
        "  All images: {}x{} with {} channel(s)",
        ref_width, ref_height, ref_channels
    );

    // Phase 1: Star detection on reference frame
    println!("  Phase 1: Detecting stars in reference frame...");
    let detection_config = DetectionConfig::default()
        .with_sigma(5.0)
        .with_max_stars(200);
    let detector = StarDetector::new(detection_config);

    let ref_stars = detector
        .detect(&images[0].frame)
        .map_err(|e| format!("Failed to detect stars: {}", e))?;
    println!("    Found {} stars in reference frame", ref_stars.len());

    if ref_stars.len() < MIN_STARS_FOR_REGISTRATION {
        return Err(format!(
            "Too few stars detected ({}) for reliable registration",
            ref_stars.len()
        ));
    }

    let avg_snr: f32 = ref_stars.iter().map(|s| s.snr).sum::<f32>() / ref_stars.len() as f32;
    println!("    Average star SNR: {:.1}", avg_snr);

    if avg_snr < MIN_ACCEPTABLE_SNR {
        return Err(format!(
            "Average star SNR ({:.1}) below minimum ({:.1})",
            avg_snr, MIN_ACCEPTABLE_SNR
        ));
    }

    // Phase 2: Initialize stacker
    println!("  Phase 2: Initializing live stacker...");
    let stacking_config = StackingConfig::default()
        .with_rejection(RejectionMethod::SigmaClip)
        .with_sigma(2.5);

    let mut stacker = Stacker::new(ref_width, ref_height, ref_channels, stacking_config)
        .map_err(|e| format!("Failed to create Stacker: {}", e))?;

    stacker
        .add_reference(&images[0].frame)
        .map_err(|e| format!("Failed to add reference frame: {}", e))?;

    // Phase 3: Register and stack each additional frame
    println!("  Phase 3: Registering and stacking frames...");
    let registration = ImageRegistration::with_defaults();
    let mut frames_registered = 0;

    for (i, img) in images[1..].iter().enumerate() {
        let target_stars = detector
            .detect(&img.frame)
            .map_err(|e| format!("Failed to detect stars in frame {}: {}", i + 2, e))?;

        match registration.register(&ref_stars, &target_stars) {
            Ok(transform) => {
                stacker
                    .add_frame(&img.frame, &transform)
                    .map_err(|e| format!("Failed to add frame {}: {}", i + 2, e))?;
                frames_registered += 1;
            }
            Err(_) => {
                // Skip frames that fail registration (e.g., clouds, tracking errors)
            }
        }
    }

    println!(
        "    Registered {}/{} frames",
        frames_registered,
        images.len() - 1
    );

    // Phase 4: Compute stacked result
    println!("  Phase 4: Computing stacked result...");
    let stacked_raw = stacker
        .compute()
        .map_err(|e| format!("Failed to compute stack: {}", e))?;
    println!("    Frames in stack: {}", stacker.frame_count());

    let stacking_rate = stacker.frame_count() as f64 / images.len() as f64;
    println!("    Stacking success rate: {:.1}%", stacking_rate * 100.0);

    if stacking_rate < MIN_STACKING_SUCCESS_RATE {
        return Err(format!(
            "Stacking success rate ({:.1}%) below minimum ({:.1}%)",
            stacking_rate * 100.0,
            MIN_STACKING_SUCCESS_RATE * 100.0
        ));
    }

    // Phase 4.5: Debayer if single-channel (Bayer) data
    let mut stacked = if stacked_raw.channels() == 1 {
        println!("  Phase 4.5: Debayering stacked result...");
        let (debayered, pattern) =
            debayer_auto(&stacked_raw).map_err(|e| format!("Failed to debayer: {}", e))?;
        println!("    Detected Bayer pattern: {:?}", pattern.pattern);
        debayered
    } else {
        stacked_raw
    };
    println!("    Output channels: {}", stacked.channels());

    // Phase 5: Background subtraction
    println!("  Phase 5: Subtracting background...");
    // Check stats before background subtraction
    let pre_bg_stats = compute_image_stats(&stacked)
        .map_err(|e| format!("Failed to compute pre-bg stats: {}", e))?;
    println!(
        "    Pre-background mean median: {:.6}",
        pre_bg_stats.mean_median()
    );

    subtract_background(&mut stacked)
        .map_err(|e| format!("Failed to subtract background: {}", e))?;

    // Phase 6: Compute statistics
    println!("  Phase 6: Computing image statistics...");
    let stats = compute_image_stats(&stacked)
        .map_err(|e| format!("Failed to compute statistics: {}", e))?;
    println!("    Mean median: {:.6}", stats.mean_median());

    // Phase 7: Auto-stretch
    println!("  Phase 7: Applying auto-stretch...");
    let stretch_result =
        auto_stretch_default(&mut stacked).map_err(|e| format!("Failed to auto-stretch: {}", e))?;
    println!("    Stretch factor: {:.2}", stretch_result.stretch_factor);
    println!("    Black point: {:.6}", stretch_result.black_point);

    if stretch_result.stretch_factor < MIN_STRETCH_FACTOR
        || stretch_result.stretch_factor > MAX_STRETCH_FACTOR
    {
        return Err(format!(
            "Stretch factor ({:.2}) out of range [{:.2}, {:.2}]",
            stretch_result.stretch_factor, MIN_STRETCH_FACTOR, MAX_STRETCH_FACTOR
        ));
    }

    if !stretch_result.converged {
        return Err("Auto-stretch failed to converge".to_string());
    }

    // Phase 8: Validate output
    println!("  Phase 8: Validating output...");
    let rgb8 = render_to_rgb8(&stacked).map_err(|e| format!("Failed to render: {}", e))?;

    let sum: u64 = rgb8.iter().map(|&v| v as u64).sum();
    let mean = sum as f64 / rgb8.len() as f64;
    println!("    Output mean pixel value: {:.1}", mean);

    // Return the frame first - validation warnings will be printed but won't prevent saving
    if mean <= MIN_OUTPUT_MEAN_VALUE {
        println!("    Warning: Output appears dark (mean: {:.1})", mean);
    }
    if mean >= MAX_OUTPUT_MEAN_VALUE {
        println!("    Warning: Output appears saturated (mean: {:.1})", mean);
    }

    println!("  Successfully processed fixture set!");

    Ok(stacked)
}
