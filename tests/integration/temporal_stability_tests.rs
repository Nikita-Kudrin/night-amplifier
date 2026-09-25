//! How much the rendered picture moves from frame to frame when the sky does not.
//!
//! Replays a real session's subs through the production preview path and encoder with one
//! `PreviewAnalysis` held across frames, as the render task holds it: live view (each sub
//! on its own) and stacked view (the running stack at every depth, rendered through the
//! held analysis *and* a fresh one). The fresh render is what the cache must reproduce;
//! the gap between them is the cache's own contribution to what the observer sees.
//!
//! This found the stacked view's pulse — the target sagging between cache refreshes and
//! jumping at each, 5 output levels on M27 — and Orion's 29-level jump where its signal
//! fraction crossed 0.2. The guard is `capture::analysis::stability_tests`; this is the
//! real-data diagnostic behind it.
//!
//! `TEMPORAL_STABILITY_FRAMES` (default 40) caps the subs per session, sessions follow
//! `RENDER_BRIGHTNESS_SETS`, and `TEMPORAL_STABILITY_ROWS` prints every frame's solve
//! inputs (captured from the solver's `Auto-stretch inputs` debug event).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serial_test::serial;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::Layer;

use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis, StackingContext};
use night_amplifier::server::state::CaptureSettings;
use night_amplifier::Frame;

use crate::integration::instruments::{load_sub, requested_sessions, session_frames};

/// The solver's inputs for one frame, from its debug event.
#[derive(Debug, Clone, Copy, Default)]
struct Solve {
    mean_sigma: f64,
    signal_fraction: f64,
    adaptive_sigma: f64,
}

#[derive(Default)]
struct SolveVisitor(Solve, bool);

impl Visit for SolveVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        match field.name() {
            "mean_sigma" => self.0.mean_sigma = value,
            "signal_fraction" => self.0.signal_fraction = value,
            "adaptive_sigma" => self.0.adaptive_sigma = value,
            _ => {}
        }
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.1 = format!("{value:?}").contains("Auto-stretch inputs");
        }
    }
}

/// Keeps the last solve seen.
#[derive(Clone, Default)]
struct LastSolve(Arc<Mutex<Solve>>);

impl<S: tracing::Subscriber> Layer<S> for LastSolve {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = SolveVisitor::default();
        event.record(&mut visitor);
        if visitor.1 {
            *self.0.lock().unwrap() = visitor.0;
        }
    }
}

/// What the observer sees of one frame, in output levels.
#[derive(Debug, Clone, Copy)]
struct Seen {
    sky: [f64; 3],
    target: f64,
    grain: f64,
}

/// Where the sky and the target are, found once on the first render so every frame is
/// measured on the same pixels. From a 17 px mean, so a pixel's own noise cannot select it.
struct Regions {
    width: usize,
    height: usize,
    sky: Vec<usize>,
    target: Vec<usize>,
}

impl Regions {
    fn find(rgb8: &[u8], width: usize, height: usize) -> Self {
        let blurred = box_mean(&green(rgb8), width, height, 8);
        let mut sorted = blurred.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let (sky_below, target_above) =
            (sorted[sorted.len() * 4 / 10], sorted[sorted.len() * 97 / 100]);
        // Off the outer twentieth, where registration borders and coverage live.
        let (mx, my) = (width / 20, height / 20);
        let (mut sky, mut target) = (Vec::new(), Vec::new());
        for y in my..height - my {
            for x in mx..width - mx {
                let i = y * width + x;
                if blurred[i] < sky_below {
                    sky.push(i);
                } else if blurred[i] > target_above {
                    target.push(i);
                }
            }
        }
        Self { width, height, sky, target }
    }

    fn see(&self, rgb8: &[u8]) -> Seen {
        let mean = |idx: &[usize], c: usize| {
            idx.iter().map(|&i| rgb8[i * 3 + c] as f64).sum::<f64>() / idx.len().max(1) as f64
        };
        // Robust sigma of the sky against a 9 px local mean: the grain, not the gradient.
        let g = green(rgb8);
        let local = box_mean(&g, self.width, self.height, 4);
        let mut residual: Vec<f64> = self.sky.iter().map(|&i| g[i] - local[i]).collect();
        let centre = median(&mut residual);
        let mut deviation: Vec<f64> = residual.iter().map(|v| (v - centre).abs()).collect();
        Seen {
            sky: [mean(&self.sky, 0), mean(&self.sky, 1), mean(&self.sky, 2)],
            target: mean(&self.target, 1),
            grain: median(&mut deviation) * 1.4826,
        }
    }
}

fn green(rgb8: &[u8]) -> Vec<f64> {
    rgb8.chunks_exact(3).map(|p| p[1] as f64).collect()
}

fn median(values: &mut [f64]) -> f64 {
    let mid = values.len() / 2;
    *values.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1
}

/// Separable box mean of half-width `r`, edge-clamped.
fn box_mean(plane: &[f64], w: usize, h: usize, r: usize) -> Vec<f64> {
    let mut rows = vec![0.0; plane.len()];
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            rows[y * w + x] = plane[y * w + a..=y * w + b].iter().sum::<f64>() / (b - a + 1) as f64;
        }
    }
    let mut out = vec![0.0; plane.len()];
    for y in 0..h {
        let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
        for x in 0..w {
            out[y * w + x] = (a..=b).map(|yy| rows[yy * w + x]).sum::<f64>() / (b - a + 1) as f64;
        }
    }
    out
}

/// Render one frame through the production path at the eyepiece's 1440 box.
fn render(
    mut frame: Frame,
    ctx: AnalysisContext,
    analysis: &mut PreviewAnalysis,
    settings: &CaptureSettings,
) -> (Vec<u8>, usize, usize) {
    let rendered = night_amplifier::server::capture::pipeline::process_preview_frame_with_analysis(
        &mut frame, settings, ctx, analysis,
    )
    .unwrap();
    let ready = night_amplifier::server::state::RenderReadyFrame {
        noise: None,
        linear_frame: Arc::new(frame),
        pipeline_config: rendered.pipeline_config,
        stretch_result: rendered.stretch_result,
    };
    let (bytes, w, h) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, 1440, 1440).unwrap();
    (bytes, w as usize, h as usize)
}

/// Subs as the capture path hands them to the stacking task: raw-CFA stage, then demosaic.
/// Loaded one at a time — a session held in memory is 4 GB of IMX533 frames.
struct Subs {
    files: Vec<PathBuf>,
    settings: CaptureSettings,
    pattern: Option<night_amplifier::debayer::CfaPattern>,
}

impl Subs {
    fn get(&mut self, index: usize) -> Frame {
        use night_amplifier::server::capture::pipeline::{build_cfa_pipeline, debayer_algorithm};
        let sub = load_sub(&self.files[index]);
        if !sub.is_bayer {
            return sub.frame;
        }
        let pattern = *self.pattern.get_or_insert_with(|| {
            night_amplifier::debayer::detect_cfa_pattern(&sub.frame).unwrap().pattern
        });
        let mut cfa = night_amplifier::CfaFrame::mosaic(sub.frame, pattern).unwrap();
        build_cfa_pipeline(&self.settings).apply(&mut cfa);
        cfa.debayer(debayer_algorithm(&self.settings)).unwrap()
    }
}

struct Row {
    depth: u32,
    solve: Solve,
    held: Seen,
    fresh: Option<Seen>,
}

fn print_rows(rows: &[Row]) {
    println!("  depth     sigma  sfrac  adsig    skyR   skyG   skyB  target  grain   fresh");
    for r in rows {
        let fresh = r.fresh.map(|f| format!("{:>7.1}", f.target)).unwrap_or_default();
        println!(
            "  {:>5} {:.7} {:.4} {:>5.2}  {:>6.2} {:>6.2} {:>6.2} {:>7.1} {:>6.2} {fresh}",
            r.depth,
            r.solve.mean_sigma,
            r.solve.signal_fraction,
            r.solve.adaptive_sigma,
            r.held.sky[0],
            r.held.sky[1],
            r.held.sky[2],
            r.held.target,
            r.held.grain,
        );
    }
}

fn summarise(label: &str, rows: &[Row]) {
    let mut sky = [0.0f64; 3];
    let (mut rise, mut dip, mut grain_rise) = (0.0f64, 0.0f64, 0.0f64);
    for pair in rows.windows(2) {
        for (c, worst) in sky.iter_mut().enumerate() {
            *worst = worst.max((pair[1].held.sky[c] - pair[0].held.sky[c]).abs());
        }
        let step = pair[1].held.target - pair[0].held.target;
        rise = rise.max(step);
        dip = dip.min(step);
        grain_rise = grain_rise.max(pair[1].held.grain - pair[0].held.grain);
    }
    let versus_fresh = rows
        .iter()
        .filter_map(|r| r.fresh.map(|f| (r.held.target - f.target).abs()))
        .fold(0.0f64, f64::max);
    println!(
        "  {label}: sky step max R/G/B {:.2}/{:.2}/{:.2}  target step {dip:+.2}..{rise:+.2}  \
         grain rise max {grain_rise:+.2}  held vs fresh target max {versus_fresh:.2}",
        sky[0], sky[1], sky[2]
    );
}

/// One view of a session: each frame with its context, rendered through the held
/// analysis and, when `fresh`, a second time through a fresh one.
fn measure(
    frames: impl Iterator<Item = (Frame, AnalysisContext)>,
    fresh: bool,
    last_solve: &LastSolve,
) -> Vec<Row> {
    let settings = CaptureSettings::default();
    let mut analysis = PreviewAnalysis::new();
    let mut regions: Option<Regions> = None;
    let mut rows = Vec::new();
    for (frame, ctx) in frames {
        let fresh_seen = fresh.then(|| frame.clone()).map(|copy| {
            let (rgb8, w, h) = render(copy, ctx, &mut PreviewAnalysis::new(), &settings);
            regions.get_or_insert_with(|| Regions::find(&rgb8, w, h)).see(&rgb8)
        });
        let (rgb8, w, h) = render(frame, ctx, &mut analysis, &settings);
        let held = regions.get_or_insert_with(|| Regions::find(&rgb8, w, h)).see(&rgb8);
        rows.push(Row {
            depth: ctx.stack_depth,
            solve: *last_solve.0.lock().unwrap(),
            held,
            fresh: fresh_seen,
        });
    }
    rows
}

#[test]
#[serial]
#[ignore = "diagnostic - the four sessions live outside the repo; run with --ignored"]
fn measure_temporal_stability_on_real_sessions() {
    let limit = std::env::var("TEMPORAL_STABILITY_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(40usize);
    let rows_wanted = std::env::var("TEMPORAL_STABILITY_ROWS").is_ok();
    let last_solve = LastSolve::default();
    let _subscriber =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(last_solve.clone()));

    for (name, dir) in requested_sessions() {
        let Some(files) = session_frames(Path::new(dir)) else {
            println!("  {name}: not on this machine, skipped");
            continue;
        };
        let files: Vec<PathBuf> = files.into_iter().take(limit).collect();
        let count = files.len();
        println!("\n=== {name}: {count} subs ===");
        let mut subs = Subs {
            files,
            settings: CaptureSettings::default(),
            pattern: None,
        };

        let live = (0..count).map(|i| (subs.get(i), AnalysisContext::ONE_SHOT));
        let rows = measure(live, false, &last_solve);
        if rows_wanted {
            print_rows(&rows);
        }
        summarise("live view", &rows);

        let reference = subs.get(0);
        let mut stack = StackingContext::new(
            reference.width(),
            reference.height(),
            reference.channels(),
            &subs.settings,
        )
        .unwrap();
        stack.initialize_with_reference(&reference).unwrap();
        drop(reference);
        let mut next = 0;
        let stacked = std::iter::from_fn(|| {
            if next == count {
                return None;
            }
            if next > 0 {
                let _ = stack.add_frame(&subs.get(next));
            }
            next += 1;
            let ctx = AnalysisContext {
                showing_stack: true,
                stack_depth: stack.frame_count() as u32,
            };
            Some((stack.compute().unwrap(), ctx))
        });
        let rows = measure(stacked, true, &last_solve);
        if rows_wanted {
            print_rows(&rows);
        }
        summarise("stacked view", &rows);
    }
}
