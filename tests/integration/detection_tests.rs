//! Tests for star detection on real images.

use night_amplifier::{DetectionConfig, StarDetector};
use rayon::prelude::*;
use serial_test::serial;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::integration::common::MIN_STARS_FOR_REGISTRATION;
use crate::integration::image_loading::load_all_fixture_images;

/// Test star detection on loaded images
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_star_detection_on_real_images() {
    println!("\n=== Star Detection Test ===\n");

    let images = load_all_fixture_images();

    if images.is_empty() {
        println!("No test images found. Skipping test.\n");
        return;
    }

    let configs = [
        (
            "Conservative (sigma=8)",
            DetectionConfig::default().with_sigma(8.0),
        ),
        (
            "Standard (sigma=5)",
            DetectionConfig::default().with_sigma(5.0),
        ),
        (
            "Sensitive (sigma=3)",
            DetectionConfig::default().with_sigma(3.0),
        ),
    ];

    // Track detection results for validation (thread-safe)
    let any_image_has_sufficient_stars = AtomicBool::new(false);
    let best_detection_count = AtomicUsize::new(0);
    let best_avg_snr = Mutex::new(0.0f32);

    // Process all image+config combinations in parallel, grouped by image
    // Each image produces a vec of (name, result) for its configs
    let results: Vec<_> = images
        .par_iter()
        .map(|img| {
            let config_results: Vec<_> = configs
                .par_iter()
                .map(|(name, config)| {
                    let detector = StarDetector::new(config.clone());
                    let result = detector.detect(&img.frame);
                    (*name, result)
                })
                .collect();
            (img, config_results)
        })
        .collect();

    // Process results sequentially for ordered output
    for (img, config_results) in &results {
        println!("Image: {:?}", img.path.file_name().unwrap_or_default());

        for (name, result) in config_results {
            match result {
                Ok(stars) => {
                    let avg_snr: f32 = if stars.is_empty() {
                        0.0
                    } else {
                        stars.iter().map(|s| s.snr).sum::<f32>() / stars.len() as f32
                    };
                    println!(
                        "  {}: {} stars detected (avg SNR: {:.1})",
                        name,
                        stars.len(),
                        avg_snr
                    );

                    // Track best results for validation
                    best_detection_count.fetch_max(stars.len(), Ordering::Relaxed);
                    {
                        let mut best = best_avg_snr.lock().unwrap();
                        if avg_snr > *best {
                            *best = avg_snr;
                        }
                    }
                    if stars.len() >= MIN_STARS_FOR_REGISTRATION {
                        any_image_has_sufficient_stars.store(true, Ordering::Relaxed);
                    }
                }
                Err(e) => {
                    println!("  {}: Failed - {}", name, e);
                }
            }
        }
        println!();
    }

    let best_detection_count = best_detection_count.load(Ordering::Relaxed);
    let best_avg_snr = *best_avg_snr.lock().unwrap();
    let any_image_has_sufficient_stars = any_image_has_sufficient_stars.load(Ordering::Relaxed);

    // VALIDATION: Report summary and validate results
    println!("Detection Summary:");
    println!("  Best star count: {}", best_detection_count);
    println!("  Best average SNR: {:.1}", best_avg_snr);
    println!(
        "  Any image with sufficient stars (>={}): {}",
        MIN_STARS_FOR_REGISTRATION, any_image_has_sufficient_stars
    );

    // VALIDATION: At least one configuration should detect minimum stars
    assert!(
        best_detection_count >= 3,
        "VALIDATION FAILED: Best detection found only {} stars. \
         At least 3 stars are required for triangle matching.",
        best_detection_count
    );

    // VALIDATION: Average SNR should be above noise level
    assert!(
        best_avg_snr > 1.0,
        "VALIDATION FAILED: Best average SNR ({:.1}) is too low. \
         Stars may not be distinguishable from noise.",
        best_avg_snr
    );

    println!("\n=== Star Detection Test Complete ===\n");
}
