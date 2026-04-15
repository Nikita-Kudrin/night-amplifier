//! Tests for debayering functionality with real Bayer images.

use night_amplifier::{debayer_auto, detect_cfa_pattern, DebayerAlgorithm, DebayerConfig};
use serial_test::serial;
use std::thread;

use crate::integration::common::{
    find_fixture_sets, prepare_test_output_dir, DEBAYER_OUTPUT_DIR, FIXTURES_DIR,
};
use crate::integration::image_loading::{load_image, save_processed_frame_to_dir};

/// Test debayering functionality with real Bayer images
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_debayer_pipeline() {
    println!("\n=== Debayer Pipeline Test ===\n");

    // Ensure fixtures are downloaded from Google Drive
    crate::integration::common::ensure_fixtures_sync();

    // Prepare dedicated output directory FIRST (clears old data before any processing)
    let output_dir = match prepare_test_output_dir(DEBAYER_OUTPUT_DIR) {
        Ok(dir) => {
            println!("Output directory: {}", dir.display());
            dir
        }
        Err(e) => {
            eprintln!("Warning: Failed to prepare output directory: {}", e);
            return;
        }
    };

    println!(
        "Looking for test images in fixture subdirectories: {}",
        FIXTURES_DIR
    );

    // Get first image from each fixture set (not all images)
    let fixture_sets = find_fixture_sets();
    let first_images: Vec<_> = fixture_sets
        .iter()
        .filter_map(|set| set.files.first().cloned())
        .collect();

    let total_sets = first_images.len();

    if total_sets == 0 {
        println!("No test images found. Skipping test.\n");
        return;
    }

    // Determine parallelism - use all CPUs since each image is processed sequentially
    let num_cpus = thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4);

    println!(
        "Found {} fixture set(s), testing first image from each ({} CPUs available).\n",
        total_sets, num_cpus
    );

    let mut processed_count = 0;
    let mut bayer_count = 0;

    // Process first image from each dataset
    for (idx, path) in first_images.iter().enumerate() {
        let idx = idx + 1;

        // Load single image
        let img = match load_image(path) {
            Ok(img) => img,
            Err(e) => {
                println!("[{}/{}] Failed to load {:?}: {}", idx, total_sets, path, e);
                continue;
            }
        };

        // Skip non-Bayer images
        if !img.is_bayer {
            println!(
                "[{}/{}] Skipping non-Bayer image: {:?}",
                idx, total_sets, path
            );
            continue;
        }

        bayer_count += 1;
        println!(
            "[{}/{}] Processing: {:?} ({}x{})",
            idx,
            total_sets,
            img.path.file_name().unwrap_or_default(),
            img.width,
            img.height
        );

        // Test pattern detection
        let detection = match detect_cfa_pattern(&img.frame) {
            Ok(detection) => {
                println!(
                    "  Pattern: {:?}, Confidence: {:.1}%",
                    detection.pattern,
                    detection.confidence * 100.0
                );
                detection
            }
            Err(e) => {
                println!("  Pattern detection failed: {}", e);
                continue;
            }
        };

        // Test debayering with auto-detection (uses Bilinear by default)
        match debayer_auto(&img.frame) {
            Ok((rgb_frame, _)) => {
                assert_eq!(rgb_frame.channels(), 3);
                assert_eq!(rgb_frame.width(), img.width);
                assert_eq!(rgb_frame.height(), img.height);

                // Test VNG algorithm as well (more expensive)
                let vng_config =
                    DebayerConfig::new(detection.pattern).with_algorithm(DebayerAlgorithm::Vng);
                match night_amplifier::debayer_with_config(&img.frame, vng_config) {
                    Ok(vng_result) => {
                        let sum: f64 = vng_result.data().iter().map(|&v| v as f64).sum();
                        let mean = sum / vng_result.data().len() as f64;
                        println!("  VNG algorithm: mean={:.4}", mean);
                    }
                    Err(e) => {
                        println!("  VNG algorithm failed: {}", e);
                    }
                }

                // Save debayered result (from auto/bilinear)
                let output_name = format!(
                    "debayered_{}",
                    img.path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                );
                match save_processed_frame_to_dir(&rgb_frame, &output_dir, &output_name) {
                    Ok(path) => {
                        println!("  Saved to: {}", path.display());
                    }
                    Err(e) => {
                        println!("  Warning: Could not save: {}", e);
                    }
                }

                processed_count += 1;
            }
            Err(e) => {
                println!("  Debayering failed: {}", e);
            }
        }

        // Image memory is released here at end of loop iteration
    }

    println!(
        "\n=== Debayer Pipeline Test Complete ({} Bayer images processed out of {} total) ===\n",
        processed_count, bayer_count
    );
}
