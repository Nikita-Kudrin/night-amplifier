//! The measurement instruments, shared by both repos.
//!
//! **Nothing in this file may name `crate::`.** Pro's integration tests pull it in with
//! `#[path = "../../night-amplifier/tests/integration/instruments.rs"] mod instruments;`, as
//! they do `common.rs`, which works only while it is reachable from either crate's root. So
//! fixture discovery arrives as a parameter instead of through `common`.
//!
//! Ten instruments, each because a single number misled a real fix (each one's doc says how):
//!
//! 1. Octave-band sky noise ([`octave_band_sigma`]): one grain figure misled three changes.
//! 2. The star radial profile ([`radial_excess`]): the only one that catches ringing.
//! 3. Centre against edge ([`centre_and_edge`]): a whole frame averages the border away.
//! 4. Line coherence ([`line_coherence`]): banding far below the grain floor, which no RMS sees.
//! 5. Lattice lines ([`lattice_lines`]): a dither tile's spectral lines, removed, not moved.
//! 6. Block-mean error ([`block_mean_error`]): whether an encoder kept what a dither carried.
//! 7. Target structure ([`object_structure`]): texture, which a median at three radii misses.
//! 8. Star response ([`star_response`]): a star on the target rings where an isolated one doesn't.
//! 9. Target signal and noise ([`signal_and_noise`]): 7's structure told apart from grain on it.
//! 10. Star size ([`half_flux_radius`]): FWHM read 2.3 px on stars holding half their flux in 3.7.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use night_amplifier::statistics::MAD_TO_SIGMA;
use night_amplifier::Frame;

#[path = "replay_cache.rs"]
pub mod replay_cache;

/// One sub as it comes off the disk, before the raw-CFA stage.
///
/// The caller's loader decides how — FITS, TIFF, whatever the fixture set holds — and
/// says whether the frame is still mosaiced.
pub struct RawSub {
    pub frame: Frame,
    pub is_bayer: bool,
}

/// Load one sub from disk: FITS, or one of the PNG/TIFF fixture sets.
///
/// The same readers `integration::image_loading` uses, minus everything about fixture
/// discovery — a loader here may not name `crate::`, and a loader that disagrees with the
/// application is not testing the application.
pub fn load_sub(path: &Path) -> RawSub {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext == "fits" || ext == "fit" {
        let frame = night_amplifier::fits::read_frame(path)
            .unwrap_or_else(|e| panic!("failed to load FITS {path:?}: {e}"));
        // Greyscale FITS from an astronomy camera is undebayered CFA data.
        let is_bayer = frame.channels() == 1;
        return RawSub { frame, is_bayer };
    }

    let img = image::open(path).unwrap_or_else(|e| panic!("failed to open {path:?}: {e}"));
    let (width, height) = (img.width() as usize, img.height() as usize);
    let (bytes, format, channels, is_bayer) = match img {
        image::DynamicImage::ImageLuma16(g) => (
            g.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect::<Vec<u8>>(),
            night_amplifier::PixelFormat::Bayer16,
            1,
            true,
        ),
        image::DynamicImage::ImageLuma8(g) => {
            (g.into_raw(), night_amplifier::PixelFormat::Bayer8, 1, true)
        }
        image::DynamicImage::ImageRgb16(rgb) => (
            rgb.as_raw().iter().flat_map(|&v| v.to_le_bytes()).collect::<Vec<u8>>(),
            night_amplifier::PixelFormat::Rgb16,
            3,
            false,
        ),
        other => (
            other.to_rgb8().into_raw(),
            night_amplifier::PixelFormat::Rgb8,
            3,
            false,
        ),
    };
    let frame = night_amplifier::Frame::from_raw(&bytes, width, height, channels, format)
        .unwrap_or_else(|e| panic!("failed to build a frame from {path:?}: {e}"));
    RawSub { frame, is_bayer }
}

// ---------------------------------------------------------------------------
// Stacking a real session
// ---------------------------------------------------------------------------

/// FITS files of a session directory, sorted, or `None` when it is not on this machine.
pub fn session_files(dir: &Path) -> Option<Vec<PathBuf>> {
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

// ---------------------------------------------------------------------------
// Replaying a session through the application
// ---------------------------------------------------------------------------

/// A fixture directory replayed the way the application runs a simulator session.
///
/// The simulated camera captures, `convert_captured_frame` runs the raw-CFA stage and the
/// demosaic, and `stack_frame` offers the result to the live stack: the three calls the
/// capture and stacking tasks make, so whatever a test reads here is production's verdict,
/// not a re-implementation of it. The one departure is a 1 µs exposure, so the camera does
/// not sleep through the real one.
pub struct Replay {
    camera: night_amplifier::camera::SimulatedCamera,
    capture: night_amplifier::camera::CaptureConfig,
    settings: night_amplifier::session::state::CaptureSettings,
    plugins: night_amplifier::plugins::Plugins,
    cfa_pipeline: night_amplifier::cfa::CfaPipeline,
    algorithm: night_amplifier::DebayerAlgorithm,
    stacker: Option<Box<dyn night_amplifier::session::capture::LiveStacker>>,
    stacking_failed: bool,
    captured: usize,
    /// Overrides the subs a deep-sky stack holds before judging them (`StackSettings`);
    /// `Some(0)` replays the stack as it streamed before probation existed.
    probation: Option<usize>,
}

impl Replay {
    /// A deep-sky stacking session at the shipped settings, with the installed plugins.
    pub fn open(dir: &Path) -> Self {
        Self::with_settings(dir, Self::shipped_settings())
    }

    fn shipped_settings() -> night_amplifier::session::state::CaptureSettings {
        night_amplifier::session::state::CaptureSettings { stacking: true, ..Default::default() }
    }

    /// Plays every sub and keeps what a test reads of the session: no display copies, of
    /// which the reference's (a whole sub) was half of every stored replay.
    pub fn played(mut self) -> ReplayRecord {
        let mut outcomes = self.run();
        outcomes.iter_mut().for_each(|outcome| outcome.display_frame = None);
        let (stack, coverage) = match self.depth() {
            0 => (None, None),
            _ => {
                let (stack, coverage) = self.snapshot_with_coverage();
                (Some(stack), coverage)
            }
        };
        ReplayRecord { subs: self.subs(), depth: self.depth(), outcomes, stack, coverage }
    }

    pub fn with_settings(dir: &Path, settings: night_amplifier::session::state::CaptureSettings) -> Self {
        use night_amplifier::session::capture::pipeline::{build_cfa_pipeline, debayer_algorithm};
        let camera = night_amplifier::camera::SimulatedCamera::new(dir.to_path_buf())
            .unwrap_or_else(|e| panic!("cannot replay {dir:?}: {e}"));
        let capture = settings.to_capture_config().with_exposure_us(1);
        Self {
            camera,
            capture,
            cfa_pipeline: build_cfa_pipeline(&settings),
            algorithm: debayer_algorithm(&settings),
            plugins: night_amplifier::plugins::Plugins::installed(),
            settings,
            stacker: None,
            stacking_failed: false,
            captured: 0,
            probation: None,
        }
    }

    /// The same replay with the stack's probation held to `subs` (0: none).
    pub fn with_probation(mut self, subs: usize) -> Self {
        self.probation = Some(subs);
        self
    }

    /// Subs in the session, the reference included.
    pub fn subs(&self) -> usize {
        self.camera.frame_count()
    }

    /// The next sub as the stacking task receives it; `None` once every sub has been
    /// played (the camera itself would wrap to the first again).
    pub fn capture(&mut self) -> Option<Frame> {
        use night_amplifier::camera::Camera;
        if self.captured == self.subs() {
            return None;
        }
        self.captured += 1;
        let raw = self.camera.capture(&self.capture).expect("the simulator captures");
        let frame = night_amplifier::session::capture::pipeline::convert_captured_frame(
            &raw,
            self.camera.info(),
            &self.cfa_pipeline,
            self.algorithm,
        )
        .expect("a replayed sub converts");
        Some(frame)
    }

    /// Offers `frame` to the session's stack, as the stacking task does.
    pub fn stack(
        &mut self,
        frame: &Frame,
        want_display: bool,
    ) -> night_amplifier::session::capture::pipeline::StackingOutcome {
        let mut stack = night_amplifier::session::capture::StackSettings::of(&self.settings, &self.plugins);
        if let Some(subs) = self.probation {
            stack.probation_subs = subs;
        }
        night_amplifier::session::capture::pipeline::stack_frame_with(
            frame,
            self.settings.stacking_type,
            &stack,
            &mut self.stacker,
            &mut self.stacking_failed,
            want_display,
        )
    }

    /// Captures the next sub and stacks it, without a display copy.
    pub fn step(&mut self) -> Option<night_amplifier::session::capture::pipeline::StackingOutcome> {
        let frame = self.capture()?;
        Some(self.stack(&frame, false))
    }

    /// Plays every remaining sub, handing back each outcome in order (the reference's first).
    pub fn run(&mut self) -> Vec<night_amplifier::session::capture::pipeline::StackingOutcome> {
        std::iter::from_fn(|| self.step()).collect()
    }

    /// Frames in the stack, the reference included.
    pub fn depth(&self) -> usize {
        self.stacker.as_ref().map_or(0, |s| s.depth())
    }

    /// The stack as the render task would receive it.
    pub fn snapshot(&self) -> Frame {
        self.snapshot_with_coverage().0
    }

    /// [`Self::snapshot`] with the coverage map the stacking task sends beside it (`None`:
    /// every sub covered all of it).
    pub fn snapshot_with_coverage(&self) -> (Frame, Option<night_amplifier::NoiseField>) {
        self.stacker
            .as_ref()
            .expect("a stack exists once a reference has been taken")
            .snapshot()
            .expect("the stack reads")
    }
}

/// A session [`Replay::open`] played to the end: [`Replay::played`] as values.
pub struct ReplayRecord {
    /// Subs in the session, the reference included.
    pub subs: usize,
    /// Frames in the final stack.
    pub depth: usize,
    /// Every sub's outcome in order, the reference's first, each without `display_frame`.
    pub outcomes: Vec<night_amplifier::session::capture::pipeline::StackingOutcome>,
    /// `None` where no sub could be the reference.
    pub stack: Option<Frame>,
    /// The stack's coverage map: `None` where every sub covered all of it, or no stack.
    pub coverage: Option<night_amplifier::NoiseField>,
}

/// [`Replay::open`] played to the end once per test run: the first test replaying `dir`
/// stores it and the others read it back ([`replay_cache`]). A test that steps the replay
/// or feeds it frames of its own opens a [`Replay`] instead.
pub fn replayed(dir: &Path) -> ReplayRecord {
    let key = replay_cache::Key::new("replay")
        .files(&files_under(dir))
        .settings(&Replay::shipped_settings())
        .plugins(&night_amplifier::plugins::Plugins::installed());
    let label = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
    replay_cache::cached(&key, &label, || Replay::open(dir).played())
}

/// Every file below `dir`, sorted: whatever the simulated camera may read.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for path in std::fs::read_dir(&dir).into_iter().flatten().filter_map(|e| e.ok().map(|e| e.path())) {
            if path.is_dir() { pending.push(path) } else { files.push(path) }
        }
    }
    files.sort();
    files
}

/// Stacks a real session, handing back `(depth, stack)` at each requested depth.
///
/// Frames go through the capture path's raw-CFA stage (hot pixels, row/column FPN) and
/// demosaic, as the capture task runs them: a bare debayer leaves every hot pixel in, and
/// registration drags each one into a dotted trail along the session's drift. The stacks
/// are [`accumulator_snapshots`]' frames, the same running mean `compute` reads.
pub fn stack_snapshots(files: &[PathBuf], depths: &[usize]) -> Vec<(usize, Frame)> {
    accumulator_snapshots(files, depths).into_iter().map(|s| (s.depth, s.frame)).collect()
}

/// One depth of a stack, with everything the accumulator knows about it.
pub struct AccumulatorSnapshot {
    pub depth: usize,
    pub frame: Frame,
    /// What the render task carries: coverage only — see `MasterStack::compute_with_coverage`.
    pub noise: night_amplifier::NoiseField,
    /// The whole measurement, variance included, which only diagnostics ask for.
    pub measured: night_amplifier::NoiseField,
    /// `count / frame_count` per pixel: how much of the stack each pixel really holds.
    pub coverage: Frame,
    /// `StackedFrame::stack_generation`: the stack's rebuilds so far (a probation's commit).
    pub generation: u64,
}

/// The accumulator at each requested depth: the display copy, the field the render task
/// would carry, the full measurement and the per-pixel coverage.
///
/// Stacked once per test run for each `files` and `depths` ([`replay_cache`]), which is
/// why the subs are always [`load_sub`]'s: a loader cannot be part of a key.
pub fn accumulator_snapshots(files: &[PathBuf], depths: &[usize]) -> Vec<AccumulatorSnapshot> {
    let key = replay_cache::Key::new("stack")
        .files(files)
        .with("depths", format!("{depths:?}"))
        .settings(&night_amplifier::session::state::CaptureSettings::default())
        .plugins(&night_amplifier::plugins::Plugins::installed());
    let label = files
        .first()
        .and_then(|f| f.parent()?.file_name())
        .map_or_else(|| "stack".to_string(), |n| n.to_string_lossy().into_owned());
    replay_cache::cached(&key, &label, || {
        let mut out = Vec::new();
        for_each_snapshot(files, depths, &load_sub, |snapshot| out.push(snapshot));
        out
    })
}

/// [`accumulator_snapshots`] handed to `visit` one depth at a time and then dropped: a sweep
/// over every depth of a 64-sub 3008² session would otherwise hold 7 GB of stacks.
pub fn for_each_snapshot(
    files: &[PathBuf],
    depths: &[usize],
    load: &dyn Fn(&Path) -> RawSub,
    mut visit: impl FnMut(AccumulatorSnapshot),
) {
    with_stack(files, depths, load, |ctx, generation| {
        let (frame, noise) = ctx.compute_with_coverage().unwrap();
        visit(AccumulatorSnapshot {
            depth: ctx.frame_count(),
            frame,
            noise,
            measured: ctx.stacker.noise_field(),
            coverage: ctx.stacker.coverage_map(),
            generation,
        });
    });
}

/// Drives a `StackingContext` through the calls `stack_frame` makes (`set_reference`,
/// `apply_settings`, `offer`), holding the concrete context so `snapshot` can read the
/// accumulator diagnostics the `LiveStacker` trait hides. Subs come from `load` rather
/// than the simulated camera so callers can stack any subset of a session.
fn with_stack<T>(
    files: &[PathBuf],
    depths: &[usize],
    load: &dyn Fn(&Path) -> RawSub,
    mut snapshot: impl FnMut(&night_amplifier::session::capture::StackingContext, u64) -> T,
) -> Vec<(usize, T)> {
    use night_amplifier::session::capture::pipeline::{
        build_cfa_pipeline, correct_and_demosaic, debayer_algorithm,
    };
    use night_amplifier::session::capture::{LiveStacker, StackSettings, StackingContext};

    let settings = night_amplifier::session::state::CaptureSettings::default();
    let stack_settings = StackSettings::of(&settings, &night_amplifier::plugins::Plugins::installed());
    let cfa_pipeline = build_cfa_pipeline(&settings);
    let algorithm = debayer_algorithm(&settings);

    let mut pattern = None;
    let mut prepare = |path: &Path| {
        let img = load(path);
        if !img.is_bayer {
            return img.frame;
        }
        let p = *pattern.get_or_insert_with(|| {
            night_amplifier::debayer::detect_cfa_pattern(&img.frame).unwrap().pattern
        });
        let cfa = night_amplifier::CfaFrame::mosaic(img.frame, p).unwrap();
        correct_and_demosaic(cfa, &cfa_pipeline, algorithm).unwrap()
    };

    let reference = prepare(&files[0]);
    let mut ctx = StackingContext::new(
        reference.width(),
        reference.height(),
        reference.channels(),
        &stack_settings,
    )
    .unwrap();
    ctx.set_reference(&reference).unwrap();
    drop(reference);

    let mut snapshots = Vec::new();
    let mut next = 0;
    let mut generation = 0u64;
    for path in files.iter().skip(1) {
        while next < depths.len() && ctx.frame_count() >= depths[next] {
            snapshots.push((ctx.frame_count(), snapshot(&ctx, generation)));
            next += 1;
        }
        ctx.apply_settings(&stack_settings);
        let offered = ctx.offer(&prepare(path), &stack_settings);
        generation += u64::from(offered.as_ref().is_ok_and(|a| a.restacked.is_some()));
        match offered {
            Ok(a) if !a.added => println!(
                "  {} not stacked: {:?} ({} stars matched, residual {:.1} px)",
                path.display(),
                a.rejected_because,
                a.matched_stars,
                a.mean_residual
            ),
            Err(e) => println!("  {} not stacked: {e}", path.display()),
            Ok(_) => {}
        }
    }
    if snapshots.last().map(|s| s.0) != Some(ctx.frame_count()) {
        snapshots.push((ctx.frame_count(), snapshot(&ctx, generation)));
    }
    snapshots
}

// ---------------------------------------------------------------------------
// Rendering through the production path
// ---------------------------------------------------------------------------

/// Render a stack the way the render task does, and hand back the RGB8 bytes.
///
/// Through the analysis door, not `process_preview_frame`: the stretch spends the
/// stack's depth on how calm the sky is, so a one-shot render would measure a
/// single-frame tone curve over a deep stack.
pub fn render(
    frame: Frame,
    settings: &night_amplifier::session::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
) -> (Vec<u8>, usize, usize) {
    render_with(frame, settings, denoise, max, stack_depth, None, |_| {})
}

/// [`render`], with a last look at the pipeline config and the frame's noise map.
///
/// The denoisers run in the encoder rather than the pipeline, so a caller can still
/// change their configuration after the solve — which is what lets an experiment try a
/// threshold ladder without a per-variant rebuild. `noise` is what the encoder hands the
/// filters; passing `None` is how the global-estimate path stays reachable for an A/B.
pub fn render_with(
    frame: Frame,
    settings: &night_amplifier::session::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
    noise: Option<night_amplifier::NoiseField>,
    tweak: impl FnOnce(&mut night_amplifier::render::RenderPipelineConfig),
) -> (Vec<u8>, usize, usize) {
    let (frame, noise) = cut_for_pipeline(frame, noise, None, settings);
    render_cut(frame, settings, denoise, max, stack_depth, noise, tweak)
}

/// [`render`] unbinned, whatever `preview_resolution` or `PROCESSING_RESOLUTION` asks: for an instrument
/// that must see one picture in every order. The ring test's stars, found on each order's own
/// render, were 8 others of 25 binned first, whose surround read +1.12 levels un-denoised
/// against +0.28 on the Native list (+0.24 at Native).
pub fn render_unbinned(
    frame: Frame,
    settings: &night_amplifier::session::state::CaptureSettings,
    max: (u32, u32),
    stack_depth: u32,
) -> (Vec<u8>, usize, usize) {
    render_cut(frame, settings, false, max, stack_depth, None, |_| {})
}

/// [`render_with`] on a frame already cut as [`cut_for_pipeline`] cuts it.
fn render_cut(
    mut frame: Frame,
    settings: &night_amplifier::session::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
    noise: Option<night_amplifier::NoiseField>,
    tweak: impl FnOnce(&mut night_amplifier::render::RenderPipelineConfig),
) -> (Vec<u8>, usize, usize) {
    use night_amplifier::session::capture::{AnalysisContext, PreviewAnalysis};

    let night_amplifier::session::capture::pipeline::PreviewRender {
        mut pipeline_config,
        stretch_result,
        ..
    } = night_amplifier::session::capture::pipeline::process_preview_frame_with_analysis(
        &mut frame,
        settings,
        &night_amplifier::plugins::Plugins::installed(),
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
    let ready = night_amplifier::render::display::RenderReadyFrame {
        noise: noise.map(std::sync::Arc::new),
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    };
    let (bytes, w, h) =
        night_amplifier::render::display::frame_to_rgb8_downsampled(&ready, max.0, max.1).unwrap();
    (bytes, w as usize, h as usize)
}

/// `frame` and its coverage map as the render task hands them to the pipeline: cut to the
/// auto crop's `rect` and binned first when `settings.preview_resolution` (Processing
/// Resolution) asks, in one pass, the factor resolved on the uncropped size — the render
/// task's own calls. At the default Native and no crop nothing changes, and the stream box
/// alone shrinks the picture, in the encoder, after the stretch.
///
/// `PROCESSING_RESOLUTION` (`4K`, `1440p`, `1080p`) overrides a Native setting, so a whole
/// suite can be rerun in the other order without editing its tests.
pub fn cut_for_pipeline(
    frame: Frame,
    noise: Option<night_amplifier::NoiseField>,
    rect: Option<&night_amplifier::stacking::CropRect>,
    settings: &night_amplifier::session::state::CaptureSettings,
) -> (Frame, Option<night_amplifier::NoiseField>) {
    use night_amplifier::session::capture::stack_crop;
    use night_amplifier::session::state::Resolution;
    let resolution = match settings.preview_resolution {
        Resolution::Native => std::env::var("PROCESSING_RESOLUTION")
            .ok()
            .and_then(|v| [Resolution::Uhd2160, Resolution::Qhd1440, Resolution::Hd1080].into_iter().find(|r| r.label() == v))
            .unwrap_or(Resolution::Native),
        chosen => chosen,
    };
    let bin = night_amplifier::session::capture::preview_bin(frame.width(), frame.height(), resolution);
    let noise = match rect {
        Some(rect) => stack_crop::crop_coverage(noise.as_ref(), rect).unwrap(),
        None => noise,
    };
    let mut shared = std::sync::Arc::new(frame);
    let binned = stack_crop::cut_display_frame(&mut shared, rect, bin).unwrap();
    let noise = if binned { noise.and_then(|field| field.binned(bin).ok()) } else { noise };
    (std::sync::Arc::into_inner(shared).expect("the only handle"), noise)
}

/// Render an accumulator snapshot the way the render task does: with the stack's coverage
/// map attached whenever it has something to say, as production passes it. A guard that
/// renders the mean alone guards a picture no observer sees.
pub fn render_snapshot(
    snapshot: &AccumulatorSnapshot,
    settings: &night_amplifier::session::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
) -> (Vec<u8>, usize, usize) {
    render_with(
        snapshot.frame.clone(),
        settings,
        denoise,
        max,
        snapshot.depth as u32,
        snapshot.noise.is_usable().then(|| snapshot.noise.clone()),
        |_| {},
    )
}

/// FNV-1a over a render's bytes: two renders compare equal by hash only if identical.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// One depth of a [`replay_held`] session, as the render task showed it.
pub struct HeldRender {
    pub depth: usize,
    /// The stack's own size, before any crop or bin.
    pub stack: (usize, usize),
    /// The auto crop the frame was cut to, in stack pixels.
    pub crop: Option<night_amplifier::stacking::CropRect>,
    /// `None` with auto stretch off.
    pub solve: Option<night_amplifier::render::AutoStretchResult>,
    /// The stack's MAD sigma, before the pipeline.
    pub stack_sigma: f32,
    /// One `(rgb8, w, h)` per `denoise` mode at the depths [`replay_held`] encodes, else empty.
    pub renders: Vec<(Vec<u8>, usize, usize)>,
}

impl HeldRender {
    /// Stack pixel `(x, y)` on this depth's render, `w` px wide.
    pub fn to_render(&self, (x, y): (f64, f64), w: usize) -> (f64, f64) {
        let (x0, y0, cut) = self.window();
        let scale = w as f64 / cut as f64;
        ((x - x0) * scale, (y - y0) * scale)
    }

    /// Render pixel `(x, y)`, of a render `w` px wide, in stack pixels.
    pub fn to_stack(&self, (x, y): (f64, f64), w: usize) -> (f64, f64) {
        let (x0, y0, cut) = self.window();
        let scale = cut as f64 / w as f64;
        (x0 + x * scale, y0 + y * scale)
    }

    /// Block `b` of `from`'s render (`from_w` px wide) on this depth's `w x h` render: the
    /// same place on the sky, the same size.
    pub fn block_from(&self, b: PixelBox, from: &HeldRender, from_w: usize, (w, h): (usize, usize)) -> PixelBox {
        let centre = ((b.0 + b.2) as f64 / 2.0, (b.1 + b.3) as f64 / 2.0);
        let (x, y) = self.to_render(from.to_stack(centre, from_w), w);
        box_at((x / w as f64, y / h as f64), w, h)
    }

    fn window(&self) -> (f64, f64, usize) {
        self.crop.map_or((0.0, 0.0, self.stack.0), |r| (r.x as f64, r.y as f64, r.width))
    }
}

/// What [`replay_held`] hands back: each order's depths, and the deepest snapshot.
pub struct Replayed {
    pub orders: Vec<Vec<HeldRender>>,
    pub deepest: AccumulatorSnapshot,
}

/// Every depth of `files` as the render task shows a stack: per order one `PreviewAnalysis`
/// and one `StackCrop` held across depths, the stack's generation noted, every depth solved.
/// A fresh analysis per depth ([`render_snapshot`]) has no cached model, noise trend or held
/// gate, and production never renders so. The `encode` depths, and every depth past the last
/// of them (so the deepest), are encoded in their order's stream box once per `denoise` mode,
/// with the coverage map production passes; of the depths past the last, only the deepest
/// keeps its renders.
pub fn replay_held(
    files: &[PathBuf],
    load: &dyn Fn(&Path) -> RawSub,
    orders: &[RenderOrder],
    encode: &[usize],
    denoise: &[bool],
) -> Replayed {
    use night_amplifier::session::capture::stack_crop::StackCrop;
    use night_amplifier::session::capture::{AnalysisContext, PreviewAnalysis};
    let plugins = night_amplifier::plugins::Plugins::installed();
    let last_asked = encode.iter().copied().max().unwrap_or(0);
    let depths: Vec<usize> = (1..=files.len()).collect();
    let mut held: Vec<_> = orders.iter().map(|o| (o.settings(), PreviewAnalysis::new(), StackCrop::new(), Vec::<HeldRender>::new())).collect();
    let mut deepest = None;
    for_each_snapshot(files, &depths, load, |snapshot| {
        let stack_sigma = night_amplifier::statistics::compute_image_stats(&snapshot.frame).unwrap().mean_sigma();
        for (order, (settings, analysis, crop, rows)) in orders.iter().zip(&mut held) {
            let noise = snapshot.noise.is_usable().then(|| snapshot.noise.clone());
            let rect = crop.decide(settings.auto_crop, snapshot.depth, noise.as_ref());
            analysis.note_crop(rect);
            analysis.note_stack(snapshot.generation);
            let (mut frame, noise) = cut_for_pipeline(snapshot.frame.clone(), noise, rect.as_ref(), settings);
            let ctx = AnalysisContext { showing_stack: snapshot.depth > 1, stack_depth: snapshot.depth as u32 };
            let night_amplifier::session::capture::pipeline::PreviewRender { pipeline_config, stretch_result, solve } =
                night_amplifier::session::capture::pipeline::process_preview_frame_with_analysis(&mut frame, settings, &plugins, ctx, analysis)
                    .unwrap();
            let mut renders = Vec::new();
            if snapshot.depth > last_asked || encode.contains(&snapshot.depth) {
                let ready = night_amplifier::render::display::RenderReadyFrame {
                    noise: noise.map(std::sync::Arc::new),
                    linear_frame: std::sync::Arc::new(frame),
                    pipeline_config,
                    stretch_result,
                };
                let max = order.stream.bounding_box();
                for &on in denoise {
                    let mut ready = ready.clone();
                    if !on {
                        ready.pipeline_config.denoise = night_amplifier::render::DenoiseConfig::OFF;
                    }
                    let (bytes, w, h) = night_amplifier::render::display::frame_to_rgb8_downsampled(&ready, max.0, max.1).unwrap();
                    renders.push((bytes, w as usize, h as usize));
                }
            }
            if let Some(previous) = rows.last_mut().filter(|r| r.depth > last_asked) {
                previous.renders.clear();
            }
            let stack = (snapshot.frame.width(), snapshot.frame.height());
            rows.push(HeldRender { depth: snapshot.depth, stack, crop: rect, solve, stack_sigma, renders });
        }
        deepest = Some(snapshot);
    });
    Replayed { orders: held.into_iter().map(|(_, _, _, rows)| rows).collect(), deepest: deepest.expect("a stack") }
}

// ---------------------------------------------------------------------------
// Instrument 1: octave-band sky noise
// ---------------------------------------------------------------------------

/// Band edges in pixels: each band is the detail between two successive box blurs.
pub const OCTAVE_EDGES: [usize; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// Human labels for [`octave_band_sigma`]'s output, in the same order.
pub const OCTAVE_LABELS: [&str; 7] =
    ["1-2", "2-4", "4-8", "8-16", "16-32", "32-64", "64-128"];

/// The green plane of `region`, as f64 output levels.
///
/// Green rather than luminance: it carries two of the four Bayer sites, so it is the
/// channel the sky's noise is measured on everywhere else in this suite.
pub fn green_region(
    rgb8: &[u8],
    width: usize,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
) -> (Vec<f64>, usize, usize) {
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0));
    for y in y0..y1 {
        for x in x0..x1 {
            out.push(rgb8[(y * width + x) * 3 + 1] as f64);
        }
    }
    (out, x1 - x0, y1 - y0)
}

/// Separable box blur of half-width `radius`, edge-clamped, via running sums.
fn box_blur(src: &[f64], w: usize, h: usize, radius: usize) -> Vec<f64> {
    if radius == 0 {
        return src.to_vec();
    }
    let mut tmp = vec![0.0; src.len()];
    let span = (2 * radius + 1) as f64;
    for y in 0..h {
        let row = &src[y * w..(y + 1) * w];
        // Seed the window as if the row extended by clamping its first sample.
        let mut sum: f64 = row[0] * radius as f64;
        for x in 0..=radius.min(w - 1) {
            sum += row[x.min(w - 1)];
        }
        for x in 0..w {
            tmp[y * w + x] = sum / span;
            let drop = row[x.saturating_sub(radius)];
            let add = row[(x + radius + 1).min(w - 1)];
            sum += add - drop;
        }
    }
    let mut out = vec![0.0; src.len()];
    for x in 0..w {
        let mut sum: f64 = tmp[x] * radius as f64;
        for y in 0..=radius.min(h - 1) {
            sum += tmp[y.min(h - 1) * w + x];
        }
        for y in 0..h {
            out[y * w + x] = sum / span;
            let drop = tmp[y.saturating_sub(radius) * w + x];
            let add = tmp[(y + radius + 1).min(h - 1) * w + x];
            sum += add - drop;
        }
    }
    out
}

/// Robust sigma of a zero-centred detail plane, in output levels.
fn mad_sigma(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = v[v.len() / 2];
    let mut dev: Vec<f64> = v.iter().map(|x| (x - median).abs()).collect();
    dev.sort_by(|a, b| a.partial_cmp(b).unwrap());
    dev[dev.len() / 2] * f64::from(MAD_TO_SIGMA)
}

/// Sky noise split into octaves, as robust sigma in output levels per band.
///
/// Band `k` is `blur(edge[k]) - blur(edge[k+1])`, so it holds the detail between those
/// two scales and nothing else. The eye's grain sits in the last four bands. One global
/// grain figure misled three consecutive changes: an observer reads grain at 8-128 px, and
/// the bands either side of that move independently.
pub fn octave_band_sigma(plane: &[f64], w: usize, h: usize) -> [f64; 7] {
    let blurs: Vec<Vec<f64>> = OCTAVE_EDGES
        .iter()
        .map(|&r| box_blur(plane, w, h, r))
        .collect();
    let mut out = [0.0; 7];
    for k in 0..7 {
        let band: Vec<f64> = blurs[k]
            .iter()
            .zip(&blurs[k + 1])
            .map(|(a, b)| a - b)
            .collect();
        out[k] = mad_sigma(&band);
    }
    out
}

/// Half-width of the region octave bands are measured over, around a sky anchor.
///
/// **Not** the 96 px box the sky *level* is read from: a band cannot be measured on a
/// region narrower than a few times its own scale, so a 96 px box makes the 32-64 and
/// 64-128 px numbers — the two the eye weighs most — an artefact of the box. 192 gives a
/// 384 px region, three times the coarsest band.
pub const GRAIN_HALF_WIDTH: usize = 192;

/// The octave-band region: `sky_box` grown about its own centre, clamped into the frame.
pub fn grain_box(sky_box: (usize, usize, usize, usize), w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (cx, cy) = ((sky_box.0 + sky_box.2) / 2, (sky_box.1 + sky_box.3) / 2);
    let half = GRAIN_HALF_WIDTH.min(w / 2).min(h / 2);
    let x0 = cx.saturating_sub(half).min(w - 2 * half);
    let y0 = cy.saturating_sub(half).min(h - 2 * half);
    (x0, y0, x0 + 2 * half, y0 + 2 * half)
}

/// Fine (1-8 px) and visible (8-128 px) octave bands apart, each as an RMS.
///
/// Keeping them apart is the point: a filter can halve the fine speckle and leave the
/// mottle an observer complains about untouched, and a summary number hides which.
pub fn fine_and_visible(bands: &[f64; 7]) -> (f64, f64) {
    let rms = |xs: &[f64]| (xs.iter().map(|v| v * v).sum::<f64>() / xs.len() as f64).sqrt();
    (rms(&bands[0..3]), rms(&bands[3..7]))
}

/// One line of octave bands, for a diagnostic's table.
pub fn format_octaves(bands: &[f64; 7]) -> String {
    bands
        .iter()
        .map(|v| format!("{v:>7.2}"))
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// Instrument 2: radial profile around bright isolated stars
// ---------------------------------------------------------------------------

/// Radii the profile is sampled at, in pixels of the streamed image.
pub const PROFILE_RADII: [usize; 13] = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25];

/// Where a star's own light has certainly stopped, so its sky can be read.
const SKY_ANNULUS: (f64, f64) = (34.0, 46.0);

/// Keep-out radius between accepted stars: no second star may sit inside the sky
/// annulus, or one star's wings become another's baseline.
const ISOLATION: usize = 50;

pub struct Star {
    pub x: usize,
    pub y: usize,
    pub peak: f64,
}

/// The `count` brightest isolated, point-like stars, found on `plane`.
///
/// Positions are found once on the *un-denoised* render and reused for every variant, so a
/// profile difference is the render's and not the finder's. Three rejections were needed
/// before the profile looked like a star at all on M27's dense field: a maximum must stand
/// clear of its own 34-46 px surround (not sit on nebulosity, most of the frame's bright
/// local maxima), it must be point-like by r=9, and nothing else accepted may sit inside the
/// keep-out — judged on this render, which has no spatial filter applied, so it selects genuine point sources without pre-selecting the shape under test.
pub fn find_isolated_stars(plane: &[f64], w: usize, h: usize, count: usize) -> Vec<Star> {
    let margin = SKY_ANNULUS.1 as usize + 2;
    if w < 2 * margin || h < 2 * margin {
        return Vec::new();
    }
    let sky = plane_median(plane);

    // A local maximum of its 5x5 neighbourhood, well clear of the sky. 40 levels keeps
    // the list to real stars on an 8-bit render whose sky grain is a level or two.
    let mut peaks = Vec::new();
    for y in margin..h - margin {
        for x in margin..w - margin {
            let v = plane[y * w + x];
            if v < sky + 40.0 {
                continue;
            }
            let mut best = true;
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    if (dx, dy) == (0, 0) {
                        continue;
                    }
                    let n = plane[(y as i32 + dy) as usize * w + (x as i32 + dx) as usize];
                    if n > v {
                        best = false;
                    }
                }
            }
            if best {
                peaks.push(Star { x, y, peak: v });
            }
        }
    }
    peaks.sort_by(|a, b| b.peak.partial_cmp(&a.peak).unwrap());

    let mut kept: Vec<Star> = Vec::new();
    for star in peaks {
        // Reject on *any* neighbour inside the keep-out, brighter or not: a companion in
        // the sky annulus biases the baseline whichever way round they are.
        let crowded = kept
            .iter()
            .any(|k| k.x.abs_diff(star.x) < ISOLATION && k.y.abs_diff(star.y) < ISOLATION);
        if crowded {
            continue;
        }
        let rings = star_rings(plane, w, h, &star);
        let Some(rings) = rings else { continue };
        // Standing on the sky, not on the target: M27 fills enough of this frame that
        // most of its bright maxima are nebula texture, and those carry the nebula's own
        // gradient out to every radius the profile reads.
        if rings.sky - sky > 20.0 {
            continue;
        }
        // Point-like: gone by r=9. A star whose wings still carry 15 % of the core at
        // 9 px is either unresolved double or sitting in a glow, and either way its
        // far radii say nothing about ringing.
        let r9 = rings.excess[PROFILE_RADII.iter().position(|&r| r == 9).unwrap()];
        if r9 > 0.15 * rings.excess[0] {
            continue;
        }
        kept.push(star);
        if kept.len() == count {
            break;
        }
    }
    kept
}

struct Rings {
    sky: f64,
    excess: [f64; PROFILE_RADII.len()],
}

/// One star's ring medians as excess over its own 34-46 px annulus.
///
/// **Median** per ring, not mean: M27's field puts a faint star in some ring of most
/// bright stars, and a mean carries it straight into the profile — which is what made a
/// clean star read as though it had 7 levels of wing out at r=25.
fn star_rings(plane: &[f64], w: usize, h: usize, star: &Star) -> Option<Rings> {
    let mut rings: Vec<Vec<f64>> = vec![Vec::new(); PROFILE_RADII.len()];
    let mut sky_samples = Vec::new();
    let reach = SKY_ANNULUS.1 as i32 + 1;
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let (px, py) = (star.x as i32 + dx, star.y as i32 + dy);
            if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                continue;
            }
            let r = ((dx * dx + dy * dy) as f64).sqrt();
            let v = plane[py as usize * w + px as usize];
            if r >= SKY_ANNULUS.0 && r <= SKY_ANNULUS.1 {
                sky_samples.push(v);
                continue;
            }
            for (i, &target) in PROFILE_RADII.iter().enumerate() {
                if (r - target as f64).abs() < 1.0 {
                    rings[i].push(v);
                }
            }
        }
    }
    if sky_samples.is_empty() || rings.iter().any(|r| r.is_empty()) {
        return None;
    }
    let sky = plane_median(&sky_samples);
    let mut excess = [0.0; PROFILE_RADII.len()];
    for (i, ring) in rings.iter().enumerate() {
        excess[i] = plane_median(ring) - sky;
    }
    Some(Rings { sky, excess })
}

/// Median ring excess over local sky at each of [`PROFILE_RADII`], averaged over `stars`.
///
/// Each star's baseline is its own 34-46 px annulus rather than one global sky level: a
/// spatial filter's defect is a *local* lift or trough around the star, and a global
/// baseline hides exactly that by folding the lift into the sky it subtracts.
pub fn radial_excess(
    plane: &[f64],
    w: usize,
    h: usize,
    stars: &[Star],
) -> [f64; PROFILE_RADII.len()] {
    radial_excess_and_surround(plane, w, h, stars).0
}

/// [`radial_excess`], and how far each star's own 34-46 px surround sits above the field
/// sky, averaged over the stars.
///
/// The surround is the instrument for a *lit disc* — a gain driven by a local mean
/// darkens less around every star, so each one ends up sitting in its own pool of
/// brighter sky. `sky_shadow` measured +2.3 output levels at r=21 px this way. It has to
/// be measured against the **field** sky rather than read off the radial profile, because
/// the profile subtracts exactly the thing that is lit.
pub fn radial_excess_and_surround(
    plane: &[f64],
    w: usize,
    h: usize,
    stars: &[Star],
) -> ([f64; PROFILE_RADII.len()], f64) {
    let field_sky = plane_median(plane);
    let mut sums = [0.0; PROFILE_RADII.len()];
    let mut surround = 0.0;
    let mut counted = 0.0;
    for star in stars {
        let Some(rings) = star_rings(plane, w, h, star) else { continue };
        for (s, e) in sums.iter_mut().zip(rings.excess) {
            *s += e;
        }
        surround += rings.sky - field_sky;
        counted += 1.0;
    }
    if counted > 0.0 {
        for s in sums.iter_mut() {
            *s /= counted;
        }
        surround /= counted;
    }
    (sums, surround)
}

pub fn format_profile(profile: &[f64; PROFILE_RADII.len()]) -> String {
    profile
        .iter()
        .map(|v| format!("{v:>7.1}"))
        .collect::<Vec<_>>()
        .join("")
}

pub fn profile_header() -> String {
    PROFILE_RADII
        .iter()
        .map(|r| format!("{:>7}", format!("r{r}")))
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// Object brightness at three radii
// ---------------------------------------------------------------------------

pub struct ObjectExcess {
    pub core: f64,
    pub mid: f64,
    pub outer: f64,
}

/// Median excess over `sky_level` in three annuli about `(cx, cy)`.
///
/// Medians, not means, so a star inside the annulus cannot carry it. The radii are
/// fractions of `span` — the object's own extent — so the same call describes a small
/// planetary nebula and a galaxy that fills the frame.
pub fn object_excess(
    plane: &[f64],
    w: usize,
    h: usize,
    (cx, cy): (usize, usize),
    span: usize,
    sky_level: f64,
) -> ObjectExcess {
    let bands = [(0.0, 0.25), (0.35, 0.60), (0.70, 1.00)];
    let mut out = [0.0; 3];
    for (i, (lo, hi)) in bands.iter().enumerate() {
        let (lo, hi) = (lo * span as f64, hi * span as f64);
        let mut samples = Vec::new();
        let reach = hi.ceil() as i32;
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (px, py) = (cx as i32 + dx, cy as i32 + dy);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                let r = ((dx * dx + dy * dy) as f64).sqrt();
                if r >= lo && r <= hi {
                    samples.push(plane[py as usize * w + px as usize]);
                }
            }
        }
        if samples.is_empty() {
            continue;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        out[i] = samples[samples.len() / 2] - sky_level;
    }
    ObjectExcess { core: out[0], mid: out[1], outer: out[2] }
}

/// Median of a plane, the sky level of a frame the object does not fill.
pub fn plane_median(plane: &[f64]) -> f64 {
    let mut v: Vec<f64> = plane.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// Brightest 96 px block's centre and the darkest block, by green median.
///
/// Both searched over the frame's central 60 % only. The darkest block of the whole
/// frame is reliably a corner of the **registration border**, where fewer subs
/// contributed: its noise is the shallow stack's, not the session's, and anchoring the
/// sky there moved M27's 16-32 px band by 26 % for no change in the render.
pub fn locate_object(
    plane: &[f64],
    w: usize,
    h: usize,
) -> ((usize, usize), (usize, usize, usize, usize)) {
    const B: usize = 96;
    let block_median = |x0: usize, y0: usize| {
        let mut v = Vec::with_capacity(B * B);
        for y in y0..y0 + B {
            v.extend_from_slice(&plane[y * w + x0..y * w + x0 + B]);
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    };
    let (x_lo, x_hi) = (w / 5, w - w / 5 - B);
    let (y_lo, y_hi) = (h / 5, h - h / 5 - B);
    let mut darkest = (f64::MAX, (x_lo, y_lo));
    let mut brightest = (f64::MIN, (x_lo, y_lo));
    for y in (y_lo..=y_hi).step_by(B / 2) {
        for x in (x_lo..=x_hi).step_by(B / 2) {
            let med = block_median(x, y);
            if med < darkest.0 {
                darkest = (med, (x, y));
            }
            if med > brightest.0 {
                brightest = (med, (x, y));
            }
        }
    }
    let (bx, by) = brightest.1;
    let (dx, dy) = darkest.1;
    ((bx + B / 2, by + B / 2), (dx, dy, dx + B, dy + B))
}

// ---------------------------------------------------------------------------
// Sky sigma in 8-bit levels, and plane helpers
// ---------------------------------------------------------------------------

/// Robust sigma of one channel of an interleaved RGB8 buffer, in 8-bit levels.
///
/// A MAD sets the clip and a clipped standard deviation is what gets reported. The MAD
/// alone is what this used to return, and on byte samples it can only take integer
/// values — so the figure snapped to multiples of 1.4826 levels and could not resolve
/// any change smaller than one output level, which is most of them.
pub fn sky_sigma_levels(rgb8: &[u8], channel: usize) -> f64 {
    let samples: Vec<f64> = rgb8.iter().skip(channel).step_by(3).map(|&v| v as f64).collect();
    clipped_sigma(&samples)
}

fn clipped_sigma(samples: &[f64]) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];

    let mut deviations: Vec<f64> = sorted.iter().map(|v| (v - median).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mad_sigma = deviations[deviations.len() / 2] * f64::from(MAD_TO_SIGMA);

    // A floor of one level, or a sky already smooth enough to have a zero MAD would clip
    // away everything including its own noise.
    let clip = (mad_sigma * 3.0).max(1.0);
    let kept: Vec<f64> = sorted.iter().copied().filter(|v| (v - median).abs() <= clip).collect();
    if kept.len() < 2 {
        return mad_sigma;
    }
    let mean = kept.iter().sum::<f64>() / kept.len() as f64;
    (kept.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / kept.len() as f64).sqrt()
}

/// One channel of an interleaved RGB8 buffer as `f64` levels.
pub fn channel_plane(rgb8: &[u8], channel: usize) -> Vec<f64> {
    rgb8.iter().skip(channel).step_by(3).map(|&v| v as f64).collect()
}

/// A rectangular crop of a plane, as its own plane.
pub fn crop(plane: &[f64], width: usize, rect: (usize, usize, usize, usize)) -> (Vec<f64>, usize, usize) {
    let (x0, y0, x1, y1) = rect;
    let (w, h) = (x1.saturating_sub(x0), y1.saturating_sub(y0));
    let mut out = Vec::with_capacity(w * h);
    for y in y0..y1 {
        out.extend_from_slice(&plane[y * width + x0..y * width + x1]);
    }
    (out, w, h)
}

// ---------------------------------------------------------------------------
// Instrument 3: centre against edge
// ---------------------------------------------------------------------------

/// Octave bands of a centre crop and an edge crop, separately, on the green plane.
///
/// **A whole-frame figure cannot answer the question a noise map is asked.** Fewer subs
/// overlap at the stack's border, so noise there is higher by `sqrt(N/count)` — and a
/// single number averages that border into the middle and reports almost no change
/// either way. `fraction` sizes both crops as a share of the shorter axis; the edge crop
/// sits against `side`, where registration drift leaves the thinnest coverage.
pub fn centre_and_edge(
    rgb8: &[u8],
    width: usize,
    height: usize,
    fraction: f64,
    side: Side,
) -> CentreAndEdge {
    let span = ((width.min(height) as f64 * fraction) as usize).clamp(64, width.min(height));
    let (cx, cy) = (width / 2 - span / 2, height / 2 - span / 2);
    let (ex, ey) = match side {
        Side::Left => (0, height / 2 - span / 2),
        Side::Right => (width - span, height / 2 - span / 2),
        Side::Top => (width / 2 - span / 2, 0),
        Side::Bottom => (width / 2 - span / 2, height - span),
    };
    let (centre, cw, ch) = green_region(rgb8, width, (cx, cy, cx + span, cy + span));
    let (edge, ew, eh) = green_region(rgb8, width, (ex, ey, ex + span, ey + span));
    CentreAndEdge {
        centre: octave_band_sigma(&centre, cw, ch),
        edge: octave_band_sigma(&edge, ew, eh),
    }
}

/// Which margin [`centre_and_edge`] crops its edge sample from.
#[derive(Debug, Clone, Copy)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

/// Octave bands at the centre and at one edge.
pub struct CentreAndEdge {
    pub centre: [f64; 7],
    pub edge: [f64; 7],
}

impl CentreAndEdge {
    /// Edge over centre, per band. `1.0` is the goal: the border denoised like the middle.
    pub fn ratios(&self) -> [f64; 7] {
        std::array::from_fn(|i| {
            if self.centre[i] > 0.0 {
                self.edge[i] / self.centre[i]
            } else {
                0.0
            }
        })
    }

    /// Edge over centre across the bands an observer reads grain in (8-128 px), in
    /// quadrature.
    pub fn visible_ratio(&self) -> f64 {
        let energy = |bands: &[f64; 7]| bands[3..].iter().map(|b| b * b).sum::<f64>().sqrt();
        let centre = energy(&self.centre);
        if centre > 0.0 {
            energy(&self.edge) / centre
        } else {
            0.0
        }
    }
}

// ---------------------------------------------------------------------------
// Instrument 4: line coherence
// ---------------------------------------------------------------------------

/// How much coherent row/column structure a sky carries, in output levels.
///
/// Per-row and per-column means with stars excluded by an upper threshold, then the robust
/// sigma of *those means* after removing a low-order fit. Isotropic noise of sigma `s` over
/// `W` columns contributes `s/sqrt(W)` to this (the `floor` returned alongside); anything
/// meaningfully above that floor is coherent banding. **A grain metric cannot see this at
/// all**: a 1440-px row is 40 degrees long at this eyepiece, and the eye integrates along it,
/// so banding well below the grain floor is still visible.
pub struct LineCoherence {
    pub rows: f64,
    pub columns: f64,
    /// `s / sqrt(W)`: what purely isotropic noise of the measured sigma would give.
    pub floor: f64,
}

pub fn line_coherence(plane: &[f64], width: usize, height: usize) -> LineCoherence {
    let sigma = clipped_sigma(plane);
    let mut sorted = plane.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    // Stars and the target out: this is a measurement of the *sky*'s structure.
    let ceiling = median + 4.0 * sigma.max(0.5);

    let line_mean = |samples: &mut dyn Iterator<Item = f64>| -> Option<f64> {
        let kept: Vec<f64> = samples.filter(|v| *v <= ceiling).collect();
        (kept.len() > 8).then(|| kept.iter().sum::<f64>() / kept.len() as f64)
    };

    let row_means: Vec<f64> = (0..height)
        .filter_map(|y| line_mean(&mut plane[y * width..(y + 1) * width].iter().copied()))
        .collect();
    let col_means: Vec<f64> = (0..width)
        .filter_map(|x| line_mean(&mut (0..height).map(|y| plane[y * width + x])))
        .collect();

    LineCoherence {
        rows: detrended_sigma(&row_means),
        columns: detrended_sigma(&col_means),
        floor: sigma / (width.min(height) as f64).sqrt(),
    }
}

/// Robust sigma of a series after removing a straight line, so a real gradient across the
/// frame does not read as banding.
fn detrended_sigma(series: &[f64]) -> f64 {
    if series.len() < 4 {
        return 0.0;
    }
    let n = series.len() as f64;
    let mean_x = (n - 1.0) / 2.0;
    let mean_y = series.iter().sum::<f64>() / n;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, &y) in series.iter().enumerate() {
        let dx = i as f64 - mean_x;
        sxy += dx * (y - mean_y);
        sxx += dx * dx;
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    let residuals: Vec<f64> = series
        .iter()
        .enumerate()
        .map(|(i, &y)| y - (mean_y + slope * (i as f64 - mean_x)))
        .collect();
    clipped_sigma(&residuals)
}

// ---------------------------------------------------------------------------
// Instrument 5: a repeating tile's lines in the 8-bit output
// ---------------------------------------------------------------------------

/// Power on a `period`-px tile's lattice in the output spectrum, as a ratio to the
/// spectrum just beside each line: `1.0` is no periodic structure at that period.
pub struct LatticeLines {
    /// The tile's fundamental, `(±1, 0)`, `(0, ±1)` and `(±1, ±1)` cycles per tile.
    pub fundamental: f64,
    /// Every line of the lattice below half Nyquist (periods of 4 px and longer).
    pub below_half_nyquist: f64,
}

/// [`LatticeLines`] on a square patch of `plane` centred near `centre`.
///
/// The patch side is an **odd** multiple of `period`, so this tile's lines land exactly on
/// DFT bins while a tile of twice the period or more — the 64 px mask when asking about
/// Bayer's 8 px — leaks into a continuum instead of lining up with them. Each line is
/// judged against the median of the bins within three of it, off the lattice, so a sky's
/// broadband grain and a star's smooth spectrum cancel out of the ratio.
pub fn lattice_lines(
    plane: &[f64],
    width: usize,
    height: usize,
    centre: (usize, usize),
    period: usize,
) -> LatticeLines {
    // Below 4 px no lattice line lies under half Nyquist, and `below_half_nyquist` would
    // read 0 — "no lattice" — without measuring anything.
    assert!(period >= 4, "a {period} px tile has no lines below half Nyquist to measure");
    let limit = width.min(height).min(256);
    assert!(limit >= period, "a {period} px tile does not fit a {width}x{height} plane");
    let mut cycles = limit / period;
    if cycles.is_multiple_of(2) {
        cycles -= 1;
    }
    let side = cycles * period;
    let x0 = centre.0.saturating_sub(side / 2).min(width - side);
    let y0 = centre.1.saturating_sub(side / 2).min(height - side);
    let mut patch: Vec<f64> = (0..side)
        .flat_map(|y| plane[(y0 + y) * width + x0..][..side].iter().copied())
        .collect();
    let mean = patch.iter().sum::<f64>() / patch.len() as f64;
    patch.iter_mut().for_each(|v| *v -= mean);

    let twiddle: Vec<(f64, f64)> = (0..side)
        .map(|k| {
            let phase = -2.0 * std::f64::consts::PI * k as f64 / side as f64;
            (phase.cos(), phase.sin())
        })
        .collect();
    let wrap = |f: i64| f.rem_euclid(side as i64) as usize;
    // Row transforms at one horizontal frequency, cached: every line and its
    // neighbourhood reuse a handful of them.
    let mut rows: std::collections::HashMap<usize, Vec<(f64, f64)>> = Default::default();
    let mut power = |u: i64, v: i64| -> f64 {
        let u = wrap(u);
        let row = rows.entry(u).or_insert_with(|| {
            (0..side)
                .map(|y| {
                    let line = &patch[y * side..][..side];
                    line.iter().enumerate().fold((0.0, 0.0), |(re, im), (x, &p)| {
                        let (c, s) = twiddle[(u * x) % side];
                        (re + p * c, im + p * s)
                    })
                })
                .collect()
        });
        let v = wrap(v);
        let (re, im) = row.iter().enumerate().fold((0.0, 0.0), |(re, im), (y, &(r, i))| {
            let (c, s) = twiddle[(v * y) % side];
            (re + r * c - i * s, im + r * s + i * c)
        });
        re * re + im * im
    };

    let step = cycles as i64;
    let on_lattice = |u: i64, v: i64| u.rem_euclid(step) == 0 && v.rem_euclid(step) == 0;
    let mut line_ratio = |a: i64, b: i64| -> f64 {
        let (u, v) = (a * step, b * step);
        let mut beside = Vec::new();
        for dv in -3i64..=3 {
            for du in -3i64..=3 {
                if !on_lattice(u + du, v + dv) {
                    beside.push(power(u + du, v + dv));
                }
            }
        }
        beside.sort_by(|p, q| p.partial_cmp(q).unwrap());
        // The median of an exponential is ln 2 of its mean, so this reads 1.0 on noise.
        let reference = beside[beside.len() / 2] / std::f64::consts::LN_2;
        power(u, v) / reference.max(1e-300)
    };

    let fundamental = [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)];
    let fundamental =
        fundamental.iter().map(|&(a, b)| line_ratio(a, b)).sum::<f64>() / fundamental.len() as f64;
    // Half Nyquist is a quarter of the patch's bins; in lattice steps, `period / 4`.
    let reach = (period / 4) as i64;
    let mut sum = 0.0;
    let mut count = 0;
    for b in -reach..=reach {
        for a in -reach..=reach {
            if (a, b) != (0, 0) && a * a + b * b <= reach * reach {
                sum += line_ratio(a, b);
                count += 1;
            }
        }
    }
    LatticeLines { fundamental, below_half_nyquist: sum / count.max(1) as f64 }
}

// ---------------------------------------------------------------------------
// Instrument 6: what an encoder keeps of the brightness between two levels
// ---------------------------------------------------------------------------

/// RMS difference of `block`-px block means between `plane` and `reference` over the
/// blocks wholly inside `region` (`x0, y0, x1, y1`), in the planes' own units.
///
/// The eye averages a few pixels, so the brightness a dither puts between two 8-bit
/// levels lives in local means — and JPEG, which zeroes the fine detail of each 8x8 block
/// first, can take exactly that back: at q90 a flat sky's block means miss by as much as
/// with no dither. `reference` is the true input on a synthetic sky, or the bytes before
/// the encoder on a real one.
pub fn block_mean_error(
    plane: &[f64],
    reference: &[f64],
    width: usize,
    (x0, y0, x1, y1): (usize, usize, usize, usize),
    block: usize,
) -> f64 {
    let (mut sum, mut count) = (0.0, 0usize);
    for by in (y0..y1).step_by(block).take_while(|&by| by + block <= y1) {
        for bx in (x0..x1).step_by(block).take_while(|&bx| bx + block <= x1) {
            let mut difference = 0.0;
            for y in by..by + block {
                let row = y * width;
                for x in bx..bx + block {
                    difference += plane[row + x] - reference[row + x];
                }
            }
            sum += (difference / (block * block) as f64).powi(2);
            count += 1;
        }
    }
    assert!(count > 0, "no {block} px block fits inside {:?}", (x0, y0, x1, y1));
    (sum / count as f64).sqrt()
}

// ---------------------------------------------------------------------------
// Instrument 7: the target's own structure
// ---------------------------------------------------------------------------

/// Robust sigma of the `lo..hi` px band-pass (difference of two box blurs) at the
/// object's core, mid and outer annuli — the radii [`object_excess`] reads brightness at.
///
/// Pass a crop around the object: the blurs run over the whole plane. The brightness
/// median cannot see a change in texture, which is all a local-contrast gain makes.
pub fn object_structure(
    plane: &[f64],
    w: usize,
    h: usize,
    (cx, cy): (usize, usize),
    span: usize,
    (lo, hi): (usize, usize),
) -> [f64; 3] {
    let band = band_pass(plane, w, h, (lo, hi));
    let bands = [(0.0, 0.25), (0.35, 0.60), (0.70, 1.00)];
    std::array::from_fn(|i| {
        let (l, u) = (bands[i].0 * span as f64, bands[i].1 * span as f64);
        let reach = u.ceil() as i64;
        let mut samples = Vec::new();
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (px, py) = (cx as i64 + dx, cy as i64 + dy);
                let r = ((dx * dx + dy * dy) as f64).sqrt();
                if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h && r >= l && r <= u {
                    let at = py as usize * w + px as usize;
                    samples.push(band[at]);
                }
            }
        }
        if samples.len() < 8 {
            return 0.0;
        }
        mad_sigma(&samples)
    })
}

/// The `lo..hi` px band-pass of `plane`: a box blur of radius `lo` less one of `hi`.
pub fn band_pass(plane: &[f64], w: usize, h: usize, (lo, hi): (usize, usize)) -> Vec<f64> {
    let (fine, coarse) = (box_blur(plane, w, h, lo), box_blur(plane, w, h, hi));
    fine.iter().zip(&coarse).map(|(f, c)| f - c).collect()
}

// ---------------------------------------------------------------------------
// Instrument 8: a star's own response
// ---------------------------------------------------------------------------

/// Add a Gaussian star of `peak` and `sigma` (input pixels) at each of `at`, given in
/// *output* pixels and mapped back by `scale` (input per output pixel), to every channel.
pub fn add_stars(frame: &mut Frame, at: &[(f64, f64)], scale: f64, peak: f32, sigma: f64) {
    let (w, h) = (frame.width(), frame.height());
    let reach = (sigma * 5.0).ceil() as i64;
    let (r, g, b) = frame.planes_mut();
    for &(sx, sy) in at {
        let (cx, cy) = (sx * scale, sy * scale);
        for dy in -reach..=reach {
            for dx in -reach..=reach {
                let (px, py) = (cx.round() as i64 + dx, cy.round() as i64 + dy);
                if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                    continue;
                }
                let (fx, fy) = (px as f64 - cx, py as f64 - cy);
                let v = peak * (-(fx * fx + fy * fy) / (2.0 * sigma * sigma)).exp() as f32;
                let i = py as usize * w + px as usize;
                r[i] += v;
                g[i] += v;
                b[i] += v;
            }
        }
    }
}

/// The median over `at` of each star's ring-median `with - without` at r = 0..=24 on the
/// green plane, in output levels: what the star itself became in the render.
///
/// Stars are injected rather than found because the radial profile's stars must be isolated
/// on the sky — the one place a filter that rings on a nebula can look perfect. The
/// difference cancels the target around the star, but not a filter's reaction to it:
/// protection switched on near a star leaves a disc of unraised texture, reading here as an
/// offset, not a ring — over a few sites it can read as a moat, so pool enough of them ([`response_rings`]) that the target's own texture averages out.
pub fn star_response(with: &[u8], without: &[u8], w: usize, at: &[(f64, f64)]) -> [f64; 25] {
    median_rings(&response_rings(with, without, w, at))
}

/// Each site's ring-median `with - without` at r = 0..=24, NaN where no pixel of the ring
/// could be read: [`star_response`] before the median, so injection passes can be pooled.
///
/// A pixel white in `with` (green >= 250) is not read. A star clipped there cannot grow,
/// so its difference is the filter's change to the target *under* it, negated: on Orion's
/// core, which renders at ~230, every star from 20 to 500 sigma read +4 at r=0-2 for that
/// reason alone. Dropping pixels rather than stars keeps a bright star's outer rings,
/// which is where a moat would be.
pub fn response_rings(with: &[u8], without: &[u8], w: usize, at: &[(f64, f64)]) -> Vec<[f64; 25]> {
    rings_where(with, without, w, at, |i| with[i] < 250)
}

/// [`response_rings`] over the pixels `readable` accepts, by index into the RGB8 bytes.
fn rings_where(
    with: &[u8],
    without: &[u8],
    w: usize,
    at: &[(f64, f64)],
    readable: impl Fn(usize) -> bool,
) -> Vec<[f64; 25]> {
    let h = with.len() / (w * 3);
    at.iter()
        .map(|&(sx, sy)| {
            let mut rings: Vec<Vec<f64>> = vec![Vec::new(); 25];
            for dy in -25i64..=25 {
                for dx in -25i64..=25 {
                    let (px, py) = (sx.round() as i64 + dx, sy.round() as i64 + dy);
                    if px < 0 || py < 0 || px as usize >= w || py as usize >= h {
                        continue;
                    }
                    let r = (px as f64 - sx).hypot(py as f64 - sy).round() as usize;
                    let i = (py as usize * w + px as usize) * 3 + 1;
                    if r < 25 && readable(i) {
                        rings[r].push(with[i] as f64 - without[i] as f64);
                    }
                }
            }
            std::array::from_fn(|r| if rings[r].is_empty() { f64::NAN } else { plane_median(&rings[r]) })
        })
        .collect()
}

/// Per radius, the median over sites of [`response_rings`], skipping the NaNs; 0 where none.
pub fn median_rings(rings: &[[f64; 25]]) -> [f64; 25] {
    std::array::from_fn(|r| {
        let at: Vec<f64> = rings.iter().map(|site| site[r]).filter(|v| !v.is_nan()).collect();
        if at.is_empty() { 0.0 } else { plane_median(&at) }
    })
}

/// A stack through the pipeline, stopped where the encoder takes over: the linear frame,
/// and what the encoder needs to render it. Split so a test can change the frame between
/// the two — [`add_stars`] after the solve, so the stars cannot move the tone curve.
pub fn preview_parts(
    stack: &Frame,
    settings: &night_amplifier::session::state::CaptureSettings,
    stack_depth: u32,
) -> (Frame, night_amplifier::render::RenderPipelineConfig, Option<night_amplifier::render::display::StretchResult>) {
    use night_amplifier::session::capture::{AnalysisContext, PreviewAnalysis};
    let (mut frame, _) = cut_for_pipeline(stack.clone(), None, None, settings);
    let render = night_amplifier::session::capture::pipeline::process_preview_frame_with_analysis(
        &mut frame,
        settings,
        &night_amplifier::plugins::Plugins::installed(),
        AnalysisContext { showing_stack: stack_depth > 1, stack_depth },
        &mut PreviewAnalysis::new(),
    )
    .unwrap();
    (frame, render.pipeline_config, render.stretch_result)
}

/// Encode a frame from [`preview_parts`] at the stream size: RGB8 bytes, width, height.
pub fn encode_preview(
    frame: Frame,
    pipeline_config: &night_amplifier::render::RenderPipelineConfig,
    stretch_result: &Option<night_amplifier::render::display::StretchResult>,
) -> (Vec<u8>, usize, usize) {
    encode_preview_at(frame, pipeline_config, stretch_result, stream())
}

/// [`encode_preview`] into the box `(max_w, max_h)`.
pub fn encode_preview_at(
    frame: Frame,
    pipeline_config: &night_amplifier::render::RenderPipelineConfig,
    stretch_result: &Option<night_amplifier::render::display::StretchResult>,
    (max_w, max_h): (u32, u32),
) -> (Vec<u8>, usize, usize) {
    let ready = night_amplifier::render::display::RenderReadyFrame {
        noise: None,
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config: pipeline_config.clone(),
        stretch_result: stretch_result.clone(),
    };
    let (bytes, w, h) =
        night_amplifier::render::display::frame_to_rgb8_downsampled(&ready, max_w, max_h).unwrap();
    (bytes, w as usize, h as usize)
}

/// A render of [`preview_parts`]: its pipeline configuration and stretch.
pub type PreviewRender<'a> = (
    &'a night_amplifier::render::RenderPipelineConfig,
    &'a Option<night_amplifier::render::display::StretchResult>,
);

/// What render `b` does to stars of `peak` injected on `frame` against render `a`, pooled
/// over four passes of `sites(offset)` at offsets of 0 and 22 px each way: `(a's
/// [`star_response`], the median over stars of each one's own b-less-a response, stars read per radius)`.
///
/// Paired and pooled star by star: a difference of two medians over a single 44 px grid
/// (which keeps each star's r <= 24 rings clear of its neighbours) is noise on a textured
/// target — texture shifts every site by several levels where the profile is steep, and
/// stars on Orion's outskirts that Detail left untouched read -3 at r=3 that way, -5 over the two sites a single grid could read.
pub fn pooled_star_response(
    frame: &Frame,
    [a, b]: [PreviewRender; 2],
    sites: impl Fn((i64, i64)) -> Vec<(f64, f64)>,
    (peak, star_sigma, scale): (f32, f64, f64),
) -> ([f64; 25], [f64; 25], [usize; 25]) {
    let (without_a, w, _) = encode_preview(frame.clone(), a.0, a.1);
    let (without_b, _, _) = encode_preview(frame.clone(), b.0, b.1);
    let (mut rings_a, mut change) = (Vec::new(), Vec::new());
    for offset in [(0, 0), (22, 0), (0, 22), (22, 22)] {
        let at = sites(offset);
        let mut starred = frame.clone();
        add_stars(&mut starred, &at, scale, peak, star_sigma);
        let (with_a, _, _) = encode_preview(starred.clone(), a.0, a.1);
        let (with_b, _, _) = encode_preview(starred, b.0, b.1);
        // One set of pixels for both: masked apart, the two renders' rings took different
        // pixels at a clipped core's edge.
        let readable = |i: usize| with_a[i] < 250 && with_b[i] < 250;
        let site_a = rings_where(&with_a, &without_a, w, &at, readable);
        let site_b = rings_where(&with_b, &without_b, w, &at, readable);
        change.extend(site_a.iter().zip(&site_b).map(|(a, b)| std::array::from_fn(|r| b[r] - a[r])));
        rings_a.extend(site_a);
    }
    let read = std::array::from_fn(|r| change.iter().filter(|site: &&[f64; 25]| !site[r].is_nan()).count());
    (median_rings(&rings_a), median_rings(&change), read)
}

// ---------------------------------------------------------------------------
// Instrument 9: the target's signal and noise apart
// ---------------------------------------------------------------------------

/// Two independent stacks of one session, alternate subs each, sharing the first as
/// their registration reference so they stay aligned: `[(depth, frame); 2]`.
///
/// The shared sub is one of each half's ~15-50 — a few percent of common noise, which
/// reads as signal in [`signal_and_noise`]. Render both through **one** tone curve
/// ([`preview_parts`] of the first, [`encode_preview`] of both): curves solved per half
/// put a gain mismatch into their difference that scales with the signal.
pub fn half_stacks(files: &[PathBuf]) -> [(usize, Frame); 2] {
    let even: Vec<PathBuf> = files.iter().step_by(2).cloned().collect();
    let odd: Vec<PathBuf> = files[..1].iter().chain(files.iter().skip(1).step_by(2)).cloned().collect();
    [even, odd].map(|half| stack_snapshots(&half, &[]).pop().expect("a stack"))
}

/// Pixels within 5 px of a compact source: over 6 MADs of the 1-6 px [`band_pass`].
pub fn compact_sources(plane: &[f64], w: usize, h: usize) -> Vec<bool> {
    let compact = band_pass(plane, w, h, (1, 6));
    let limit = 6.0
        * plane_median(&compact.iter().map(|v| v.abs()).collect::<Vec<_>>())
        * f64::from(MAD_TO_SIGMA);
    let mut mask = vec![false; w * h];
    for (p, _) in compact.iter().enumerate().filter(|&(_, &v)| v > limit) {
        let (x, y) = ((p % w) as i64, (p / w) as i64);
        for dy in -5i64..=5 {
            for dx in -5i64..=5 {
                let (xx, yy) = (x + dx, y + dy);
                if dx * dx + dy * dy <= 25 && xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                    mask[yy as usize * w + xx as usize] = true;
                }
            }
        }
    }
    mask
}

/// `(signal, noise)` sigmas over `keep` from two renders of [`half_stacks`] — their
/// band-passes, say: the covariance is what both hold, half the variance of their
/// difference what each holds alone.
pub fn signal_and_noise(a: &[f64], b: &[f64], keep: &[usize]) -> (f64, f64) {
    let n = keep.len() as f64;
    let mean = |v: &[f64]| keep.iter().map(|&i| v[i]).sum::<f64>() / n;
    let (ma, mb) = (mean(a), mean(b));
    let covariance = keep.iter().map(|&i| (a[i] - ma) * (b[i] - mb)).sum::<f64>() / n;
    let difference = keep.iter().map(|&i| (a[i] - b[i] - (ma - mb)).powi(2)).sum::<f64>() / n;
    (covariance.max(0.0).sqrt(), (difference / 2.0).sqrt())
}

// ---------------------------------------------------------------------------
// The stream every instrument measures at
// ---------------------------------------------------------------------------

/// Stream size every instrument measures at — the resolution an observer is actually
/// served, so the encoders' box downsample has already had its share of the noise.
///
/// `STREAM_MAX` overrides it. That box downsample is not neutral to this measurement: it
/// averages 2x2 or more before the filters run, so a change to the *finest* wavelet scale
/// looks far smaller here than it does in the saved full-resolution PNG.
pub const STREAM: (u32, u32) = (2560, 1440);

pub fn stream() -> (u32, u32) {
    match std::env::var("STREAM_MAX").ok().and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => (n, n),
        None => STREAM,
    }
}

// ---------------------------------------------------------------------------
// Diagnostics over the four out-of-repo sessions
// ---------------------------------------------------------------------------

/// The four sessions the render's brightness was tuned against. They fail differently: M27's
/// dense field is the worst case for anything spatial, Andromeda for extended low-contrast
/// structure, Orion is another sensor and a much shallower stack.
///
/// Out of the repo, so these are diagnostics rather than guards, driven by
/// `RENDER_BRIGHTNESS_SETS` (a comma-separated subset) and cached as stacked FITS under
/// `RENDER_BRIGHTNESS_CACHE` — stacking 266 subs takes far longer than rendering them, and
/// the same stacks are measured under a dozen settings.
const SESSIONS: &[(&str, &str)] = &[
    ("m27", "/home/neon/Documents/Night_Amplifier/data/dumbbell-250mm-dob-imx533-100-subs"),
    ("globular", "/home/neon/Documents/Night_Amplifier/data/globular-cluster"),
    ("andromeda", "/home/neon/Documents/Night_Amplifier/data/andromeda-250mm-dob-imx533"),
    ("orion", "/opt/GitHub/night-amplifier-pro/tests/fixtures/250mm-dob-imx464-orion-png"),
];

fn cache_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("RENDER_BRIGHTNESS_CACHE")
            .unwrap_or_else(|_| "/tmp/night-amplifier-stacks".to_string()),
    )
}

/// Every sub of a session, sorted, FITS or PNG.
pub fn session_frames(dir: &Path) -> Option<Vec<PathBuf>> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "fits" | "fit" | "png"))
        })
        .collect();
    if files.len() < 8 {
        return None;
    }
    files.sort();
    Some(files)
}

/// The session's stack, from the cache when it is there and stacked when it is not.
///
/// Raw planar f32 with a four-number header, **not** FITS: `fits::read_frame`'s float arm
/// min/max-normalises what it reads to `[0,1]`, since a float FITS from another tool
/// carries no declared full-well — so `write_fits` followed by `read_frame` is not an
/// identity and a cached stack rendered 22 output levels dimmer than the one it came
/// from. A cache that changes the measurement is worse than no cache.
pub fn cached_stack(name: &str, dir: &str) -> Option<(night_amplifier::Frame, u32)> {
    let files = session_frames(Path::new(dir))?;
    let cache = cache_dir().join(format!("{name}-{}.stack", files.len()));
    if let Some(hit) = read_cached_stack(&cache) {
        println!("  {name}: {} of {} subs, from cache", hit.1, files.len());
        return Some(hit);
    }
    println!("  {name}: stacking {} subs...", files.len());
    // The *stacked* count, not the file count: a sub that failed registration is not in
    // the stack, and the tone curve is solved from this number.
    let (stacked, stack) = stack_snapshots(&files, &[]).pop()?;
    let depth = stacked as u32;
    println!("  {name}: {depth} of {} subs registered and stacked", files.len());
    write_cached_stack(&cache, &stack, depth);
    Some((stack, depth))
}

fn read_cached_stack(path: &Path) -> Option<(night_amplifier::Frame, u32)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 16 {
        return None;
    }
    let word = |i: usize| u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize;
    let (w, h, c, depth) = (word(0), word(1), word(2), word(3) as u32);
    let samples = w * h * c;
    if bytes.len() != 16 + samples * 4 {
        return None;
    }
    let data: Vec<f32> = bytes[16..]
        .as_chunks::<4>().0.iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    Some((night_amplifier::Frame::from_f32_vec(data, w, h, c).ok()?, depth))
}

fn write_cached_stack(path: &Path, frame: &night_amplifier::Frame, depth: u32) {
    let _ = std::fs::create_dir_all(cache_dir());
    let mut bytes = Vec::with_capacity(16 + frame.data().len() * 4);
    for n in [frame.width() as u32, frame.height() as u32, frame.channels() as u32, depth] {
        bytes.extend_from_slice(&n.to_le_bytes());
    }
    for v in frame.data() {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    if let Err(e) = std::fs::write(path, bytes) {
        println!("  could not cache the stack: {e}");
    }
}

/// The sessions named by `RENDER_BRIGHTNESS_SETS`, or all four.
pub fn requested_sessions() -> Vec<(&'static str, &'static str)> {
    let filter = std::env::var("RENDER_BRIGHTNESS_SETS").ok();
    SESSIONS
        .iter()
        .filter(|(name, _)| match &filter {
            Some(list) => list.split(',').any(|w| w.trim() == *name),
            None => true,
        })
        .copied()
        .collect()
}

/// Where a session is measured: the object's centre, the sky patch, and the stars.
///
/// Found once and then **reused for every render of that session**, including renders
/// from a different build. Re-finding them per variant compares different pixels: the
/// darkest 96 px block moves when the sky level moves, which made a sky patch swap read
/// as a 4x improvement in coarse grain, and the brightest block moving off the object's
/// centre read as the target dimming 11 %.
pub struct Anchors {
    pub object: (usize, usize),
    pub sky_box: (usize, usize, usize, usize),
    pub stars: Vec<Star>,
}

/// Anchors for a session, from the cache when it is there.
///
/// The reference render they are found on is the product default with denoising off:
/// off, because the star positions and their point-likeness must be judged on a render
/// no spatial filter has touched.
pub fn anchors(name: &str, stack: &night_amplifier::Frame, depth: u32) -> Anchors {
    // Keyed on the render size too: every anchor is a pixel coordinate in the rendered
    // image, so a set found at 1440 points somewhere else entirely at 3008.
    let path = cache_dir().join(format!("{name}-{depth}-{}.anchors", stream().0));
    if std::env::var("RENDER_BRIGHTNESS_REANCHOR").is_err() {
        if let Some(a) = read_anchors(&path) {
            return a;
        }
    }
    let settings = night_amplifier::session::state::CaptureSettings::default();
    let (rgb8, w, h) = render(stack.clone(), &settings, false, STREAM, depth);
    let (plane, pw, ph) = green_region(&rgb8, w, (0, 0, w, h));
    let (object, sky_box) = locate_object(&plane, pw, ph);
    let stars = find_isolated_stars(&plane, pw, ph, 25);
    let a = Anchors { object, sky_box, stars };
    write_anchors(&path, &a, (w, h));
    a
}

fn read_anchors(path: &Path) -> Option<Anchors> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let nums = |line: &str| -> Vec<usize> {
        line.split_whitespace().filter_map(|w| w.parse().ok()).collect()
    };
    let head = nums(lines.next()?);
    if head.len() != 6 {
        return None;
    }
    let stars = lines
        .filter_map(|l| {
            let v = nums(l);
            (v.len() == 3).then(|| Star { x: v[0], y: v[1], peak: v[2] as f64 })
        })
        .collect();
    Some(Anchors {
        object: (head[0], head[1]),
        sky_box: (head[2], head[3], head[4], head[5]),
        stars,
    })
}

fn write_anchors(path: &Path, a: &Anchors, (w, h): (usize, usize)) {
    let _ = std::fs::create_dir_all(cache_dir());
    let (x0, y0, x1, y1) = a.sky_box;
    let mut text = format!("{} {} {x0} {y0} {x1} {y1}\n", a.object.0, a.object.1);
    for s in &a.stars {
        text.push_str(&format!("{} {} {}\n", s.x, s.y, s.peak as i64));
    }
    if let Err(e) = std::fs::write(path, text) {
        println!("  could not cache anchors: {e}");
    }
    println!(
        "  anchored on a {w}x{h} reference: object {:?}, sky {:?}, {} stars",
        a.object,
        a.sky_box,
        a.stars.len()
    );
}

/// Every instrument, on one render, at the session's fixed anchors.
pub fn report(label: &str, rgb8: &[u8], w: usize, h: usize, a: &Anchors) {
    let (plane, pw, ph) = green_region(rgb8, w, (0, 0, w, h));
    let (sky_plane, sw, sh) = green_region(rgb8, w, a.sky_box);
    let sky_level = plane_median(&sky_plane);
    let obj = object_excess(&plane, pw, ph, a.object, 96, sky_level);
    let bands = octave_band_sigma(&sky_plane, sw, sh);
    let profile = radial_excess(&plane, pw, ph, &a.stars);

    println!(
        "{label:<24} sky {sky_level:>5.1}  core {:>6.1}  mid {:>6.1}  outer {:>6.1}  octaves{}",
        obj.core,
        obj.mid,
        obj.outer,
        format_octaves(&bands),
    );
    println!("{:<24} profile{}", "", format_profile(&profile));
}

// ---------------------------------------------------------------------------
// Grain against depth on a real session
// ---------------------------------------------------------------------------

/// The grain ratio a stack of `frames` is expected to reach, at the shipped split.
///
/// Read off the curve itself rather than restated as `N^(-1/8)`: the exponent and the
/// depth it stops at are the product's decision — now the middle of a user-facing dial —
/// and a copy here would go on asserting the old one after that decision changed.
pub fn expected_grain_ratio(frames: usize) -> f64 {
    1.0 / night_amplifier::render::depth_grain_gain(
        frames as u32,
        night_amplifier::render::DEFAULT_GRAIN_SPLIT,
    ) as f64
}

pub struct Measured {
    pub sky_grain: f64,
    pub sky_level: f64,
    pub target_contrast: f64,
}

pub fn crop_rgb8(rgb8: &[u8], width: usize, (x0, y0, x1, y1): (usize, usize, usize, usize)) -> Vec<u8> {
    let mut out = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        let row = y * width;
        out.extend_from_slice(&rgb8[(row + x0) * 3..(row + x1) * 3]);
    }
    out
}

pub fn green_median(rgb8: &[u8]) -> f64 {
    let mut g: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    g.sort_unstable();
    g[g.len() / 2] as f64
}

/// Levels are [`green_level`]s, so a target under one output level still has a contrast:
/// as whole-level medians IC 63 read 0 at one sub, and target-to-grain gain divided by it.
pub fn measure_sky_and_target(rgb8: &[u8], width: usize, sky: (usize, usize, usize, usize), target: (usize, usize, usize, usize)) -> Measured {
    let sky_px = crop_rgb8(rgb8, width, sky);
    let target_px = crop_rgb8(rgb8, width, target);
    let sky_level = green_level(&sky_px);
    Measured {
        sky_grain: sky_sigma_levels(&sky_px, 1),
        sky_level,
        target_contrast: green_level(&target_px) - sky_level,
    }
}

/// Mean of the green channel's middle half: a star in the block moves it no more than a
/// median, and the block's noise dithers it finer than one output level.
pub fn green_level(rgb8: &[u8]) -> f64 {
    let mut g: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    g.sort_unstable();
    let quarter = g.len() / 4;
    let middle = &g[quarter..g.len() - quarter];
    middle.iter().map(|&v| f64::from(v)).sum::<f64>() / middle.len() as f64
}

/// `(x0, y0, x1, y1)` in pixels.
pub type PixelBox = (usize, usize, usize, usize);

/// The side of the blocks [`locate_boxes`] and [`box_at`] measure.
const BOX: usize = 96;

/// The sky and the target: the 96 px green block whose median sits nearest the rendered
/// sky level (the interior's histogram mode, what the stretch solves its sky to), and the
/// brightest block. Of the blocks equally near, the most uniform (5-95 % range): one
/// straddling a population boundary reads the step as grain.
///
/// Not the darkest block: on a target that fills the frame that is real sky *below* the
/// mode, which the black point is built to put near black — M33's corner went 6 -> 4
/// levels with depth and its grain fell to 0.32x while the sky the curve solved held.
pub fn locate_boxes(rgb8: &[u8], w: usize, h: usize) -> (PixelBox, PixelBox) {
    const B: usize = BOX;
    let sky_level = interior_green_mode(rgb8, w, h, B) as f64;
    let mut sky = ((f64::MAX, u8::MAX), (0, 0, B, B));
    let mut brightest = (f64::MIN, (0, 0, B, B));
    // Stay off the edges: registration leaves a partially covered border.
    for y in (B..h - 2 * B).step_by(B / 2) {
        for x in (B..w - 2 * B).step_by(B / 2) {
            let bx = (x, y, x + B, y + B);
            let block = crop_rgb8(rgb8, w, bx);
            let med = green_median(&block);
            let fit = ((med - sky_level).abs(), green_range(&block));
            if fit.0 < sky.0 .0 || (fit.0 == sky.0 .0 && fit.1 < sky.0 .1) {
                sky = (fit, bx);
            }
            if med > brightest.0 {
                brightest = (med, bx);
            }
        }
    }
    (sky.1, brightest.1)
}

/// Spread of the green channel between its 5th and 95th percentiles, in levels.
fn green_range(rgb8: &[u8]) -> u8 {
    let mut g: Vec<u8> = rgb8.iter().skip(1).step_by(3).copied().collect();
    g.sort_unstable();
    g[g.len() * 95 / 100] - g[g.len() * 5 / 100]
}

/// Most common green level inside a `margin` px border.
fn interior_green_mode(rgb8: &[u8], w: usize, h: usize, margin: usize) -> u8 {
    let mut histogram = [0usize; 256];
    for y in margin..h - margin {
        for x in margin..w - margin {
            histogram[rgb8[(y * w + x) * 3 + 1] as usize] += 1;
        }
    }
    (0..=255u8).max_by_key(|&level| histogram[level as usize]).expect("256 levels")
}

/// The 96 px block centred on `(cx, cy)`, in frame fractions, kept inside a `w x h` frame
/// (at its origin, if the frame is narrower than a block).
pub fn box_at((cx, cy): (f64, f64), w: usize, h: usize) -> PixelBox {
    let corner = |c: f64, side: usize| ((c * side as f64) as usize).saturating_sub(BOX / 2).min(side.saturating_sub(BOX));
    let (x, y) = (corner(cx, w), corner(cy, h));
    (x, y, x + BOX, y + BOX)
}

/// Grain against depth on a real session, the target against that grain, and how far
/// the sky level moves between one stack update and the next.
///
/// `denoise_modes` picks which halves run, because they now belong to different repos:
/// `false` is the tone curve's own contract and Community's to keep; `true` asserts what
/// the filters do, which only the Pro repo can exercise. Without the plugin that half
/// renders no filters at all and would fail for the wrong reason.
pub fn measure_real_session(
    label: &str,
    files: &[std::path::PathBuf],
    depths: &[usize],
    denoise_modes: &[bool],
    give_back: GiveBack,
    load: &dyn Fn(&Path) -> RawSub,
) {
    if std::env::var("STACK_GRAIN_TRACE").is_ok() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("night_amplifier_core::render::autostretch=debug")
            .with_test_writer()
            .try_init();
    }
    // Every depth through one held analysis, as the render task shows a stack; the filtered
    // half renders with the noise map it attaches. The boxes are located on a plain render.
    let modes: Vec<bool> = std::iter::once(false).chain(denoise_modes.iter().copied().filter(|&d| d)).collect();
    let replayed = replay_held(files, load, &RENDER_ORDERS, depths, &modes);
    for (order, rows) in RENDER_ORDERS.iter().zip(&replayed.orders) {
        let shown: Vec<&HeldRender> = rows.iter().filter(|r| !r.renders.is_empty()).collect();
        for &denoise in denoise_modes {
            let mode = modes.iter().position(|&m| m == denoise).unwrap();
            measure_held_at(&format!("{label} @ {}", order.label), &shown, mode, denoise, give_back);
        }
    }
}

/// A Processing Resolution (`preview_resolution`: binned before the pipeline) and a
/// Streaming Resolution (shrunk in the encoder, after the stretch) a stack is rendered at.
#[derive(Clone, Copy, Debug)]
pub struct RenderOrder {
    pub label: &'static str,
    pub processing: night_amplifier::session::state::Resolution,
    pub stream: night_amplifier::session::state::Resolution,
}

impl RenderOrder {
    /// The default settings at this order's Processing Resolution.
    pub fn settings(&self) -> night_amplifier::session::state::CaptureSettings {
        night_amplifier::session::state::CaptureSettings { preview_resolution: self.processing, ..Default::default() }
    }
}

/// What a real session is rendered at: a 1440p stream at the default Native processing,
/// which the filters were tuned on; the same with 1440p processing, binned before the
/// stretch as the render task does then;
/// and the default Native (up to `MAX_STREAM_EDGE`), where every processed pixel is shown.
pub const RENDER_ORDERS: [RenderOrder; 3] = {
    use night_amplifier::session::state::Resolution::{Native, Qhd1440};
    [
        RenderOrder { label: "1440p", processing: Native, stream: Qhd1440 },
        RenderOrder { label: "1440p-binned", processing: Qhd1440, stream: Qhd1440 },
        RenderOrder { label: "Native", processing: Native, stream: Native },
    ]
};

/// How [`measure_real_session`] judges a target's contrast against a shallower stack's.
#[derive(Clone, Copy, Debug)]
pub enum GiveBack {
    /// Never more than 2 output levels below the best so far.
    Banded,
    /// [`Self::Banded`] on the target its test names rather than the frame's brightest
    /// block, which on a faint target's frame is a star or the glow.
    Named(NamedTarget),
}

/// Where a [`GiveBack::Named`] target is, in fractions of the `frame` (stack size, px) it
/// was plate-solved on. The size is checked against the stack: fractions copied from a
/// crop of IC 59 once put the test's IC 63 on the glow 0.46 of a frame away.
#[derive(Clone, Copy, Debug)]
pub struct NamedTarget {
    pub centre: (f64, f64),
    pub frame: (usize, usize),
}

impl NamedTarget {
    /// The target's [`box_at`] block in `stack` rendered at `w x h`.
    pub fn block(&self, stack: &Frame, (w, h): (usize, usize)) -> PixelBox {
        self.check((stack.width(), stack.height()));
        box_at(self.centre, w, h)
    }

    /// [`Self::block`] on `held`'s `w x h` render, through its crop.
    pub fn block_on(&self, held: &HeldRender, (w, h): (usize, usize)) -> PixelBox {
        self.check(held.stack);
        let (x, y) = held.to_render((self.centre.0 * self.frame.0 as f64, self.centre.1 * self.frame.1 as f64), w);
        box_at((x / w as f64, y / h as f64), w, h)
    }

    fn check(&self, stack: (usize, usize)) {
        assert_eq!(stack, self.frame, "the target was located on another frame size; plate-solve this stack");
    }
}

/// [`measure_real_session`]'s assertions on one order's `shown` depths, read from each one's
/// `mode`th render, the boxes found on the deepest's plain render and moved through each crop.
fn measure_held_at(label: &str, shown: &[&HeldRender], mode: usize, denoise: bool, give_back: GiveBack) {
    let deepest = shown.last().unwrap();
    let (deep_rgb, w, h) = &deepest.renders[0];
    let (sky, mut target) = locate_boxes(deep_rgb, *w, *h);
    if let GiveBack::Named(named) = give_back {
        target = named.block_on(deepest, (*w, *h));
    }

    {
        println!(
            "\n=== {label}, denoise {} (sky {sky:?}, target {target:?} at {} subs) ===",
            if denoise { "default" } else { "off" },
            deepest.depth
        );
        println!("   N   sigma(ADU)  sky lvl  grain(lvl)  target(lvl)  target/grain  N^-1/4  crop");
        let mut rows = Vec::new();
        for held in shown {
            let (rgb8, rw, rh) = &held.renders[mode];
            let at = |b| held.block_from(b, deepest, *w, (*rw, *rh));
            let m = measure_sky_and_target(rgb8, *rw, at(sky), at(target));
            println!(
                "{:>4}   {:>9.2}   {:>6.1}   {:>9.2}   {:>10.1}   {:>11.2}   {:>7.2}  {}",
                held.depth,
                held.stack_sigma * 65535.0,
                m.sky_level,
                m.sky_grain,
                m.target_contrast,
                m.target_contrast / m.sky_grain.max(1e-9),
                expected_grain_ratio(held.depth),
                held.crop.map_or("-".to_string(), |r| format!("{}x{}", r.width, r.height))
            );
            rows.push((held.depth, m));
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
            // With the filters on, neither bound above means what it says. The wavelet holds
            // the sky near its floor from the *first* sub — 1.41 output levels at N=1 on the
            // 106-sub set against 5.70 with it off — so there is almost nothing left for
            // depth to take, and what remains drifts *up* as residual noise migrates to
            // coarse scales a 4-level transform only partly reaches (1.41 -> 1.66 over 106
            // subs). Normalising against N=1 is misleading since the ratio starts from the filter's best case.
            //
            // This half asserts the absolute state instead: the sky stays smooth at every depth and the target grows.
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

        // The target is what the depth is for, so it must not be spent down to buy the sky. The
        // synthetic sweep asserts it rises outright; a real session gets a band, since its sigma
        // doesn't fall as sqrt(N) and the measurement is a 96 px block median quantised to whole
        // output levels (1-2 % of the numbers here) — against the best *so far*, not the sweep's
        // maximum, since contrast keeps climbing and only a giveback below an earlier depth is a
        // defect. `MAX_GAIN_DEPTH` at 256: the 106-sub set peaked at 32 subs and gave back 77 ->
        // 69 levels (10 %) by the end; the cap at 64 gives back none on either set.
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

// ---------------------------------------------------------------------------
// 10. Star size
// ---------------------------------------------------------------------------

/// Median half-flux radius of a frame's stars, whole frame and two zones.
#[derive(Debug, Clone, Copy)]
pub struct StarSize {
    pub all: f32,
    /// Within 35 % of the way from the centre to a corner, where a rotation error does least.
    pub centre: f32,
    /// Beyond 60 % of the way to a corner (~43 % of the frame): a rotation error grows
    /// with distance from the centre, so it shows here first.
    pub corners: f32,
    pub stars: usize,
    pub centre_stars: usize,
    pub corner_stars: usize,
}

/// [`half_flux_radii`] at the frame's own [`measurable_stars`], summarised by zone.
///
/// `compute_median_fwhm` counts pixels above half maximum and read 2.3 px on subs whose stars
/// hold half their flux inside 3.7; a misaligned or doubled sub shows only as width in the
/// stack, corners first.
pub fn half_flux_radius(frame: &Frame) -> StarSize {
    let at = measurable_stars(frame);
    let radii = half_flux_radii(frame, &at);
    StarSize::of(&radii, &at, frame.width(), frame.height())
}

/// Window radius the flux is gathered over, and the sky ring's width outside it.
const HFR_WINDOW: i32 = 10;
const HFR_RING: i32 = 3;

/// The brightest unsaturated stars (≤ 120) with no neighbour inside 25 px.
pub fn measurable_stars(frame: &Frame) -> Vec<(f32, f32)> {
    let stars = night_amplifier::detection::detect_stars_adaptive(frame).unwrap_or_default();
    stars
        .iter()
        .filter(|s| s.peak < 0.7)
        .take(120)
        .filter(|s| !stars.iter().any(|o| o != *s && o.distance_to(s) < 2.5 * HFR_WINDOW as f32))
        .map(|s| (s.x, s.y))
        .collect()
}

/// Radius holding half of the background-subtracted flux around each star near `at`, on
/// mean luminance; `None` where the window leaves the frame or holds no flux.
///
/// Measured about the star's own centroid, found near `at`, not about `at` itself: a
/// position off by a pixel would read as width, and a caller placing stars by a
/// registration transform would then see that transform's error on both sides of a
/// comparison and never notice it.
///
/// Half-flux, not FWHM or a second moment: a moment weights by r² and so reads the sky
/// noise inside the window as width (a single sub measured 7 px by moments against 3.7 by
/// half-flux), which buries the fraction of a pixel a registration change moves.
pub fn half_flux_radii(frame: &Frame, at: &[(f32, f32)]) -> Vec<Option<f32>> {
    let lum = night_amplifier::detection::mean_luminance(frame);
    let (w, h) = (frame.width(), frame.height());
    at.iter()
        .map(|&(x, y)| {
            let (cx, cy) = star_window(&lum, w, h, x, y)?.centroid();
            star_window(&lum, w, h, cx, cy)?.half_flux_radius()
        })
        .collect()
}

/// The sky-subtracted samples within [`HFR_WINDOW`] of a position, as `(dx, dy, value)`
/// relative to it.
struct StarWindow {
    samples: Vec<(f32, f32, f32)>,
    x: f32,
    y: f32,
}

fn star_window(lum: &[f32], w: usize, h: usize, x: f32, y: f32) -> Option<StarWindow> {
    const R: i32 = HFR_WINDOW;
    let margin = R + HFR_RING;
    let (cx, cy) = (x.round() as i32, y.round() as i32);
    if cx < margin || cy < margin || cx >= w as i32 - margin || cy >= h as i32 - margin {
        return None;
    }
    let sample = |dx: i32, dy: i32| lum[(cy + dy) as usize * w + (cx + dx) as usize];
    let mut ring = Vec::new();
    for dy in -margin..=margin {
        for dx in -margin..=margin {
            let r2 = dx * dx + dy * dy;
            if r2 > R * R && r2 <= margin * margin {
                ring.push(sample(dx, dy));
            }
        }
    }
    ring.sort_by(f32::total_cmp);
    let sky = ring[ring.len() / 2];
    let mut samples = Vec::new();
    for dy in -R..=R {
        for dx in -R..=R {
            let (ox, oy) = (cx as f32 + dx as f32 - x, cy as f32 + dy as f32 - y);
            if ox.hypot(oy) <= R as f32 {
                samples.push((ox, oy, sample(dx, dy) - sky));
            }
        }
    }
    Some(StarWindow { samples, x, y })
}

impl StarWindow {
    /// Flux-weighted centre of the window's positive samples.
    fn centroid(&self) -> (f32, f32) {
        let (mut m0, mut mx, mut my) = (0.0, 0.0, 0.0);
        for &(dx, dy, v) in self.samples.iter().filter(|s| s.2 > 0.0) {
            m0 += v;
            mx += v * dx;
            my += v * dy;
        }
        if m0 <= 0.0 {
            return (self.x, self.y);
        }
        (self.x + mx / m0, self.y + my / m0)
    }

    fn half_flux_radius(&self) -> Option<f32> {
        let total: f32 = self.samples.iter().map(|s| s.2).sum();
        if total <= 0.0 {
            return None;
        }
        let mut by_radius: Vec<(f32, f32)> =
            self.samples.iter().map(|&(dx, dy, v)| (dx.hypot(dy), v)).collect();
        by_radius.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut held = 0.0;
        by_radius
            .iter()
            .find(|p| {
                held += p.1;
                held >= 0.5 * total
            })
            .map(|p| p.0)
    }
}

impl StarSize {
    /// Medians of `radii` over the whole frame and its zones, placing each star by `at`
    /// in a `width` x `height` frame.
    pub fn of(radii: &[Option<f32>], at: &[(f32, f32)], width: usize, height: usize) -> Self {
        let (mut all, mut centre, mut corners) = (Vec::new(), Vec::new(), Vec::new());
        for (radius, &(x, y)) in radii.iter().zip(at) {
            let Some(radius) = *radius else { continue };
            all.push(radius);
            let (dx, dy) = (x - width as f32 / 2.0, y - height as f32 / 2.0);
            let reach = dx.hypot(dy) / (width as f32 / 2.0).hypot(height as f32 / 2.0);
            if reach < 0.35 {
                centre.push(radius);
            }
            if reach > 0.6 {
                corners.push(radius);
            }
        }
        let median = |mut v: Vec<f32>| {
            if v.is_empty() {
                return f32::NAN;
            }
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        Self {
            stars: all.len(),
            centre_stars: centre.len(),
            corner_stars: corners.len(),
            all: median(all),
            centre: median(centre),
            corners: median(corners),
        }
    }
}
