//! What a deeper stack is allowed to look like.
//!
//! Stacking `N` frames buys `sqrt(N)` SNR; the tone curve decides the spend. A
//! scale-invariant curve spends it all on contrast, none on sky: grain is `T(1-T)/k`, so 100
//! subs look as grainy as 1 (4.2 output levels at 1 sub, 4.4 at 8). `depth_grain_gain` splits
//! it instead: `k` grows as `N^s`, grain falls as `N^-s`, contrast rises as `N^(1/2-s)`. `s`
//! is the Grain dial's lever, default `1/8` not `1/4`. Measured in output bytes via the real
//! preview/encoder path: synthetic sky, the fixture set, and a 106-sub session when present.

use std::path::Path;

use serial_test::serial;

use crate::integration::instruments::{
    expected_grain_ratio, load_sub, measure_real_session, measure_sky_and_target, render,
    session_files, GiveBack, NamedTarget,
};

/// The managed fixture set, which CI has: 35 IMX533 subs of M27 on a 250 mm Dobsonian.
const FIXTURE_SET: &str = "250mm-dob-imx533-dumbbell-fits";

/// The same target and rig, whole session: 106 subs cropped to 1024².
const DEEP_SET: &str = "deep-stack-dumbbell-106";

/// A flat sky, a faint Gaussian nebula and a sparse star field, plus one fixed
/// Gaussian noise pattern scaled by `sigma`: between depths the noise *amplitude* is
/// the only thing that differs.
fn synthetic_sky(sigma: f32) -> night_amplifier::Frame {
    const SIZE: usize = 1024;
    const SKY: f32 = 0.05;
    const NEBULA: f32 = 0.004;

    let plane = SIZE * SIZE;
    let mut data = vec![0.0f32; plane * 3];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut uniform = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        ((seed >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    };
    let c = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let r2 = (x as f32 - c).powi(2) + (y as f32 - c).powi(2);
            let nebula = NEBULA * (-r2 / (2.0 * 90.0f32.powi(2))).exp();
            for ch in 0..3 {
                let (u1, u2) = (uniform(), uniform());
                let gauss = ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32;
                data[ch * plane + y * SIZE + x] = SKY + nebula + sigma * gauss;
            }
        }
    }
    let mut frame = night_amplifier::Frame::from_f32_vec(data, SIZE, SIZE, 3).unwrap();
    for i in 0..200usize {
        let (sx, sy) = ((i * 7919) % (SIZE - 8) + 4, (i * 104_729) % (SIZE - 8) + 4);
        for dy in 0..3 {
            for dx in 0..3 {
                for ch in 0..3 {
                    let v = frame.get_pixel(sx + dx, sy + dy, ch);
                    frame.set_pixel(sx + dx, sy + dy, ch, (v + 0.3).min(1.0));
                }
            }
        }
    }
    frame
}

/// The split, with nothing but the noise able to move: a stack `N` times deeper is
/// rendered with `N^(-1/4)` of the sky grain and `N^(1/4)` of the faint-signal
/// contrast, so their ratio — the physical `sqrt(N)` — is unchanged.
///
/// Measured at native size (no downsample averaging), with denoising off (the tone
/// curve alone) and at the product default.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_deeper_stack_is_rendered_calmer_and_brighter_in_step() {
    let settings = night_amplifier::session::state::CaptureSettings {
        auto_stretch: true,
        background_subtraction: false,
        ..Default::default()
    };

    const SIGMA_1: f32 = 0.004;
    let sky = (0, 0, 256, 256);
    let target = (462, 462, 562, 562);

    for denoise in [false, true] {
        println!("\n=== Synthetic sky, denoise {} ===", if denoise { "default" } else { "off" });
        println!("   N   sigma(lin)  sky lvl  grain(lvl)  nebula(lvl)  nebula/grain");
        let mut rows = Vec::new();
        for n in [1u32, 4, 16, 64, 256] {
            let sigma = SIGMA_1 / (n as f32).sqrt();
            let (rgb8, w, _) = render(synthetic_sky(sigma), &settings, denoise, (4096, 4096), n);
            let m = measure_sky_and_target(&rgb8, w, sky, target);
            println!(
                "{n:>4}   {sigma:.6}   {:>6.1}   {:>9.2}   {:>10.1}   {:>11.2}",
                m.sky_level,
                m.sky_grain,
                m.target_contrast,
                m.target_contrast / m.sky_grain.max(1e-9)
            );
            rows.push((n, m));
        }

        let (_, first) = &rows[0];
        for (n, m) in rows.iter().skip(1) {
            let grain_ratio = m.sky_grain / first.sky_grain;
            let expected = expected_grain_ratio(*n as usize);
            if !denoise {
                // The tone curve alone. With denoising on the filters take their own
                // share of the grain, so the exponent is no longer the curve's.
                assert!(
                    grain_ratio < expected * 1.35 && grain_ratio > expected * 0.74,
                    "{n} frames: grain ratio {grain_ratio:.3}, expected about \
                     {expected:.3} (N^-1/4) — the curve is not spending the stack's \
                     depth on the sky"
                );
            }
            assert!(
                m.target_contrast > first.target_contrast,
                "{n} frames: faint-signal contrast fell to {:.1} levels from {:.1}; the \
                 calmer sky must not be paid for out of the target",
                m.target_contrast,
                first.target_contrast
            );
        }

        let (n_last, last) = &rows[rows.len() - 1];
        let snr_gain =
            (last.target_contrast / last.sky_grain) / (first.target_contrast / first.sky_grain);
        let physical = (*n_last as f64).sqrt();
        println!(
            "  contrast-to-grain rose {snr_gain:.1}x over {n_last} frames (sqrt(N) = {physical:.1})"
        );
        assert!(
            snr_gain > physical * 0.6,
            "the render threw away the stack's signal-to-noise: {snr_gain:.1}x against a \
             physical {physical:.1}x"
        );
    }
}

/// The bundled fixture set, so this runs in CI.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_real_session_gets_calmer_with_depth() {
    let files = managed_session(FIXTURE_SET);
    // The tone curve's half only: the filters' half is the Pro repo's
    // (`denoise_depth_tests`), since without the plugin it renders no filters at all.
    measure_real_session(FIXTURE_SET, &files, &[1, 2, 4, 8, 16, 32], &[false], GiveBack::Banded, &load_sub);
}

/// The same past the depth the gain stops at, which the 35-sub set cannot reach.
///
/// A session only pays for the trade while its own noise keeps falling; past that the
/// curve takes the difference out of the target. 106 subs is where that shows, and it
/// is why this set exists rather than sweeping the short one harder.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_deep_session_gets_calmer_with_depth() {
    let files = managed_session(DEEP_SET);
    measure_real_session(DEEP_SET, &files, &[1, 2, 4, 8, 16, 32, 64, 106], &[false], GiveBack::Banded, &load_sub);
}

/// Other targets and tones than M27: a faint emission nebula and two galaxies, IMX533,
/// full frame. The curve was tuned on one bright nebula; these are what it must also hold on.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_faint_nebula_session_gets_calmer_with_depth() {
    const SET: &str = "ic-59-ghost-of-cassiopeia-nebula";
    // The set's target, IC 63: ASTAP on sub 1 (1.23"/px at bin 2), OpenNGC's position. IC 59
    // lies off the frame, and the γ Cas glow rises from below it into the lower right.
    const IC_63: NamedTarget = NamedTarget { centre: (0.410, 0.462), frame: (3008, 3008) };
    let files = managed_session(SET);
    // Measured 2026-10-22 at the shipped tone, unprotected as Community renders it: 0-1
    // levels over the sky at every depth, both streams. Too faint at 15 subs for a floor,
    // so this holds the band; a protected IC 63 is Pro's (`catalog_protection_keeps_ic_63`).
    measure_real_session(SET, &files, &[1, 2, 4, 8, 15], &[false], GiveBack::Named(IC_63), &load_sub);
}

#[test]
fn a_named_target_is_measured_where_its_fractions_put_it_on_the_render() {
    let target = NamedTarget { centre: (0.410, 0.462), frame: (64, 48) };
    let stack = night_amplifier::Frame::filled(64, 48, 1, 0.1).unwrap();
    assert_eq!(target.block(&stack, (1440, 1440)), (542, 617, 638, 713));
    // A corner stays inside the frame, and a frame narrower than a block keeps it at 0.
    let corner = NamedTarget { centre: (1.0, 0.0), ..target };
    assert_eq!(corner.block(&stack, (1440, 1440)), (1344, 0, 1440, 96));
    assert_eq!(corner.block(&stack, (64, 48)), (0, 0, 96, 96));
}

/// Fractions solved on a crop point somewhere else on the full frame.
#[test]
#[should_panic(expected = "another frame size")]
fn a_named_target_located_on_another_frame_size_is_refused() {
    let target = NamedTarget { centre: (0.82, 0.67), frame: (1504, 1504) };
    target.block(&night_amplifier::Frame::filled(3008, 3008, 1, 0.1).unwrap(), (1440, 1440));
}

/// The sky box sits on the sky the stretch solved, not on darker real sky below it: a
/// frame-filling target leaves the darkest block in that population (M33's corner).
#[test]
fn the_sky_box_is_the_solved_sky_not_the_darkest_corner() {
    const SIZE: usize = 768;
    let mut rgb8 = vec![0u8; SIZE * SIZE * 3];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    for (i, px) in rgb8.chunks_mut(3).enumerate() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let (x, y) = (i % SIZE, i / SIZE);
        let (dx, dy) = (x as i64 - SIZE as i64 / 2, y as i64 - SIZE as i64 / 2);
        let level = if dx * dx + dy * dy < 80 * 80 {
            120
        } else if x < 300 && y < 300 {
            6 // the darker population a black point anchored on the mode puts near black
        } else {
            20
        };
        px.fill((level + (seed % 3) as i32 - 1) as u8);
    }
    let (sky, target) = crate::integration::instruments::locate_boxes(&rgb8, SIZE, SIZE);
    let sky_level = crate::integration::instruments::green_median(
        &crate::integration::instruments::crop_rgb8(&rgb8, SIZE, sky),
    );
    assert!((sky_level - 20.0).abs() <= 1.0, "sky box {sky:?} reads {sky_level} levels, not the solved sky's 20");
    assert!(sky.0 >= 300 || sky.1 >= 300, "sky box {sky:?} sits in the dark corner");
    let centre = ((target.0 + target.2) / 2, (target.1 + target.3) / 2);
    assert!(centre.0.abs_diff(SIZE / 2) < 80 && centre.1.abs_diff(SIZE / 2) < 80, "target box {target:?} missed the target");
}

#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_galaxy_session_gets_calmer_with_depth() {
    for set in ["m33-triangulum-galaxy-imx533", "m101-pinwheel-galaxy-imx533"] {
        let files = managed_session(set);
        measure_real_session(set, &files, &[1, 2, 4, 8, 16, 32], &[false], GiveBack::Banded, &load_sub);
    }
}

/// A managed fixture set's frames, downloading it if this machine does not have it.
///
/// Panics rather than skipping when it cannot be had: a depth assertion that silently
/// does not run is worse than no assertion, because the suite then reports green.
pub(crate) fn managed_session(name: &str) -> Vec<std::path::PathBuf> {
    use crate::integration::common::{missing_fixture_message, FIXTURES_DIR};
    crate::integration::common::ensure_fixtures_sync_named(&[name]);
    let dir = Path::new(FIXTURES_DIR).join(name);
    session_files(&dir).unwrap_or_else(|| panic!("{}", missing_fixture_message(name)))
}
