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

use crate::integration::display_output_tests::sky_sigma_levels;
use crate::integration::image_loading::load_image;

/// The managed fixture set, which CI has: 35 IMX533 subs of M27 on a 250 mm Dobsonian.
const FIXTURE_SET: &str = "250mm-dob-imx533-dumbbell-fits";

/// The same target and rig, whole session: 106 subs cropped to 1024².
const DEEP_SET: &str = "deep-stack-dumbbell-106";

/// The grain ratio a stack of `frames` is expected to reach, at the shipped split.
///
/// Read off the curve itself rather than restated as `N^(-1/8)`: the exponent and the
/// depth it stops at are the product's decision — now the middle of a user-facing dial —
/// and a copy here would go on asserting the old one after that decision changed.
fn expected_grain_ratio(frames: usize) -> f64 {
    1.0 / night_amplifier::render::depth_grain_gain(
        frames as u32,
        night_amplifier::render::DEFAULT_GRAIN_SPLIT,
    ) as f64
}

struct Measured {
    sky_grain: f64,
    sky_level: f64,
    target_contrast: f64,
}

fn crop(rgb8: &[u8], width: usize, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        let row = y * width;
        out.extend_from_slice(&rgb8[(row + x0) * 3..(row + x1) * 3]);
    }
    out
}

fn green_median(rgb8: &[u8]) -> f64 {
    let mut g: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    g.sort_unstable();
    g[g.len() / 2] as f64
}

fn measure(rgb8: &[u8], width: usize, sky: (usize, usize, usize, usize), target: (usize, usize, usize, usize)) -> Measured {
    let sky_px = crop(rgb8, width, sky);
    let target_px = crop(rgb8, width, target);
    let sky_level = green_median(&sky_px);
    Measured {
        sky_grain: sky_sigma_levels(&sky_px, 1),
        sky_level,
        target_contrast: green_median(&target_px) - sky_level,
    }
}

pub(crate) fn render(
    frame: night_amplifier::Frame,
    settings: &night_amplifier::server::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
) -> (Vec<u8>, usize, usize) {
    render_with(frame, settings, denoise, max, stack_depth, |_| {})
}

/// [`render`], with a last look at the pipeline config before it is used.
///
/// The denoisers run in the encoder rather than the pipeline, so a caller can still
/// change their configuration after the solve — which is what lets an experiment try a
/// threshold ladder without a per-variant rebuild.
pub(crate) fn render_with(
    mut frame: night_amplifier::Frame,
    settings: &night_amplifier::server::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
    tweak: impl FnOnce(&mut night_amplifier::render::RenderPipelineConfig),
) -> (Vec<u8>, usize, usize) {
    use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis};
    // Through the analysis door, not `process_preview_frame`: the stretch spends the
    // stack's depth on how calm the sky is, so a one-shot render would measure a
    // single-frame tone curve over a deep stack.
    let (mut pipeline_config, stretch_result) =
        night_amplifier::server::capture::pipeline::process_preview_frame_with_analysis(
            &mut frame,
            settings,
            AnalysisContext {
                showing_stack: stack_depth > 1,
                stack_depth,
            },
            &mut PreviewAnalysis::new(),
        )
        .unwrap();
    if !denoise {
        pipeline_config.denoise = night_amplifier::render::DenoiseConfig::OFF;
    }
    tweak(&mut pipeline_config);
    let ready = night_amplifier::server::state::RenderReadyFrame {
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    };
    let (bytes, w, h) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, max.0, max.1).unwrap();
    (bytes, w as usize, h as usize)
}

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
            let m = measure(&rgb8, w, sky, target);
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

/// Darkest and brightest 96 px block of the green channel, by median.
fn locate_boxes(rgb8: &[u8], w: usize, h: usize) -> ((usize, usize, usize, usize), (usize, usize, usize, usize)) {
    const B: usize = 96;
    let mut darkest = (f64::MAX, (0, 0, B, B));
    let mut brightest = (f64::MIN, (0, 0, B, B));
    // Stay off the edges: registration leaves a partially covered border.
    for y in (B..h - 2 * B).step_by(B / 2) {
        for x in (B..w - 2 * B).step_by(B / 2) {
            let bx = (x, y, x + B, y + B);
            let med = green_median(&crop(rgb8, w, bx));
            if med < darkest.0 {
                darkest = (med, bx);
            }
            if med > brightest.0 {
                brightest = (med, bx);
            }
        }
    }
    (darkest.1, brightest.1)
}

/// Stacks a real session, handing back `(depth, stack)` at each requested depth.
///
/// Frames go through the capture path's raw-CFA stage (hot pixels, row/column FPN) and
/// demosaic, as the capture task runs them: a bare debayer leaves every hot pixel in,
/// and registration drags each one into a dotted trail along the session's drift.
pub(crate) fn stack_snapshots(
    files: &[std::path::PathBuf],
    depths: &[usize],
) -> Vec<(usize, night_amplifier::Frame)> {
    use night_amplifier::server::capture::pipeline::{build_cfa_pipeline, debayer_algorithm};
    use night_amplifier::server::capture::StackingContext;

    let settings = night_amplifier::server::state::CaptureSettings::default();
    let cfa_pipeline = build_cfa_pipeline(&settings);
    let algorithm = debayer_algorithm(&settings);

    let mut pattern = None;
    let mut load = |path: &Path| {
        let img = load_image(path).unwrap();
        if !img.is_bayer {
            return img.frame;
        }
        let p = *pattern.get_or_insert_with(|| {
            night_amplifier::debayer::detect_cfa_pattern(&img.frame).unwrap().pattern
        });
        let mut cfa = night_amplifier::CfaFrame::mosaic(img.frame, p).unwrap();
        cfa_pipeline.apply(&mut cfa);
        cfa.debayer(algorithm).unwrap()
    };

    let reference = load(&files[0]);
    let mut ctx = StackingContext::new(
        reference.width(),
        reference.height(),
        reference.channels(),
        &settings,
    )
    .unwrap();
    ctx.initialize_with_reference(&reference).unwrap();
    drop(reference);

    let mut snapshots = Vec::new();
    let mut next = 0;
    for path in files.iter().skip(1) {
        while next < depths.len() && ctx.frame_count() >= depths[next] {
            snapshots.push((ctx.frame_count(), ctx.compute().unwrap()));
            next += 1;
        }
        let _ = ctx.add_frame(&load(path));
    }
    if snapshots.last().map(|s| s.0) != Some(ctx.frame_count()) {
        snapshots.push((ctx.frame_count(), ctx.compute().unwrap()));
    }
    snapshots
}

/// FITS files of a session directory, sorted, or `None` when it is not on this machine.
pub(crate) fn session_files(dir: &Path) -> Option<Vec<std::path::PathBuf>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "fits" || e == "fit"))
        .collect();
    if files.len() < 8 {
        return None;
    }
    files.sort();
    Some(files)
}

/// Grain against depth on a real session, the target against that grain, and how far
/// the sky level moves between one stack update and the next.
fn measure_real_session(label: &str, files: &[std::path::PathBuf], depths: &[usize]) {
    if std::env::var("STACK_GRAIN_TRACE").is_ok() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("night_amplifier::render::autostretch=debug")
            .with_test_writer()
            .try_init();
    }
    let settings = night_amplifier::server::state::CaptureSettings::default();
    let snapshots = stack_snapshots(files, depths);

    let deepest = snapshots.last().unwrap();
    let (deep_rgb, w, h) =
        render(deepest.1.clone(), &settings, false, (2560, 1440), deepest.0 as u32);
    let (sky, target) = locate_boxes(&deep_rgb, w, h);

    for denoise in [false, true] {
        println!(
            "\n=== {label}, denoise {} (sky {sky:?}, target {target:?}) ===",
            if denoise { "default" } else { "off" }
        );
        println!("   N   sigma(ADU)  sky lvl  grain(lvl)  target(lvl)  target/grain  N^-1/4");
        let mut rows = Vec::new();
        for (n, stack) in &snapshots {
            let stats = night_amplifier::statistics::compute_image_stats(stack).unwrap();
            let (rgb8, w, _) = render(stack.clone(), &settings, denoise, (2560, 1440), *n as u32);
            let m = measure(&rgb8, w, sky, target);
            println!(
                "{n:>4}   {:>9.2}   {:>6.1}   {:>9.2}   {:>10.1}   {:>11.2}   {:>7.2}",
                stats.mean_sigma() * 65535.0,
                m.sky_level,
                m.sky_grain,
                m.target_contrast,
                m.target_contrast / m.sky_grain.max(1e-9),
                expected_grain_ratio(*n)
            );
            rows.push((*n, m));
        }

        let (n0, first) = &rows[0];
        let (n1, last) = &rows[rows.len() - 1];
        assert_eq!(*n0, 1, "the shallow end of the sweep must be a single frame");

        let ratio = last.sky_grain / first.sky_grain;
        let expected = expected_grain_ratio(*n1);
        if !denoise {
            // The tone curve alone, so the exponent is the curve's and this is its
            // contract: a deeper stack is rendered calmer. Loose bounds, because a real
            // session's sigma does not fall as sqrt(N) — rejection, drift and a sky
            // that changes all leave the curve less depth to spend than the ideal.
            assert!(
                ratio < expected * 1.6,
                "{label}: {n1} subs only reached {ratio:.2}x the single-sub grain, \
                 expected about {expected:.2}x"
            );
            assert!(
                ratio > expected * 0.5,
                "{label}: {n1} subs reached {ratio:.2}x the single-sub grain against an \
                 expected {expected:.2}x — the sky is being flattened harder than the \
                 depth pays for, which comes out of the target"
            );

            // And still brighter against that grain, or the trade was a loss.
            let snr_gain = (last.target_contrast / last.sky_grain)
                / (first.target_contrast / first.sky_grain);
            assert!(
                snr_gain > 2.0,
                "{label}: target-to-grain only rose {snr_gain:.1}x over {n1} subs"
            );
        } else {
            // With the filters on, neither bound above means what it says. The wavelet
            // holds the sky near its floor from the *first* sub — 1.41 output levels at
            // N=1 on the 106-sub set against 5.70 with it off — so there is almost
            // nothing left for depth to take, and what remains drifts *up* as the
            // stack's residual noise migrates to the coarse scales a 4-level transform
            // only partly reaches (1.41 -> 1.66 over 106 subs). Normalising against
            // N=1 is misleading for the same reason: the ratio starts from the filter's
            // best case.
            //
            // So this half asserts the absolute state instead, which is what an
            // observer sees: the sky stays smooth at every depth, and the target grows.
            assert!(
                rows.iter().all(|(_, m)| m.sky_grain <= 2.5),
                "{label}: sky grain reached {:.2} output levels with denoising on — the \
                 filters are no longer holding the sky, and the curve is not going to \
                 take it back",
                rows.iter().map(|(_, m)| m.sky_grain).fold(0.0, f64::max)
            );
            assert!(
                last.target_contrast > first.target_contrast * 2.0,
                "{label}: the target only grew {:.1}x over {n1} subs ({:.0} -> {:.0} \
                 levels) — depth is not reaching it",
                last.target_contrast / first.target_contrast,
                first.target_contrast,
                last.target_contrast
            );
        }

        // The target is what the depth is *for*, so it must not be spent down to buy the
        // sky. The synthetic sweep asserts it rises outright; a real session gets a band,
        // because its sigma does not fall as sqrt(N) and the measurement is a median of a
        // 96 px block quantised to whole output levels.
        //
        // This is what the gain running past the depth a session pays for looks like: at
        // `MAX_GAIN_DEPTH` 256 the 106-sub set peaked at 32 subs and gave back 77 -> 69
        // levels, 10 %, by the end.
        // Against the best *so far*, not against the sweep's maximum: while contrast is
        // still climbing every earlier depth is below the last one, which is the whole
        // point. What must not happen is a depth giving back what a shallower one had.
        //
        // In levels rather than as a share: a median of a 96 px block is quantised to
        // whole output levels, which is 1-2 % of the numbers here, so a percentage
        // bound wide enough to absorb one level is also wide enough to absorb the
        // defect. `MAX_GAIN_DEPTH` at 256 gives back 4 levels on this set (10 % on the
        // uncropped session); the cap at 64 gives back none on either.
        let mut best = 0.0f64;
        let mut worst_n = 0usize;
        let mut worst_drop = 0.0f64;
        for (n, m) in &rows {
            if best - m.target_contrast > worst_drop {
                worst_drop = best - m.target_contrast;
                worst_n = *n;
            }
            best = best.max(m.target_contrast);
        }
        println!(
            "  target contrast peaked at {best:.0} levels, worst give-back \
             {worst_drop:.0} levels at {worst_n} subs"
        );
        assert!(
            worst_drop <= 2.0,
            "{label}: rendered target contrast fell {worst_drop:.0} levels below a \
             shallower stack's by {worst_n} subs — the curve is spending more depth on \
             the sky than the stack delivers"
        );

        // Stability: the sky must not jump between one stack update and the next. In a
        // dark eyepiece a few levels of background step reads as the field pumping.
        let worst = rows
            .windows(2)
            .map(|p| (p[1].1.sky_level - p[0].1.sky_level).abs())
            .fold(0.0f64, f64::max);
        println!("  worst sky-level step between updates: {worst:.0} levels");
        assert!(
            worst <= 4.0,
            "{label}: the sky level jumped {worst:.0} output levels between two stack \
             updates — the background pumps as the stack deepens"
        );
    }
}

/// The bundled fixture set, so this runs in CI.
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_real_session_gets_calmer_with_depth() {
    let files = managed_session(FIXTURE_SET);
    measure_real_session(FIXTURE_SET, &files, &[1, 2, 4, 8, 16, 32]);
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
    measure_real_session(DEEP_SET, &files, &[1, 2, 4, 8, 16, 32, 64, 106]);
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
