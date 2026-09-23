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
    dev[dev.len() / 2] * 1.4826
}

/// Sky noise split into octaves, as robust sigma in output levels per band.
///
/// Band `k` is `blur(edge[k]) - blur(edge[k+1])`, so it holds the detail between those
/// two scales and nothing else. The eye's grain sits in the last four bands.
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
/// Positions are found once on the *un-denoised* render and reused for every variant, so
/// a profile difference is the render's and not the finder's.
///
/// Three rejections, all of which were needed before the profile looked like a star at
/// all on M27's dense field: a maximum must stand clear of its own 34-46 px surround
/// (not sit on nebulosity, which is most of the frame's bright local maxima), it must
/// actually be point-like by r=9, and nothing else accepted may sit inside the keep-out.
/// Point-likeness is judged on this render, which has no spatial filter applied — so it
/// selects genuine point sources without pre-selecting the shape under test.
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

/// The four sessions the render's brightness was tuned against. They fail differently:
/// M27's dense field is the worst case for anything spatial, Andromeda for anything
/// touching extended low-contrast structure, Orion is another sensor and a much
/// shallower stack.
///
/// Out of the repo, so these are diagnostics rather than guards, driven by
/// `RENDER_BRIGHTNESS_SETS` (a comma-separated subset) and cached as stacked FITS under
/// `RENDER_BRIGHTNESS_CACHE` — stacking 266 subs takes far longer than rendering them,
/// and the same stacks are measured under a dozen settings.
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
fn session_frames(dir: &Path) -> Option<Vec<PathBuf>> {
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
    let (stacked, stack) = stack_snapshots(&files, &[], &load_sub).pop()?;
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
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
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
    let settings = night_amplifier::server::state::CaptureSettings::default();
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

pub fn measure_sky_and_target(rgb8: &[u8], width: usize, sky: (usize, usize, usize, usize), target: (usize, usize, usize, usize)) -> Measured {
    let sky_px = crop_rgb8(rgb8, width, sky);
    let target_px = crop_rgb8(rgb8, width, target);
    let sky_level = green_median(&sky_px);
    Measured {
        sky_grain: sky_sigma_levels(&sky_px, 1),
        sky_level,
        target_contrast: green_median(&target_px) - sky_level,
    }
}

/// Darkest and brightest 96 px block of the green channel, by median.
pub fn locate_boxes(rgb8: &[u8], w: usize, h: usize) -> ((usize, usize, usize, usize), (usize, usize, usize, usize)) {
    const B: usize = 96;
    let mut darkest = (f64::MAX, (0, 0, B, B));
    let mut brightest = (f64::MIN, (0, 0, B, B));
    // Stay off the edges: registration leaves a partially covered border.
    for y in (B..h - 2 * B).step_by(B / 2) {
        for x in (B..w - 2 * B).step_by(B / 2) {
            let bx = (x, y, x + B, y + B);
            let med = green_median(&crop_rgb8(rgb8, w, bx));
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
    load: &dyn Fn(&Path) -> RawSub,
) {
    if std::env::var("STACK_GRAIN_TRACE").is_ok() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("night_amplifier::render::autostretch=debug")
            .with_test_writer()
            .try_init();
    }
    let settings = night_amplifier::server::state::CaptureSettings::default();
    let snapshots = stack_snapshots(files, depths, load);

    let deepest = snapshots.last().unwrap();
    let (deep_rgb, w, h) =
        render(deepest.1.clone(), &settings, false, (2560, 1440), deepest.0 as u32);
    let (sky, target) = locate_boxes(&deep_rgb, w, h);

    for &denoise in denoise_modes {
        println!(
            "\n=== {label}, denoise {} (sky {sky:?}, target {target:?}) ===",
            if denoise { "default" } else { "off" }
        );
        println!("   N   sigma(ADU)  sky lvl  grain(lvl)  target(lvl)  target/grain  N^-1/4");
        let mut rows = Vec::new();
        for (n, stack) in &snapshots {
            let stats = night_amplifier::statistics::compute_image_stats(stack).unwrap();
            let (rgb8, w, _) = render(stack.clone(), &settings, denoise, (2560, 1440), *n as u32);
            let m = measure_sky_and_target(&rgb8, w, sky, target);
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
