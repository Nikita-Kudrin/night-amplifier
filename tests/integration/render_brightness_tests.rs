//! The render's brightness-against-grain measurements, and a self-test of the instruments
//! every such measurement relies on.
//!
//! The instruments themselves live in [`crate::integration::instruments`], shared with
//! the Pro repo, which is also where the guard that uses them against the filters —
//! `a_bright_star_keeps_no_ring` — now runs: without the filters it compared a render
//! with itself. What stays here is the self-test that the instruments can see a defect
//! at all, and the brightness diagnostic over the four out-of-repo sessions.

use serial_test::serial;

use crate::integration::instruments::{
    anchors, cached_stack, find_isolated_stars, format_profile, profile_header,
    radial_excess_and_surround, render, report, requested_sessions, stream, OCTAVE_LABELS,
    PROFILE_RADII,
};

// ---------------------------------------------------------------------------
// The instruments' own guard
// ---------------------------------------------------------------------------

/// The instruments themselves, against defects injected on purpose.
///
/// `a_bright_star_keeps_no_ring` — in the Pro repo since the filters became a plugin, since
/// without them it compared a render against itself — passes on every setting reachable
/// today, as it should, but that alone does not show it *can* fail. The code that produced the 2026-09-18
/// defects is gone, so they cannot be replayed; this injects each one into a synthetic
/// field instead and asserts the instrument reports it. Without this the guard is only
/// known to be quiet, not known to be sensitive.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn the_ring_instruments_see_a_dip_and_a_disc() {
    const SIZE: usize = 600;
    const SKY: f64 = 20.0;

    // A flat sky and nine well-separated Gaussian stars, which is all the instruments need.
    let mut clean = vec![SKY; SIZE * SIZE];
    let centres: Vec<(usize, usize)> =
        (0..3).flat_map(|i| (0..3).map(move |j| (100 + i * 200, 100 + j * 200))).collect();
    for &(cx, cy) in &centres {
        for dy in -30i32..=30 {
            for dx in -30i32..=30 {
                let r2 = (dx * dx + dy * dy) as f64;
                let v = 230.0 * (-r2 / (2.0 * 2.2f64.powi(2))).exp();
                let p = (cy as i32 + dy) as usize * SIZE + (cx as i32 + dx) as usize;
                clean[p] = (clean[p] + v).min(255.0);
            }
        }
    }

    let stars = find_isolated_stars(&clean, SIZE, SIZE, 25);
    assert_eq!(stars.len(), centres.len(), "the synthetic field must be fully found");
    let (base, base_surround) = radial_excess_and_surround(&clean, SIZE, SIZE, &stars);
    println!("            {}", profile_header());
    println!("clean       {} surround {base_surround:+.2}", format_profile(&base));
    assert!(base_surround.abs() < 0.2, "a flat sky must read as a flat sky");

    // A trough just outside every star, which is what local statistics over a window
    // larger than a star do to it: -2.5 output levels at r=5 was measured on coarse
    // wavelet levels 5-6.
    let mut dipped = clean.clone();
    for &(cx, cy) in &centres {
        for dy in -12i32..=12 {
            for dx in -12i32..=12 {
                let r = ((dx * dx + dy * dy) as f64).sqrt();
                if (4.0..=9.0).contains(&r) {
                    let p = (cy as i32 + dy) as usize * SIZE + (cx as i32 + dx) as usize;
                    dipped[p] -= 2.5;
                }
            }
        }
    }
    let (dip, _) = radial_excess_and_surround(&dipped, SIZE, SIZE, &stars);
    println!("dipped      {}", format_profile(&dip));
    // Anywhere inside r=10, which is what the guard checks: at r=5 a real star's own wing
    // is ~17 levels here and swallows a 2.5 level trough whole. It surfaces at r=7-9,
    // where the wing has gone and the trough has not.
    let found = PROFILE_RADII
        .iter()
        .zip(&dip)
        .any(|(&r, &excess)| r < 10 && excess < -0.5);
    assert!(found, "a 2.5 level trough did not read negative anywhere inside r=10");

    // A lit disc: every star sitting in a pool of brighter sky, which is what a gain
    // driven by a local mean does. `sky_shadow` measured +2.3 output levels at r=21 px.
    // Flat out to 70 px, so it covers the 34-46 px annulus the surround is read from —
    // a disc that tapered to nothing before 34 px would register only its tail, which is
    // the instrument's real limitation and worth knowing rather than tuning away.
    let mut lit = clean.clone();
    for &(cx, cy) in &centres {
        for dy in -70i32..=70 {
            for dx in -70i32..=70 {
                let r = ((dx * dx + dy * dy) as f64).sqrt();
                if r <= 70.0 {
                    let p = (cy as i32 + dy) as usize * SIZE + (cx as i32 + dx) as usize;
                    lit[p] += 2.3;
                }
            }
        }
    }
    let (disc, disc_surround) = radial_excess_and_surround(&lit, SIZE, SIZE, &stars);
    println!("lit disc    {} surround {disc_surround:+.2}", format_profile(&disc));
    assert!(
        disc_surround > base_surround + 0.6,
        "a 2.3 level disc read as {disc_surround:+.2} against a clean {base_surround:+.2} — \
         the surround check would not fire"
    );

    // A ring: a bump out in the wings rather than a smooth decay.
    let mut ringed = clean.clone();
    for &(cx, cy) in &centres {
        for dy in -30i32..=30 {
            for dx in -30i32..=30 {
                let r = ((dx * dx + dy * dy) as f64).sqrt();
                if (16.0..=22.0).contains(&r) {
                    let p = (cy as i32 + dy) as usize * SIZE + (cx as i32 + dx) as usize;
                    ringed[p] += 3.0;
                }
            }
        }
    }
    let (ring, _) = radial_excess_and_surround(&ringed, SIZE, SIZE, &stars);
    println!("ringed      {}", format_profile(&ring));
    let rose = (1..PROFILE_RADII.len())
        .any(|i| PROFILE_RADII[i] >= 5 && ring[i] > ring[i - 1] + 0.3);
    assert!(rose, "a 3 level ring at r=16-22 did not turn the profile back up");
}

// ---------------------------------------------------------------------------
// Diagnostics over the four out-of-repo sessions (harness in `instruments`)
// ---------------------------------------------------------------------------


/// Every instrument on every session, at whatever the code currently does.
///
/// The baseline half of a before/after: run it, change the code, run it again. The tone
/// curve's split reaches the render through settings, so the dial is swept in one run
/// ([`sweep_the_background_grain_dial`]); `ContrastConfig` is fused into the scale LUT
/// inside the preview path, so a strength comparison needs two builds.
#[test]
#[serial]
#[ignore = "diagnostic - the four sessions live outside the repo; run with --ignored"]
fn measure_render_brightness_on_real_sessions() {
    use night_amplifier::render::autostretch::StretchAggressiveness;

    let profiles = [
        ("nebulae", StretchAggressiveness::High),
        ("deep-sky", StretchAggressiveness::Medium),
        ("star-fields", StretchAggressiveness::Low),
    ];
    if std::env::var("RENDER_BRIGHTNESS_TRACE").is_ok() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("night_amplifier::render::autostretch=debug")
            .with_test_writer()
            .try_init();
    }
    let only = std::env::var("RENDER_BRIGHTNESS_PROFILES").ok();
    let shipped = night_amplifier::render::ContrastConfig::default();
    println!(
        "\ncontrast strength {:.2}, midpoint {:.2}; octaves: {}",
        shipped.strength,
        shipped.midpoint,
        OCTAVE_LABELS.join("  ")
    );

    for (name, dir) in requested_sessions() {
        let Some((stack, depth)) = cached_stack(name, dir) else {
            println!("  {name}: not on this machine, skipped");
            continue;
        };
        println!("\n=== {name}, {depth} subs ===");
        let a = anchors(name, &stack, depth);
        println!("        {}", profile_header());
        for (profile_name, aggressiveness) in profiles {
            if let Some(list) = &only {
                if !list.split(',').any(|w| w.trim() == profile_name) {
                    continue;
                }
            }
            let mut settings = night_amplifier::server::state::CaptureSettings::default();
            settings.stretch_aggressiveness = aggressiveness;
            // The live settings file pins the denoise block; the defaults are what is
            // being measured, so they are restated rather than inherited.
            settings.denoise = night_amplifier::server::state::DenoiseSettings::default();
            let (rgb8, w, h) = render(stack.clone(), &settings, true, stream(), depth);
            report(&format!("{name}/{profile_name}"), &rgb8, w, h, &a);
        }
    }
}
