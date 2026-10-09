//! Every fixture set on disk, replayed through the application and rendered.
//!
//! Longer-running: each set goes through the simulated camera, the raw-CFA stage and the
//! live stack exactly as a simulator session would (`instruments::Replay`), then through
//! the deep-sky render pipeline, and the result is saved for a human to look at.

use std::io::{self, Write};
use std::path::Path;

use night_amplifier::{compute_image_stats, debayer_auto, DetectionConfig, StarDetector};
use serial_test::serial;

use crate::integration::common::{
    find_fixture_sets, prepare_test_output_dir, FixtureSet, MAX_STRETCH_FACTOR,
    MIN_FRAMES_FOR_STACKING, MIN_STRETCH_FACTOR, STACKED_OUTPUT_DIR,
};
use crate::integration::image_loading::{load_image, save_processed_frame_to_dir};
use crate::integration::instruments::Replay;

/// Stacks and renders every fixture set, saving each result under `processed/stacked/`.
///
/// Every set must render a converged, colour-neutral stretch. How many subs each keeps is
/// reported, not asserted: a re-base legitimately discards early subs (the globular set
/// keeps 8 of 12 that way), and retention is held where the sets' shapes are known —
/// `stacking_tests` and `stack_quality_tests`.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn every_fixture_set_stacks_and_renders() {
    crate::integration::common::ensure_fixtures_sync();

    let output_dir = prepare_test_output_dir(STACKED_OUTPUT_DIR)
        .unwrap_or_else(|e| panic!("cannot prepare the output directory: {e}"));
    let fixture_sets: Vec<FixtureSet> = find_fixture_sets()
        .into_iter()
        .filter(|set| set.files.len() >= MIN_FRAMES_FOR_STACKING)
        .collect();
    assert!(!fixture_sets.is_empty(), "no fixture set to process");

    for fixture_set in &fixture_sets {
        process_fixture_set(fixture_set, &output_dir);
    }
}

fn process_fixture_set(fixture_set: &FixtureSet, output_dir: &Path) {
    let name = fixture_set.name.as_str();
    println!("\n━━━ {name} ({} files) ━━━", fixture_set.files.len());

    let mut replay = Replay::open(&fixture_set.path);
    let offered = replay.subs();
    for (n, outcome) in replay.run().iter().enumerate() {
        if let Some(admission) = outcome.admission.as_ref().filter(|a| !a.added) {
            println!(
                "  sub {} not stacked: {:?} ({} stars matched, residual {:.2} px)",
                n + 1,
                admission.rejected_because,
                admission.matched_stars,
                admission.mean_residual
            );
        }
    }
    let integrated = replay.depth();
    let retention = integrated as f64 / offered as f64;
    println!("  {integrated}/{offered} subs integrated ({:.0}%)", retention * 100.0);

    let mut stacked = replay.snapshot();
    let render_config = night_amplifier::render::pipeline::RenderPipelineConfig::deep_sky();
    let rendered = night_amplifier::render::pipeline::RenderPipeline::new(render_config)
        .process(&mut stacked)
        .unwrap_or_else(|e| panic!("{name}: the render pipeline failed: {e}"));
    if let Some(stretch) = rendered.stretch_result {
        println!(
            "  stretch factor {:.2}, black point {:.6}, converged {}",
            stretch.stretch_factor, stretch.black_point, stretch.converged
        );
        assert!(stretch.converged, "{name}: auto-stretch did not converge");
        assert!(
            (MIN_STRETCH_FACTOR..=MAX_STRETCH_FACTOR).contains(&stretch.stretch_factor),
            "{name}: stretch factor {:.2} outside [{MIN_STRETCH_FACTOR}, {MAX_STRETCH_FACTOR}]",
            stretch.stretch_factor
        );
    }

    if stacked.channels() == 3 {
        let stats = compute_image_stats(&stacked).expect("stats of a rendered stack");
        let medians: Vec<f32> = stats.channels.iter().map(|c| c.median * 255.0).collect();
        println!("  channel medians R={:.2} G={:.2} B={:.2}", medians[0], medians[1], medians[2]);
        for (a, b) in [(0, 1), (1, 2), (0, 2)] {
            assert!(
                (medians[a] - medians[b]).abs() < 3.0,
                "{name}: background not neutral, channel medians {medians:?}"
            );
        }
    }

    let path = save_processed_frame_to_dir(&stacked, output_dir, name)
        .unwrap_or_else(|e| panic!("{name}: cannot save the result: {e}"));
    println!("  saved {}", std::fs::canonicalize(&path).unwrap_or(path).display());
    let _ = io::stdout().flush();
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
