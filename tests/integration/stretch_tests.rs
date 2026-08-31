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

        match auto_stretch_frame(&mut test_frame, config, None, night_amplifier::render::ShadowFloorRequest::NONE) {
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

    // The synthetic background is B > R > G by construction. Rendering must preserve
    // that ordering; a planar-read-as-interleaved render flattens it, because each
    // output pixel becomes three adjacent samples of the same channel.
    let bg = (10usize, 10usize);
    let src = (
        frame.get_pixel(bg.0, bg.1, 0),
        frame.get_pixel(bg.0, bg.1, 1),
        frame.get_pixel(bg.0, bg.1, 2),
    );
    assert!(
        src.2 > src.0 && src.0 > src.1,
        "fixture invariant: expected B > R > G, got {src:?}"
    );

    let idx = (bg.1 * width + bg.0) * 3;
    let out = (rgb8[idx], rgb8[idx + 1], rgb8[idx + 2]);
    assert!(
        out.2 >= out.0 && out.0 >= out.1,
        "render_to_rgb8 lost the channel ordering: frame {src:?} -> rgb8 {out:?}"
    );

    println!("\n=== Edge Case Tests Complete ===\n");
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn test_eyepiece_intensity_metrics() {
    println!("\n=== Eyepiece Intensity Metrics Test ===\n");

    let images = load_all_fixture_images();
    if images.is_empty() {
        println!("No test images found. Skipping test.\n");
        return;
    }

    // Find a deep sky image for realistic testing
    let img = images
        .iter()
        .find(|i| i.path.to_string_lossy().contains("deep_sky"))
        .unwrap_or(&images[0]);
    println!(
        "Testing metrics on: {:?}",
        img.path.file_name().unwrap_or_default()
    );

    // Runs the preview pipeline at `intensity`, then applies the fused LUT it produced
    // exactly as the streaming encoder does, and reports the mean median.
    //
    // `apply_scale_lut_frame` rather than a hand-rolled `par_chunks_mut(width * 3)` loop:
    // `Frame` is planar and the loop this replaced drove the *interleaved* kernel over it,
    // so both measurements below were taken from an image whose channels had been
    // scrambled into each other. The comparison still passed, because both sides were
    // scrambled identically — which is precisely why it had stopped being a regression
    // guard for the eyepiece intensity path.
    let mean_median_at = |intensity: f32| {
        let mut settings = night_amplifier::server::state::CaptureSettings::default();
        settings.auto_stretch = true;
        settings.eyepiece.intensity = intensity;

        let mut frame = img.frame.clone();
        if frame.channels() == 1 {
            frame = night_amplifier::debayer_auto(&frame).unwrap().0;
        }

        let (_config, stretch_res) =
            night_amplifier::server::capture::pipeline::process_preview_frame(
                &mut frame, &settings,
            )
            .unwrap();
        let res = stretch_res.unwrap();

        night_amplifier::render::stretch::apply_scale_lut_frame(
            &mut frame,
            res.black_point,
            &res.scale_lut,
            1.0,
        )
        .unwrap();

        let spread = crate::integration::common::mean_chroma_spread_frame(&frame);
        crate::integration::common::assert_has_chroma(
            spread,
            &format!("stretched preview at intensity {intensity}"),
        );

        night_amplifier::compute_image_stats(&frame)
            .unwrap()
            .mean_median()
    };

    let median_base = mean_median_at(0.0);
    let median_max = mean_median_at(1.0);

    println!("Base config - Median: {:.6}", median_base);
    println!("Max config  - Median: {:.6}", median_max);

    // Validate that background became darker
    assert!(
        median_max < median_base,
        "Expected darker background at max intensity ({} < {})",
        median_max,
        median_base
    );

    println!("=== Eyepiece Intensity Metrics Test Complete ===\n");
}
