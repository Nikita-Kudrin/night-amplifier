//! What a deeper stack is allowed to look like.
//!
//! Stacking `N` frames buys `sqrt(N)` in signal-to-noise, and the tone curve decides
//! how it is spent. A scale-invariant curve spends all of it on faint-signal contrast
//! and none on the sky: the MTF solve pins `mtf(k * sigma) = target_background`, so
//! displayed grain is `T(1-T)/k` whatever `sigma` is, and 100 subs look exactly as
//! grainy as one (measured: 4.2 output levels at 1 sub, 4.4 at 8).
//! `render::autostretch::depth_grain_gain` splits it instead — `k` grows as `N^s`, so
//! grain falls as `N^-s` and contrast still rises as `N^(1/2 - s)`. The split `s` is the
//! Background Grain dial's expensive lever and defaults to `1/8`, not the even `1/4`;
//! the assertions read it from `depth_grain_gain` rather than restating it.
//!
//! Measured here in output bytes, through the real preview path and encoder: a
//! synthetic sky where only the noise amplitude changes (so nothing else can move),
//! the bundled fixture set, and — when present — a 106-sub session from outside the
//! repo.

use std::path::Path;

use serial_test::serial;

use crate::integration::instruments::{
    expected_grain_ratio, load_sub, measure_real_session, measure_sky_and_target, render,
    session_files,
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
    let mut settings = night_amplifier::server::state::CaptureSettings::default();
    settings.auto_stretch = true;
    settings.background_subtraction = false;

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

/// Stacks a real session, handing back `(depth, stack)` at each requested depth. The
/// implementation lives in `instruments`, where the Pro repo can reach it too.
pub(crate) fn stack_snapshots(
    files: &[std::path::PathBuf],
    depths: &[usize],
) -> Vec<(usize, night_amplifier::Frame)> {
    crate::integration::instruments::stack_snapshots(
        files,
        depths,
        &crate::integration::instruments::load_sub,
    )
}

/// The bundled fixture set, so this runs in CI.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_real_session_gets_calmer_with_depth() {
    let files = managed_session(FIXTURE_SET);
    // The tone curve's half only: the filters' half is the Pro repo's
    // (`denoise_depth_tests`), since without the plugin it renders no filters at all.
    measure_real_session(FIXTURE_SET, &files, &[1, 2, 4, 8, 16, 32], &[false], &load_sub);
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
    measure_real_session(DEEP_SET, &files, &[1, 2, 4, 8, 16, 32, 64, 106], &[false], &load_sub);
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
