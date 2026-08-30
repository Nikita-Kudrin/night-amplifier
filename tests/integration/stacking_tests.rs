//! Tests for the complete stacking pipeline with real image files.

use night_amplifier::{
    auto_stretch_default, compute_image_stats, debayer_auto, render_to_rgb8, subtract_background,
    DetectionConfig, ImageRegistration, RejectionMethod, Stacker, StackingConfig, StarDetector,
};
use serial_test::serial;

use crate::integration::common::{
    find_fixture_sets, prepare_test_output_dir, FixtureSet, MANAGED_FIXTURE_SETS,
    MAX_OUTPUT_MEAN_VALUE, MAX_REBASES_PER_SESSION, MAX_RESIDUAL_REJECTION_SHARE,
    MAX_STRETCH_FACTOR, MIN_ACCEPTABLE_SNR,
    MIN_FRAMES_FOR_STACKING, MIN_LIVE_STACKING_RETENTION, MIN_OUTPUT_MEAN_VALUE,
    MAX_WANDERER_RESET_SHARE, MIN_STACKING_SUCCESS_RATE, MIN_STARS_FOR_REGISTRATION,
    MIN_STRETCH_FACTOR, STACKED_OUTPUT_DIR,
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
        .with_rejection(RejectionMethod::None)
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

    // Snapshot testing to ensure mathematical algorithms remain stable
    // (Disabled because parallel floating-point reductions and sampling cause tiny drifts across hardware)
    // insta::assert_debug_snapshot!(format!("{}_stats", fixture_set.name), stats);

    // Phase 7: Auto-stretch
    println!("  Phase 7: Applying auto-stretch...");
    let stretch_result =
        auto_stretch_default(&mut stacked).map_err(|e| format!("Failed to auto-stretch: {}", e))?;
    println!("    Stretch factor: {:.2}", stretch_result.stretch_factor);
    println!("    Black point: {:.6}", stretch_result.black_point);

    // insta::assert_debug_snapshot!(format!("{}_stretch", fixture_set.name), stretch_result);

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

#[test]
fn test_pipeline_rejects_corrupted_frame() {
    use night_amplifier::{Frame, PipelineConfig, StackingPipeline};

    // Create a valid reference frame
    let ref_frame = Frame::filled(64, 64, 1, 0.5).unwrap();
    let mut config = PipelineConfig::fast();
    config.min_stars = 0;

    let mut pipeline = StackingPipeline::new(&ref_frame, config).unwrap();

    // Create a completely NaN corrupted frame
    let corrupted_data = vec![f32::NAN; 64 * 64];
    let corrupted_frame = Frame::from_f32_vec(corrupted_data, 64, 64, 1).unwrap();

    // Process the corrupted frame
    let _result = pipeline.process_frame(&corrupted_frame);

    // It should just register as 1 frame stacked (the reference) without crashing
    assert!(pipeline.stats().frames_stacked == 1);
}

/// Drives the live-stacking context over the managed fixture sets, the way the
/// capture task does, and holds the line on the two numbers that regressed
/// together: how many frames survive registration, and how well the survivors
/// align.
///
/// Restricted to `MANAGED_FIXTURE_SETS` because `tests/fixtures/` is gitignored
/// and may hold stray capture output on any given machine.
///
/// Registration used to run on the 30 stars `DetectionConfig::fast()` returned,
/// and every transform it produced was stacked regardless of how badly it fitted
/// — 6 px residuals included, which is what smeared the result.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn live_stacking_keeps_frames_and_aligns_them_well() {
    use night_amplifier::server::capture::StackingContext;
    use night_amplifier::server::state::CaptureSettings;

    crate::integration::common::ensure_fixtures_sync();

    let fixture_sets = find_fixture_sets();
    if fixture_sets.is_empty() {
        println!("No fixture directories found. Skipping test.\n");
        return;
    }

    let settings = CaptureSettings::default();
    let mut sets_checked = 0;

    for fixture_set in &fixture_sets {
        if !MANAGED_FIXTURE_SETS.contains(&fixture_set.name.as_str()) {
            continue;
        }
        let images = load_images_from_paths(&fixture_set.files);
        if images.len() < MIN_FRAMES_FOR_STACKING {
            continue;
        }

        let frames: Vec<night_amplifier::Frame> = images
            .iter()
            .map(|img| {
                if img.is_bayer {
                    debayer_auto(&img.frame)
                        .map(|(f, _)| f)
                        .unwrap_or_else(|_| img.frame.clone())
                } else {
                    img.frame.clone()
                }
            })
            .collect();

        let mut ctx = StackingContext::new(
            frames[0].width(),
            frames[0].height(),
            frames[0].channels(),
            &settings,
        )
        .expect("stacking context should be creatable");

        if ctx.initialize_with_reference(&frames[0]).is_err() {
            println!(
                "  {}: reference frame has too few stars, skipping",
                fixture_set.name
            );
            continue;
        }

        let mut accepted = Vec::new();
        let mut dropped_on_alignment = Vec::new();
        let mut rebases = 0;

        for frame in frames.iter().skip(1) {
            let admission = ctx.add_frame(frame).expect("context is initialized");
            if admission.rebased {
                rebases += 1;
            }
            if !admission.mean_residual.is_finite() {
                continue; // never registered, so it has no fit to judge
            }
            match admission.rejected_because {
                None => accepted.push(admission.mean_residual),
                // A frame can align perfectly and still be dropped for bloated
                // stars, so only the alignment verdicts belong in this
                // comparison.
                Some(reason) if reason.is_about_alignment_quality() => {
                    dropped_on_alignment.push(admission.mean_residual)
                }
                Some(_) => {}
            }
        }

        // Retention is what ended up *in the stack*, not how many admissions
        // came back `added`. A re-base is an admission that throws away every
        // frame before it, so counting admissions would score a gate that
        // re-based on every frame as perfect.
        let integrated = ctx.frame_count();
        let rate = integrated as f64 / frames.len() as f64;
        println!(
            "  {}: {}/{} frames integrated ({:.0}%), {} dropped on alignment, {rebases} re-base(s)",
            fixture_set.name,
            integrated,
            frames.len(),
            rate * 100.0,
            dropped_on_alignment.len()
        );

        assert!(
            rate >= MIN_LIVE_STACKING_RETENTION,
            "{}: only {:.0}% of frames reached the stack, expected at least {:.0}%",
            fixture_set.name,
            rate * 100.0,
            MIN_LIVE_STACKING_RETENTION * 100.0
        );

        assert!(
            rebases <= MAX_REBASES_PER_SESSION,
            "{}: re-based {rebases} times — each one discards the integration \
             built so far and drops the preview back to a single sub, so this is \
             the gate chasing noise in the FWHM estimate",
            fixture_set.name
        );

        // The gate decides online, against a rolling median that moves as the
        // night goes on, so no fixed threshold describes its verdicts after the
        // fact and no strict per-frame ordering is guaranteed. What must hold is
        // the distributional claim the gate exists to make: the frames it kept
        // align better than the ones it rejected for aligning badly. If that
        // fails, it is discarding integration without buying sharpness.
        if !dropped_on_alignment.is_empty() {
            let kept = median_of(&accepted);
            let dropped = median_of(&dropped_on_alignment);
            assert!(
                kept < dropped,
                "{}: kept frames align at {:.2} px, dropped ones at {:.2} px — \
                 the gate is not improving the stack",
                fixture_set.name,
                kept,
                dropped
            );
        }

        sets_checked += 1;
    }

    assert!(sets_checked > 0, "No fixture sets were checked");
}

fn median_of(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}

/// Loads a managed fixture set and debayers it the way the capture task would.
///
/// Returns `None` when the set is absent or too short to stack, so callers can
/// skip rather than fail on a machine that has not downloaded it.
fn managed_fixture_frames(name: &str) -> Option<Vec<night_amplifier::Frame>> {
    let fixture_set = find_fixture_sets().into_iter().find(|s| s.name == name)?;
    let images = load_images_from_paths(&fixture_set.files);
    if images.len() < MIN_FRAMES_FOR_STACKING {
        return None;
    }
    Some(
        images
            .iter()
            .map(|img| {
                if img.is_bayer {
                    debayer_auto(&img.frame)
                        .map(|(f, _)| f)
                        .unwrap_or_else(|_| img.frame.clone())
                } else {
                    img.frame.clone()
                }
            })
            .collect(),
    )
}

/// A rig that tracks well must not end up with the strictest gate.
///
/// `RESIDUAL_K * median_residual` on its own is scale-multiplicative, so the
/// tighter a session's own scatter the tighter its limit: the 250 mm dumbbell
/// fixture holds a ~0.6 px median residual on ~5.4 px stars, and that rule
/// allowed only 1.8 px and dropped 9 of its 34 frames for residuals of 1.9–3.3 px
/// — a fraction of one star's width. Misalignment only matters against the width
/// of what it is smearing, so the limit carries a floor tied to star size.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_well_tracked_session_is_not_punished_for_its_own_precision() {
    use night_amplifier::server::capture::{RejectionReason, StackingContext};
    use night_amplifier::server::state::CaptureSettings;

    crate::integration::common::ensure_fixtures_sync();

    let settings = CaptureSettings::default();
    let mut sets_checked = 0;

    for name in MANAGED_FIXTURE_SETS {
        let Some(frames) = managed_fixture_frames(name) else {
            continue;
        };

        let mut ctx = StackingContext::new(
            frames[0].width(),
            frames[0].height(),
            frames[0].channels(),
            &settings,
        )
        .expect("stacking context should be creatable");
        if ctx.initialize_with_reference(&frames[0]).is_err() {
            continue;
        }

        let mut residuals = Vec::new();
        let mut fwhm_of_stack = None;
        let mut residual_rejections = 0;

        for frame in frames.iter().skip(1) {
            let admission = ctx.add_frame(frame).expect("context is initialized");
            if admission.rejected_because == Some(RejectionReason::ResidualTooHigh) {
                residual_rejections += 1;
            }
            if admission.mean_residual.is_finite() {
                residuals.push(admission.mean_residual);
            }
        }

        if residuals.is_empty() {
            continue;
        }
        let stacked = ctx.compute().expect("stack should compute");
        if let Ok(stars) = night_amplifier::detection::detect_stars_adaptive(&stacked) {
            fwhm_of_stack = night_amplifier::detection::compute_median_fwhm(&stars);
        }

        let median_residual = median_of(&residuals);
        let Some(star_width) = fwhm_of_stack else {
            continue;
        };
        let offered = frames.len() - 1;
        let share = residual_rejections as f64 / offered as f64;

        println!(
            "  {name}: median residual {median_residual:.2} px on {star_width:.2} px stars, \
             {residual_rejections}/{offered} dropped on residual ({:.0}%)",
            share * 100.0
        );

        // Only sessions that are actually tracking well make the claim; a set
        // whose residuals genuinely approach its star width should still lose
        // frames.
        if median_residual < 0.5 * star_width {
            assert!(
                share <= MAX_RESIDUAL_REJECTION_SHARE,
                "{name}: aligns to {median_residual:.2} px on {star_width:.2} px stars — \
                 well inside one star — yet {:.0}% of frames were dropped for a high \
                 residual. The gate is scoring precision against itself again.",
                share * 100.0
            );
            sets_checked += 1;
        }
    }

    assert!(
        sets_checked > 0,
        "no well-tracked fixture set was available to check"
    );
}

/// Wanderer mode restarts the stack when a frame cannot be placed against the
/// reference — the user having swung the telescope to a new object.
///
/// The gate widened "did not stack" well past that: it also rejects frames that
/// aligned perfectly and were merely soft or loose. Wanderer read every rejection
/// as movement, so a passing cloud restarted the integration. Checked over the
/// real fixture sets because it is the pipeline, not the classifier, that decides
/// which verdict a given frame gets.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn wanderer_holds_the_stack_through_the_frames_a_session_dislikes() {
    use night_amplifier::server::capture::StackingContext;
    use night_amplifier::server::state::CaptureSettings;

    crate::integration::common::ensure_fixtures_sync();

    let settings = CaptureSettings::default();
    let mut sets_checked = 0;

    for name in MANAGED_FIXTURE_SETS {
        let Some(frames) = managed_fixture_frames(name) else {
            continue;
        };

        let mut ctx = StackingContext::new(
            frames[0].width(),
            frames[0].height(),
            frames[0].channels(),
            &settings,
        )
        .expect("stacking context should be creatable");
        if ctx.initialize_with_reference(&frames[0]).is_err() {
            continue;
        }

        let mut would_reset = 0;
        let mut quality_rejections = 0;

        for frame in frames.iter().skip(1) {
            let admission = ctx.add_frame(frame).expect("context is initialized");
            let Some(reason) = admission.rejected_because else {
                continue;
            };
            if reason.means_the_sky_moved() {
                would_reset += 1;
                continue;
            }
            quality_rejections += 1;
        }

        let offered = frames.len() - 1;
        let reset_share = would_reset as f64 / offered as f64;
        println!(
            "  {name}: {would_reset}/{offered} frames would reset the stack ({:.0}%),              {quality_rejections} soft frames held it",
            reset_share * 100.0
        );

        // A frame that genuinely will not register has always meant movement and
        // still does. What must not happen is Wanderer restarting for most of a
        // session that is simply having a rough night.
        assert!(
            reset_share <= MAX_WANDERER_RESET_SHARE,
            "{name}: Wanderer would restart the stack on {:.0}% of a real session —              quality verdicts are being read as the telescope having been moved",
            reset_share * 100.0
        );
        sets_checked += 1;
    }

    assert!(sets_checked > 0, "No fixture sets were checked");
}

/// The other half: a genuinely different sky must still restart the stack, or
/// Wanderer does not work at all.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn wanderer_reads_a_new_target_as_movement() {
    use night_amplifier::server::capture::StackingContext;
    use night_amplifier::server::state::CaptureSettings;

    crate::integration::common::ensure_fixtures_sync();

    let settings = CaptureSettings::default();
    let mut pairs_checked = 0;

    // Sets sharing a sensor, so a frame from one can stand in for the telescope
    // having been swung to the other.
    let pairings = [
        (
            "130mm-imx464-dumbell-nebulae-png",
            "130mm-imx464-ring-nebulae-png",
        ),
        (
            "130mm-imx464-ring-nebulae-png",
            "250mm-dob-imx464-orion-png",
        ),
    ];

    for (home, elsewhere) in pairings {
        let (Some(frames), Some(other)) = (
            managed_fixture_frames(home),
            managed_fixture_frames(elsewhere),
        ) else {
            continue;
        };
        if frames[0].width() != other[0].width() || frames[0].height() != other[0].height() {
            continue;
        }

        let mut ctx = StackingContext::new(
            frames[0].width(),
            frames[0].height(),
            frames[0].channels(),
            &settings,
        )
        .expect("stacking context should be creatable");
        if ctx.initialize_with_reference(&frames[0]).is_err() {
            continue;
        }
        // Build a real stack first, so the verdict is reached the way it would be
        // mid-session rather than against a cold gate.
        for frame in frames.iter().skip(1).take(6) {
            let _ = ctx.add_frame(frame);
        }

        let admission = ctx.add_frame(&other[0]).expect("context is initialized");
        let reason = admission.rejected_because.unwrap_or_else(|| {
            panic!("{home} -> {elsewhere}: a completely different field was stacked")
        });
        assert!(
            reason.means_the_sky_moved(),
            "{home} -> {elsewhere}: a completely different field must reset the stack, \
             got {}",
            reason.describe()
        );

        println!(
            "  {home} -> {elsewhere}: reads as movement ({})",
            reason.describe()
        );
        pairs_checked += 1;
    }

    assert!(pairs_checked > 0, "no fixture pairing was available to check");
}
