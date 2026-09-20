//! The measurement instruments, shared by both repos.
//!
//! **Nothing in this file may name `crate::`.** Pro's integration tests pull it in with
//! `#[path = "../../night-amplifier/tests/integration/instruments.rs"] mod instruments;`
//! the way they already pull in `common.rs`, and that only works while the module is
//! reachable from either crate's root. Fixture discovery therefore arrives as a
//! parameter rather than by calling `common` — the instruments do not need to know how
//! the frames got onto the disk.
//!
//! Four instruments, and each exists because a single number misled a real fix:
//!
//! 1. **Octave-band sky noise** ([`octave_bands`]). One global grain figure misled three
//!    consecutive changes; an observer reads grain at 8-128 px and the bands either side
//!    of that move independently.
//! 2. **The star radial profile** ([`radial_excess`]). The only thing that catches
//!    ringing. A dip at r=5-9 or a halo at r=13-25 is a defect however good the grain is.
//! 3. **Centre against edge** ([`centre_and_edge`]). A whole-frame number averages the
//!    stack's border into its middle and shows almost nothing, which is precisely the
//!    difference a per-pixel noise map is meant to remove.
//! 4. **Line coherence** ([`line_coherence`]). A 1440-px row is 40 degrees long at this
//!    eyepiece and the eye integrates along it, so residual banding outranks grain as the
//!    visible defect at an amplitude far below the grain floor. No RMS can see it.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use night_amplifier::Frame;

/// One sub as it comes off the disk, before the raw-CFA stage.
///
/// The caller's loader decides how — FITS, TIFF, whatever the fixture set holds — and
/// says whether the frame is still mosaiced.
pub struct RawSub {
    pub frame: Frame,
    pub is_bayer: bool,
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

/// Stacks a real session, handing back `(depth, stack)` at each requested depth.
///
/// Frames go through the capture path's raw-CFA stage (hot pixels, row/column FPN) and
/// demosaic, as the capture task runs them: a bare debayer leaves every hot pixel in, and
/// registration drags each one into a dotted trail along the session's drift.
pub fn stack_snapshots(
    files: &[PathBuf],
    depths: &[usize],
    load: &dyn Fn(&Path) -> RawSub,
) -> Vec<(usize, Frame)> {
    with_stack(files, depths, load, |ctx| ctx.compute().unwrap())
}

/// [`stack_snapshots`], also handing back the accumulator's per-pixel noise map.
///
/// The map is only on the accumulator, not on the frame `compute` returns, so anything
/// measuring it has to reach the `StackingContext` rather than a snapshot of its mean.
pub fn stack_snapshots_with_noise(
    files: &[PathBuf],
    depths: &[usize],
    load: &dyn Fn(&Path) -> RawSub,
) -> Vec<(usize, Frame, night_amplifier::NoiseField)> {
    let mut out = Vec::new();
    with_stack(files, depths, load, |ctx| {
        let (frame, field) = ctx.compute_with_noise().unwrap();
        out.push((ctx.frame_count(), frame.clone(), field));
        frame
    });
    out
}

fn with_stack(
    files: &[PathBuf],
    depths: &[usize],
    load: &dyn Fn(&Path) -> RawSub,
    mut snapshot: impl FnMut(&night_amplifier::server::capture::StackingContext) -> Frame,
) -> Vec<(usize, Frame)> {
    use night_amplifier::server::capture::pipeline::{build_cfa_pipeline, debayer_algorithm};
    use night_amplifier::server::capture::StackingContext;

    let settings = night_amplifier::server::state::CaptureSettings::default();
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
        let mut cfa = night_amplifier::CfaFrame::mosaic(img.frame, p).unwrap();
        cfa_pipeline.apply(&mut cfa);
        cfa.debayer(algorithm).unwrap()
    };

    let reference = prepare(&files[0]);
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
            snapshots.push((ctx.frame_count(), snapshot(&ctx)));
            next += 1;
        }
        let _ = ctx.add_frame(&prepare(path));
    }
    if snapshots.last().map(|s| s.0) != Some(ctx.frame_count()) {
        snapshots.push((ctx.frame_count(), snapshot(&ctx)));
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
    settings: &night_amplifier::server::state::CaptureSettings,
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
    mut frame: Frame,
    settings: &night_amplifier::server::state::CaptureSettings,
    denoise: bool,
    max: (u32, u32),
    stack_depth: u32,
    noise: Option<night_amplifier::NoiseField>,
    tweak: impl FnOnce(&mut night_amplifier::render::RenderPipelineConfig),
) -> (Vec<u8>, usize, usize) {
    use night_amplifier::server::capture::{AnalysisContext, PreviewAnalysis};

    let night_amplifier::server::capture::pipeline::PreviewRender {
        mut pipeline_config,
        stretch_result,
        linear_gain,
    } = night_amplifier::server::capture::pipeline::process_preview_frame_with_analysis(
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
        // Scaled exactly as the render task scales it, or the map would describe the
        // frame as it was before background neutralisation multiplied it.
        noise: noise.map(|field| std::sync::Arc::new(field.scaled(&linear_gain))),
        linear_frame: std::sync::Arc::new(frame),
        pipeline_config,
        stretch_result,
    };
    let (bytes, w, h) =
        night_amplifier::server::encoding::frame_to_rgb8_downsampled(&ready, max.0, max.1).unwrap();
    (bytes, w as usize, h as usize)
}

// ---------------------------------------------------------------------------
// Instrument 1: octave-band sky noise
// ---------------------------------------------------------------------------

/// Band labels [`octave_bands`] reports, finest first.
pub const OCTAVE_LABELS: [&str; 7] = [
    "1-2", "2-4", "4-8", "8-16", "16-32", "32-64", "64-128",
];

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
    let mad_sigma = deviations[deviations.len() / 2] * 1.4826;

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

/// Sky noise split into octave bands by successive box blurs, finest first.
///
/// Band `i` is what a `2^i`-wide blur removed, so the seven figures sum in quadrature to
/// roughly the total. `plane` is one channel at `width x height`, in output levels.
pub fn octave_bands(plane: &[f64], width: usize, height: usize) -> [f64; 7] {
    let mut bands = [0.0f64; 7];
    let mut current = plane.to_vec();
    for (i, band) in bands.iter_mut().enumerate() {
        let blurred = box_blur(&current, width, height, 1 << i);
        let detail: Vec<f64> = current.iter().zip(&blurred).map(|(a, b)| a - b).collect();
        *band = clipped_sigma(&detail);
        current = blurred;
    }
    bands
}

fn box_blur(plane: &[f64], width: usize, height: usize, radius: usize) -> Vec<f64> {
    let mut rows = vec![0.0f64; plane.len()];
    for y in 0..height {
        for x in 0..width {
            let (x0, x1) = (x.saturating_sub(radius), (x + radius + 1).min(width));
            let sum: f64 = plane[y * width + x0..y * width + x1].iter().sum();
            rows[y * width + x] = sum / (x1 - x0) as f64;
        }
    }
    let mut out = vec![0.0f64; plane.len()];
    for y in 0..height {
        let (y0, y1) = (y.saturating_sub(radius), (y + radius + 1).min(height));
        for x in 0..width {
            let sum: f64 = (y0..y1).map(|yy| rows[yy * width + x]).sum();
            out[y * width + x] = sum / (y1 - y0) as f64;
        }
    }
    out
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

/// Octave bands of a centre crop and an edge crop, separately.
///
/// **A whole-frame figure cannot answer the question a noise map is asked.** Fewer subs
/// overlap at the stack's border, so noise there is higher by `sqrt(N/count)` — and a
/// single number averages that border into the middle and reports almost no change
/// either way. `fraction` sizes both crops as a share of the shorter axis; the edge crop
/// sits against the left margin, where registration drift leaves the thinnest coverage.
pub fn centre_and_edge(
    rgb8: &[u8],
    width: usize,
    height: usize,
    channel: usize,
    fraction: f64,
) -> (([f64; 7], f64), ([f64; 7], f64)) {
    let plane = channel_plane(rgb8, channel);
    let span = ((width.min(height) as f64 * fraction) as usize).clamp(16, width.min(height));

    let cx = width / 2 - span / 2;
    let cy = height / 2 - span / 2;
    let (centre_plane, cw, ch) = crop(&plane, width, (cx, cy, cx + span, cy + span));

    let ey = height / 2 - span / 2;
    let (edge_plane, ew, eh) = crop(&plane, width, (0, ey, span, ey + span));

    (
        (octave_bands(&centre_plane, cw, ch), clipped_sigma(&centre_plane)),
        (octave_bands(&edge_plane, ew, eh), clipped_sigma(&edge_plane)),
    )
}

// ---------------------------------------------------------------------------
// Instrument 2: the star radial profile
// ---------------------------------------------------------------------------

/// Radii the profile is reported at.
pub const PROFILE_RADII: [usize; 12] = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23];

/// Mean excess over the local sky at each of [`PROFILE_RADII`], around the given stars.
///
/// The one instrument that catches ringing: a **dip** at r=5-9 or a **halo** at r=13-25
/// is a defect even when the grain numbers look fine. Reference shape on M27, code as of
/// 2026-09-19: `94.3, 11.0, 2.5, 1.7, 1.3, 1.3, 0.8, 0.7, 0.3, 0.8, 0.8, 1.0`.
pub fn radial_excess(
    plane: &[f64],
    width: usize,
    height: usize,
    stars: &[(usize, usize)],
) -> [f64; PROFILE_RADII.len()] {
    let surround = surround_level(plane, width, height, stars);
    let mut out = [0.0; PROFILE_RADII.len()];
    for (slot, &r) in out.iter_mut().zip(&PROFILE_RADII) {
        let mut total = 0.0;
        let mut count = 0usize;
        for &(sx, sy) in stars {
            for (dx, dy) in ring_offsets(r) {
                let (x, y) = (sx as isize + dx, sy as isize + dy);
                if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
                    continue;
                }
                total += plane[y as usize * width + x as usize];
                count += 1;
            }
        }
        *slot = if count > 0 { total / count as f64 - surround } else { 0.0 };
    }
    out
}

/// The sky each star sits on: an annulus well outside the profile's reach.
fn surround_level(plane: &[f64], width: usize, height: usize, stars: &[(usize, usize)]) -> f64 {
    let mut samples = Vec::new();
    for &(sx, sy) in stars {
        for r in [31usize, 35, 39] {
            for (dx, dy) in ring_offsets(r) {
                let (x, y) = (sx as isize + dx, sy as isize + dy);
                if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
                    continue;
                }
                samples.push(plane[y as usize * width + x as usize]);
            }
        }
    }
    if samples.is_empty() {
        return 0.0;
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

fn ring_offsets(r: usize) -> Vec<(isize, isize)> {
    let r = r as isize;
    let mut out = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            let d = ((dx * dx + dy * dy) as f64).sqrt();
            if (d - r as f64).abs() < 0.5 {
                out.push((dx, dy));
            }
        }
    }
    out
}

/// The brightest well-separated local maxima of a plane, as `(x, y)`.
pub fn bright_stars(plane: &[f64], width: usize, height: usize, want: usize) -> Vec<(usize, usize)> {
    const MARGIN: usize = 48;
    const SEPARATION: usize = 64;
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for y in MARGIN..height.saturating_sub(MARGIN) {
        for x in MARGIN..width.saturating_sub(MARGIN) {
            let v = plane[y * width + x];
            let peak = (-1isize..=1).all(|dy| {
                (-1isize..=1).all(|dx| {
                    let (nx, ny) = ((x as isize + dx) as usize, (y as isize + dy) as usize);
                    v >= plane[ny * width + nx]
                })
            });
            if peak {
                candidates.push((v, x, y));
            }
        }
    }
    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    let mut picked: Vec<(usize, usize)> = Vec::new();
    for (_, x, y) in candidates {
        if picked.len() == want {
            break;
        }
        let clear = picked.iter().all(|&(px, py)| {
            x.abs_diff(px) >= SEPARATION || y.abs_diff(py) >= SEPARATION
        });
        if clear {
            picked.push((x, y));
        }
    }
    picked
}

// ---------------------------------------------------------------------------
// Instrument 4: line coherence
// ---------------------------------------------------------------------------

/// How much coherent row/column structure a sky carries, in output levels.
///
/// Per-row and per-column means with stars excluded by an upper threshold, then the
/// robust sigma of *those means* after removing a low-order fit. Isotropic noise of sigma
/// `s` over `W` columns contributes `s/sqrt(W)` to this (the `floor` returned alongside);
/// anything meaningfully above that floor is coherent banding.
///
/// **A grain metric cannot see this quantity at all**, and the eye is disproportionately
/// good at it: a 1440-px row is 40 degrees long at this eyepiece and the eye integrates
/// along it, so banding well below the grain floor is still visible.
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
// Reporting
// ---------------------------------------------------------------------------

pub fn octave_header() -> String {
    OCTAVE_LABELS.iter().map(|l| format!("{l:>7}")).collect::<Vec<_>>().join(" ")
}

pub fn format_bands(bands: &[f64; 7]) -> String {
    bands.iter().map(|b| format!("{b:7.3}")).collect::<Vec<_>>().join(" ")
}

pub fn profile_header() -> String {
    PROFILE_RADII.iter().map(|r| format!("{:>6}", format!("r{r}"))).collect::<Vec<_>>().join(" ")
}

pub fn format_profile(profile: &[f64]) -> String {
    profile.iter().map(|v| format!("{v:6.1}")).collect::<Vec<_>>().join(" ")
}
