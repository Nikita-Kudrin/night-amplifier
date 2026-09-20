//! Diagnostic, not a regression guard: renders a real session at several stack
//! depths with background extraction and denoising switched independently, so the
//! stage that introduces coarse sky blotches can be isolated by comparing outputs.
//!
//! Frames go through the capture path's raw-CFA stage (`build_cfa_pipeline`: hot
//! pixels, row/column FPN) and demosaic before stacking. Skipping it leaves every hot
//! pixel in, and registration drags each one into a dotted trail along the drift.
//!
//! Driven by environment, since the sessions live outside the repo:
//! - `STACK_DIAG_SET`  directory of FITS subs (required, otherwise the test skips)
//! - `STACK_DIAG_OUT`  where PNGs and the linear stack go (required)
//! - `STACK_DIAG_MAX`  frames to stack, default 128
//! - `STACK_DIAG_STEP` take every n-th file, default 1

use std::path::Path;

use serial_test::serial;

use crate::integration::image_loading::load_image;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// `stack_depth` is not bookkeeping here: the stretch spends it on how calm the sky is
/// (`render::autostretch::depth_grain_gain`), so a diagnostic that left it at 1 would
/// render a deep stack with a single-frame tone curve and stop being a picture of what
/// the observer saw — which is the only thing this file is for.
fn render(
    mut frame: night_amplifier::Frame,
    settings: &night_amplifier::server::state::CaptureSettings,
    stack_depth: u32,
    tweak: impl FnOnce(&mut night_amplifier::render::DenoiseConfig),
) -> (Vec<u8>, u32, u32) {
    use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis};
    let night_amplifier::server::capture::pipeline::PreviewRender {
        mut pipeline_config,
        stretch_result,
        ..
    } =
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
    tweak(&mut pipeline_config.denoise);
    let ready = night_amplifier::server::state::RenderReadyFrame {
        noise: None,
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    };
    night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 2560, 1440).unwrap()
}

/// The stack at half resolution, planar f32 little-endian, for offline analysis.
fn save_linear(frame: &night_amplifier::Frame, path: &str) {
    let (w, h, c) = (frame.width() / 2, frame.height() / 2, frame.channels());
    let mut bytes = Vec::with_capacity(w * h * c * 4);
    for ch in 0..c {
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    sum += frame.get_pixel(2 * x + dx, 2 * y + dy, ch);
                }
                bytes.extend_from_slice(&(sum / 4.0).to_le_bytes());
            }
        }
    }
    std::fs::write(format!("{path}_{w}x{h}x{c}.f32"), bytes).unwrap();
}

#[test]
#[serial]
#[ignore = "diagnostic - set STACK_DIAG_SET and STACK_DIAG_OUT, run with --ignored"]
fn render_session_by_stage_for_blotch_diagnosis() {
    use night_amplifier::server::capture::pipeline::{build_cfa_pipeline, debayer_algorithm};
    use night_amplifier::server::capture::StackingContext;

    let (Ok(set), Ok(out)) = (std::env::var("STACK_DIAG_SET"), std::env::var("STACK_DIAG_OUT")) else {
        println!("STACK_DIAG_SET / STACK_DIAG_OUT not set. Skipping.");
        return;
    };
    let max = env_usize("STACK_DIAG_MAX", 128);
    let step = env_usize("STACK_DIAG_STEP", 1).max(1);

    let mut files: Vec<_> = std::fs::read_dir(&set)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "fits" || e == "fit"))
        .collect();
    files.sort();
    let files: Vec<_> = files.into_iter().step_by(step).take(max).collect();

    let settings = night_amplifier::server::state::CaptureSettings::default();
    let cfa_pipeline = build_cfa_pipeline(&settings);
    let algorithm = debayer_algorithm(&settings);
    println!("{set}: {} frames, raw stages {:?}, {algorithm:?}", files.len(), cfa_pipeline.stage_names());

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
    let mut ctx =
        StackingContext::new(reference.width(), reference.height(), reference.channels(), &settings).unwrap();
    ctx.initialize_with_reference(&reference).unwrap();
    drop(reference);

    let emit = |ctx: &StackingContext| {
        let n = ctx.frame_count();
        let stack = ctx.compute().unwrap();
        type Tweak = fn(&mut night_amplifier::render::DenoiseConfig);
        let variants: [(&str, bool, bool, Tweak); 9] = [
            ("default", true, true, |_| {}),
            ("nodenoise", true, false, |_| {}),
            ("nobg_nodenoise", false, false, |_| {}),
            ("luma_only", true, true, |d| d.chroma.enabled = false),
            ("chroma_k1", true, true, |d| { d.luma.enabled = false; d.chroma.noise_k = 1.0; }),
            ("chroma_k2", true, true, |d| { d.luma.enabled = false; d.chroma.noise_k = 2.0; }),
            ("chroma_k3", true, true, |d| { d.luma.enabled = false; d.chroma.noise_k = 3.0; }),
            ("chroma_k5", true, true, |d| { d.luma.enabled = false; d.chroma.noise_k = 5.0; }),
            ("chroma_k8", true, true, |d| { d.luma.enabled = false; d.chroma.noise_k = 8.0; }),
        ];
        for (tag, background, denoise, tweak) in variants {
            let mut s = settings.clone();
            s.background_subtraction = background;
            s.denoise.chroma = denoise;
            s.denoise.luma_strength = if denoise { 1.0 } else { 0.0 };
            let (rgb8, w, h) = render(stack.clone(), &s, n as u32, tweak);
            image::save_buffer(format!("{out}/n{n:04}_{tag}.png"), &rgb8, w, h, image::ColorType::Rgb8).unwrap();
        }
        println!("  rendered depth {n}");
        stack
    };

    let mut emitted = vec![];
    for path in files.iter().skip(1) {
        if [1usize, 8].contains(&ctx.frame_count()) && !emitted.contains(&ctx.frame_count()) {
            emitted.push(ctx.frame_count());
            emit(&ctx);
        }
        let _ = ctx.add_frame(&load(path));
    }
    let deepest = emit(&ctx);
    save_linear(&deepest, &format!("{out}/n{:04}_linear", ctx.frame_count()));
}
