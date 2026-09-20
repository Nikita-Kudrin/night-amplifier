//! Instruments for the render's brightness-against-grain trade, and the guard that
//! catches ringing around stars.
//!
//! A single global grain number is not enough to judge a change here — it agreed with
//! three separate fixes that each turned out to be a defect. Two measurements are
//! needed together:
//!
//! 1. **Octave-band sky noise** ([`octave_band_sigma`]). Perceived grain lives at
//!    8-128 px. A fine-scale metric (adjacent-pixel differences, or an integer MAD that
//!    quantises to whole output levels) is blind in exactly that band, so a filter that
//!    moves mottle from 4 px to 64 px reads as an improvement.
//! 2. **Radial profile around bright isolated stars** ([`radial_excess`]). The only
//!    thing that catches ringing: a *dip* (negative excess at r=5-9) or a *halo*
//!    (elevated excess at r=13-25). Both were measured on fixes whose total grain
//!    looked fine — `sky_shadow` lit a +2.3 level disc at r=21, coarse wavelet levels
//!    dug a -2.5 level trough at r=5.
//!
//! Object brightness is read at three radii, not just the core ([`object_excess`]):
//! faint outer structure is what observers notice and it moves differently.

use std::path::{Path, PathBuf};

use serial_test::serial;

use crate::integration::stack_depth_grain_tests::{
    managed_session, render, stack_snapshots,
};

/// The managed fixture set the ring guard runs on: 35 IMX533 subs of M27, whose dense
/// Milky Way field is the worst case for anything spatial.
const RING_FIXTURE_SET: &str = "250mm-dob-imx533-dumbbell-fits";

/// Stream size every instrument measures at — the resolution an observer is actually
/// served, so the encoders' box downsample has already had its share of the noise.
///
/// `STREAM_MAX` overrides it. That box downsample is not neutral to this measurement: it
/// averages 2x2 or more before the filters run, so a change to the *finest* wavelet scale
/// looks far smaller here than it does in the saved full-resolution PNG.
const STREAM: (u32, u32) = (2560, 1440);

fn stream() -> (u32, u32) {
    match std::env::var("STREAM_MAX").ok().and_then(|v| v.parse::<u32>().ok()) {
        Some(n) => (n, n),
        None => STREAM,
    }
}

// ---------------------------------------------------------------------------
// Instrument 1: octave-band sky noise
// ---------------------------------------------------------------------------

/// Band edges in pixels: each band is the detail between two successive box blurs.
pub(crate) const OCTAVE_EDGES: [usize; 8] = [1, 2, 4, 8, 16, 32, 64, 128];

/// Human labels for [`octave_band_sigma`]'s output, in the same order.
pub(crate) const OCTAVE_LABELS: [&str; 7] =
    ["1-2", "2-4", "4-8", "8-16", "16-32", "32-64", "64-128"];

/// The green plane of `region`, as f64 output levels.
///
/// Green rather than luminance: it carries two of the four Bayer sites, so it is the
/// channel the sky's noise is measured on everywhere else in this suite.
pub(crate) fn green_region(
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
pub(crate) fn octave_band_sigma(plane: &[f64], w: usize, h: usize) -> [f64; 7] {
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
pub(crate) fn format_octaves(bands: &[f64; 7]) -> String {
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
pub(crate) const PROFILE_RADII: [usize; 13] = [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25];

/// Where a star's own light has certainly stopped, so its sky can be read.
const SKY_ANNULUS: (f64, f64) = (34.0, 46.0);

/// Keep-out radius between accepted stars: no second star may sit inside the sky
/// annulus, or one star's wings become another's baseline.
const ISOLATION: usize = 50;

pub(crate) struct Star {
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
pub(crate) fn find_isolated_stars(plane: &[f64], w: usize, h: usize, count: usize) -> Vec<Star> {
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
pub(crate) fn radial_excess(
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
pub(crate) fn radial_excess_and_surround(
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

pub(crate) fn format_profile(profile: &[f64; PROFILE_RADII.len()]) -> String {
    profile
        .iter()
        .map(|v| format!("{v:>7.1}"))
        .collect::<Vec<_>>()
        .join("")
}

pub(crate) fn profile_header() -> String {
    PROFILE_RADII
        .iter()
        .map(|r| format!("{:>7}", format!("r{r}")))
        .collect::<Vec<_>>()
        .join("")
}

// ---------------------------------------------------------------------------
// Object brightness at three radii
// ---------------------------------------------------------------------------

pub(crate) struct ObjectExcess {
    pub core: f64,
    pub mid: f64,
    pub outer: f64,
}

/// Median excess over `sky_level` in three annuli about `(cx, cy)`.
///
/// Medians, not means, so a star inside the annulus cannot carry it. The radii are
/// fractions of `span` — the object's own extent — so the same call describes a small
/// planetary nebula and a galaxy that fills the frame.
pub(crate) fn object_excess(
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
pub(crate) fn plane_median(plane: &[f64]) -> f64 {
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
pub(crate) fn locate_object(
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
// The guard
// ---------------------------------------------------------------------------

/// Denoising must not ring: no trough just outside a star, and no halo further out.
///
/// This is the test that was missing while three separate fixes shipped and were
/// reverted. Each of them left total sky grain looking fine — the defect was entirely
/// in the shape of the profile around stars, which nothing measured.
///
/// The un-denoised render is the reference, not an absolute bound: real stars have real
/// wings, and how far they reach is the optics' business. What the filters may not do is
/// take signal out from under a star (a dip) or put light where there was none (a halo).
#[test]
#[serial]
#[ignore = "integration test - run with: cargo test --test integration_pipeline -- --ignored --test-threads=1"]
fn a_bright_star_keeps_no_ring() {
    let files = managed_session(RING_FIXTURE_SET);
    let stack = stack_snapshots(&files, &[]).pop().expect("a stack").1;
    let depth = files.len() as u32;
    let settings = night_amplifier::server::state::CaptureSettings::default();

    let (plain, w, h) = render(stack.clone(), &settings, false, STREAM, depth);
    let whole = (0, 0, w, h);
    let (plain_plane, pw, ph) = green_region(&plain, w, whole);

    let stars = find_isolated_stars(&plain_plane, pw, ph, 25);
    assert!(
        stars.len() >= 8,
        "only {} isolated stars found on {RING_FIXTURE_SET}; the profile is not \
         measurable and this guard would pass vacuously",
        stars.len()
    );

    println!("\n=== {RING_FIXTURE_SET}, {} stars, {depth} subs ===", stars.len());
    println!("              {}", profile_header());

    // Every configuration that denoises: the default, two positions on the Background
    // Grain dial's coarse half where wavelet levels 5-6 run, and Star Fields — the only
    // profile that *amplifies* the fine scales — on its own and with the coarse levels
    // underneath it. Each has a history here: the coarse levels move a star's flux
    // outward into a disc if thresholded carelessly, and the Star Fields gain digs a
    // moat around every star if pushed past ~1.4. The two mechanisms were tuned
    // separately, so the row that runs both is the one with no measurement behind it.
    for (label, dial, star_field) in [
        ("default", 0.5f32, false),
        // Mid-travel on the coarse half, where the levels run at a fraction of
        // `COARSE_K` — a shrinkage that only misbehaves at full strength would pass a
        // test that sampled the two ends.
        ("dial 75 %", 0.75, false),
        ("dial 100 %", 1.0, false),
        ("star fields", 0.5, true),
        // The combination nothing else covers: the fine threshold at
        // `STAR_FIELD_FINE_BOOST`, `STAR_FIELD_GAIN` sharpening levels 1-3, and the
        // coarse pair thresholding underneath them. Each half is tuned against the
        // other being off.
        ("star fields, dial 100 %", 1.0, true),
    ] {
        let mut settings = settings.clone();
        settings.denoise.background_grain = dial;
        if star_field {
            settings.stretch_aggressiveness =
                night_amplifier::render::StretchAggressiveness::Low;
        }
        // Each profile is compared against its **own** un-denoised render. Star Fields
        // uses a different tone curve, so measuring it against Deep Sky's reference
        // reports a 26 % lower star core with nothing wrong at all.
        let (plain, _, _) = render(stack.clone(), &settings, false, stream(), depth);
        let (plain_plane, _, _) = green_region(&plain, w, whole);
        let (reference, plain_surround) =
            radial_excess_and_surround(&plain_plane, pw, ph, &stars);
        let (denoised, dw, dh) = render(stack.clone(), &settings, true, stream(), depth);
        assert_eq!((w, h), (dw, dh), "both renders must be the same size");
        let (denoised_plane, _, _) = green_region(&denoised, w, whole);
        let (measured, surround) = radial_excess_and_surround(&denoised_plane, pw, ph, &stars);
        println!(
            "{:<13} {}  surround {plain_surround:+.2}",
            format!("{label} off"),
            format_profile(&reference)
        );
        println!("{label:<13} {}  surround {surround:+.2}", format_profile(&measured));

        // A dip: signal removed from under the star. The coarse-wavelet attempt read
        // -2.5 levels at r=5, so half a level of margin separates the defect from the
        // 8-bit quantisation of a profile averaged over ~25 stars.
        for (i, &r) in PROFILE_RADII.iter().enumerate() {
            if r >= 10 {
                continue;
            }
            assert!(
                measured[i] > -0.5,
                "{label}: denoising dug a trough of {:.1} output levels at r={r} px — a \
                 star's own light is being removed as noise",
                measured[i]
            );
        }

        // A lit disc, measured as each star's own surround against the field sky —
        // **not** as its excess at r=13-25 against the un-denoised render. That
        // comparison looks like a halo here and is not one: flattening the sky a star
        // sits in is the filter's whole job, so its real wings legitimately stand out
        // more afterwards (r=13 reads 2.9 levels denoised against 0.8 plain on this
        // fixture, with no light added anywhere). What a disc does instead is lift the
        // sky *around* stars relative to sky far from any — `sky_shadow` by +2.3 output
        // levels at r=21 px, which lands inside this annulus.
        //
        // Bounds from the measured spread on this fixture: across the shipped settings
        // and five deliberately extreme ones (darker sky at -5 % soft and hard, -6 % with
        // the eyepiece at full, double wavelet strength, the grain dial at 0) the
        // surround stayed inside -0.12..+0.32 output levels, and the coarse levels at the
        // top of the dial add at most +0.40 on the densest session measured. Note what
        // the annulus can and cannot see: it reads the lift at 34-46 px, so a disc
        // confined inside ~30 px registers only its tail here.
        assert!(
            surround <= plain_surround + 0.8,
            "{label}: the sky around a star sits {surround:.2} output levels above the \
             field sky, against {plain_surround:.2} un-denoised — every star is in a lit \
             disc"
        );
        assert!(
            surround <= 1.0,
            "{label}: stars sit in {surround:.2} output levels of brighter sky than the \
             field"
        );

        // A ring: a bump somewhere out in the wings. Real wings only ever decay, so the
        // one thing that cannot happen past the core is the profile turning back up.
        // This is what separates a disc or a halo from the legitimate contrast above.
        for i in 1..PROFILE_RADII.len() {
            let r_out = PROFILE_RADII[i];
            if r_out < 5 {
                continue;
            }
            let (inner, outer) = (measured[i - 1], measured[i]);
            assert!(
                outer <= inner + 0.3,
                "{label}: the profile turns back up at r={r_out} px ({outer:.1} against \
                 {inner:.1} just inside it) — that is a ring, not a star"
            );
        }

        // And the star itself must survive: a filter that flattens the core would pass
        // every bound above by removing the thing they are measured around.
        assert!(
            measured[0] > reference[0] * 0.9,
            "{label}: the star core fell from {:.1} to {:.1} output levels",
            reference[0],
            measured[0]
        );

        // Nor may its *wings* be eaten, which is the failure none of the bounds above
        // can see: every one of them is about light appearing where it should not, and
        // a coarse level removing a star's broad skirt adds nothing anywhere. It shows
        // up on a dense field first — at the top of the dial on the 106-sub M27 set the
        // r=7-17 excess falls by up to 45 %, because between close stars the "wings" are
        // partly unresolved field the coarse levels are entitled to flatten. On this
        // sparse fixture the same positions hold within a quarter, which is the bound.
        for (i, &r) in PROFILE_RADII.iter().enumerate() {
            if !(7..=17).contains(&r) || reference[i] < 1.0 {
                continue;
            }
            assert!(
                measured[i] > reference[i] * 0.6,
                "{label}: the star's wing at r={r} px fell from {:.1} to {:.1} output \
                 levels — the filter is removing the star, not the sky around it",
                reference[i],
                measured[i]
            );
        }
    }
}

/// The instruments themselves, against defects injected on purpose.
///
/// [`a_bright_star_keeps_no_ring`] passes on every setting reachable today — as it should,
/// but that alone does not show it *can* fail. The code that produced the 2026-09-18
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
pub(crate) fn cached_stack(name: &str, dir: &str) -> Option<(night_amplifier::Frame, u32)> {
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
pub(crate) fn requested_sessions() -> Vec<(&'static str, &'static str)> {
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
pub(crate) struct Anchors {
    pub object: (usize, usize),
    pub sky_box: (usize, usize, usize, usize),
    pub stars: Vec<Star>,
}

/// Anchors for a session, from the cache when it is there.
///
/// The reference render they are found on is the product default with denoising off:
/// off, because the star positions and their point-likeness must be judged on a render
/// no spatial filter has touched.
pub(crate) fn anchors(name: &str, stack: &night_amplifier::Frame, depth: u32) -> Anchors {
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
pub(crate) fn report(label: &str, rgb8: &[u8], w: usize, h: usize, a: &Anchors) {
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
