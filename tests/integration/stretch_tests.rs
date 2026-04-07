//! Tests for auto-stretch and rendering pipeline.

use night_amplifier::{
    auto_stretch_default, auto_stretch_frame, render_to_rgb8, AutoStretchConfig, Frame,
};
use serial_test::serial;

use crate::integration::image_loading::load_all_fixture_images;

/// Test just the stretching pipeline on a single image
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_stretch_pipeline_single_image() {
    println!("\n=== Single Image Stretch Test ===\n");

    let images = load_all_fixture_images();

    if images.is_empty() {
        println!("No test images found. Skipping test.\n");
        return;
    }

    let img = &images[0];
    println!(
        "Testing stretch on: {:?}\n",
        img.path.file_name().unwrap_or_default()
    );

    // Test different stretch presets
    let presets = [
        ("Default", AutoStretchConfig::default()),
        ("Dark Sky", AutoStretchConfig::dark_sky()),
        ("Preserve Faint", AutoStretchConfig::preserve_faint()),
        ("Light Polluted", AutoStretchConfig::light_polluted()),
    ];

    for (name, config) in presets {
        let mut test_frame = img.frame.clone();

        match auto_stretch_frame(&mut test_frame, config) {
            Ok(result) => {
                println!("{} preset:", name);
                println!("  Stretch factor: {:.2}", result.stretch_factor);
                println!("  Black point: {:.6}", result.black_point);
                println!("  Converged: {}\n", result.converged);
            }
            Err(e) => {
                println!("{} preset: Failed - {}\n", name, e);
            }
        }
    }

    println!("=== Stretch Test Complete ===\n");
}

/// Verify that the pipeline handles edge cases gracefully
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_pipeline_edge_cases() {
    println!("\n=== Edge Case Tests ===\n");

    // Test with synthetic data when no real images available
    println!("Testing with synthetic 100x100 image...");

    // Create a simple synthetic image with some structure
    let width = 100;
    let height = 100;
    let channels = 3;

    let mut frame = Frame::zeros(width, height, channels).expect("Failed to create test frame");

    // Add a gradient background
    for y in 0..height {
        for x in 0..width {
            let base = (y as f32 / height as f32) * 0.1; // Gradient
            frame.set_pixel(x, y, 0, base + 0.05);
            frame.set_pixel(x, y, 1, base + 0.04);
            frame.set_pixel(x, y, 2, base + 0.06);
        }
    }

    // Add a few synthetic "stars" (bright points)
    let star_positions = [(25, 25), (50, 50), (75, 75), (30, 70), (70, 30)];
    for (x, y) in star_positions {
        for c in 0..channels {
            frame.set_pixel(x, y, c, 0.8);
            // Add some blur around the star
            for dx in -1i32..=1 {
                for dy in -1i32..=1 {
                    let nx = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
                    let ny = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
                    let current = frame.get_pixel(nx, ny, c);
                    frame.set_pixel(nx, ny, c, current.max(0.4));
                }
            }
        }
    }

    // Test statistics
    let stats = night_amplifier::compute_image_stats(&frame).expect("Failed to compute stats");
    println!("  Stats computed - mean median: {:.4}", stats.mean_median());

    // Test star detection
    let detector = night_amplifier::StarDetector::new(
        night_amplifier::DetectionConfig::default().with_sigma(3.0),
    );
    let stars = detector.detect(&frame).expect("Failed to detect stars");
    println!("  Detected {} stars", stars.len());

    // Test stretching
    let mut stretched = frame.clone();
    let result = auto_stretch_default(&mut stretched);
    match result {
        Ok(r) => println!("  Auto-stretch applied (factor: {:.2})", r.stretch_factor),
        Err(e) => println!("  Auto-stretch failed (expected for synthetic data): {}", e),
    }

    // Test rendering
    let rgb8 = render_to_rgb8(&frame).expect("Failed to render");
    println!("  Rendered to {} bytes", rgb8.len());

    assert_eq!(rgb8.len(), width * height * 3);

    println!("\n=== Edge Case Tests Complete ===\n");
}
