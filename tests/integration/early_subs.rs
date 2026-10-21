//! Bad first subs: replaying a session through the stack as it streamed before probation
//! (`legacy`), as it ships (`probation`), and with the bad subs never offered (`oracle`,
//! the best any approach that drops subs can do), and scoring each against the oracle.
//!
//! Shared with Pro by `#[path]` beside `instruments`, so nothing here names `crate::`.
//! Real defects come from `early-bad-subs-imx533` (each session's `manifest.txt` names
//! its bad subs); synthetic ones are injected into a clean session's decoded subs.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use night_amplifier::detection::{detect_stars_adaptive, mean_luminance};
use night_amplifier::registration::AdaptiveRegistration;
use night_amplifier::stacking::warp_frame;
use night_amplifier::statistics::{select_median, MAD_TO_SIGMA};
use night_amplifier::Frame;

use super::instruments::{half_flux_radius, Replay};

/// Real sessions whose first subs are bad, cropped to 1536² and reordered.
pub const FIXTURE: &str = "early-bad-subs-imx533";

/// The sessions in [`FIXTURE`].
pub const SESSIONS: [&str; 4] =
    ["m101-trail-first", "fireworks-bump-first", "fireworks-smear-first", "dumbbell-bump-first"];

/// The probation lengths compared: none (the stack as it streamed) and as shipped.
pub const LEGACY: usize = 0;

/// One session of [`FIXTURE`]: where it is and which subs (1-based) are bad.
pub struct Session {
    pub name: String,
    pub dir: PathBuf,
    pub bad: Vec<usize>,
    pub subs: usize,
}

impl Session {
    /// Reads `<fixtures>/early-bad-subs-imx533/<name>/manifest.txt`; `None` if it is not here.
    pub fn open(fixtures: &Path, name: &str) -> Option<Self> {
        let dir = fixtures.join(FIXTURE).join(name);
        let manifest = std::fs::read_to_string(dir.join("manifest.txt")).ok()?;
        let line = |key: &str| {
            manifest.lines().find_map(|l| l.strip_prefix(key)).map(str::trim).unwrap_or_default().to_string()
        };
        let numbers = |text: String| text.split_whitespace().filter_map(|n| n.parse().ok()).collect::<Vec<usize>>();
        let subs = numbers(line("subs:")).len();
        Some(Self { name: name.to_string(), bad: numbers(line("bad:")), subs, dir })
    }
}

// ---------------------------------------------------------------------------
// Synthetic defects
// ---------------------------------------------------------------------------

/// A defect injected into a decoded sub. Amplitudes after the 2026-10-18 study, where each
/// one reproduced a failure seen at the eyepiece.
#[derive(Clone, Copy, Debug)]
pub enum Defect {
    /// A 1.2 px line, `sigmas` of the sub's pixel noise bright.
    Trail { sigmas: f32 },
    /// Transmission `t` of the signal above sky, a haze of `haze` x the sky with a
    /// gradient, and the noise a clear sub has, so the clouded one is not quieter.
    Cloud { t: f32, haze: f32 },
    /// Horizontal motion blur `len` px long (wind, a knocked tripod).
    Smear { len: usize },
    /// Three box passes of `radius` (defocus, dew).
    Defocus { radius: usize },
    /// Half the light moved by `(dx, dy)`: a bump mid-exposure.
    Doubled { dx: isize, dy: isize },
}

/// `frame` with `defect`; `k` varies a trail's line and a cloud's noise between subs.
pub fn inject(frame: &Frame, defect: Defect, k: usize) -> Frame {
    let (w, h) = (frame.width(), frame.height());
    let mut out = frame.clone();
    for c in 0..frame.channels() {
        let src = frame.channel_data(c);
        let plane = out.channel_data_mut(c);
        match defect {
            Defect::Trail { sigmas } => {
                let amp = sigmas * pixel_noise(frame);
                let (x0, y0, angle) = (w as f32 * (0.3 + 0.13 * k as f32), h as f32 * (0.4 + 0.07 * k as f32), 0.35 + 0.9 * k as f32);
                let (s, co) = angle.sin_cos();
                for (i, v) in plane.iter_mut().enumerate() {
                    let d = (((i % w) as f32 - x0) * s - ((i / w) as f32 - y0) * co).abs();
                    if d < 5.0 {
                        *v = (*v + amp * (-d * d / (2.0 * 1.2 * 1.2)).exp()).min(1.0);
                    }
                }
            }
            Defect::Cloud { t, haze } => {
                let sigma = pixel_noise(frame) * (1.0 + haze - t * t).max(0.0).sqrt();
                let mut gauss = Gauss::new(k as u64 * 3 + c as u64 + 1);
                let mut sample: Vec<f32> = src.iter().step_by(17).copied().collect();
                let sky = select_median(&mut sample);
                for (i, v) in plane.iter_mut().enumerate() {
                    let gradient = 1.0 + 0.6 * ((i % w) as f32 / w as f32 - 0.5);
                    *v = (sky + t * (src[i] - sky) + haze * sky * gradient + sigma * gauss.next()).clamp(1e-5, 1.0);
                }
            }
            Defect::Smear { len } => {
                for (i, v) in plane.iter_mut().enumerate() {
                    let (x, row) = (i % w, i - i % w);
                    *v = (0..len).map(|j| src[row + (x + j).saturating_sub(len / 2).min(w - 1)]).sum::<f32>() / len as f32;
                }
            }
            Defect::Defocus { radius } => {
                for _ in 0..3 {
                    box_blur(plane, w, h, radius);
                }
            }
            Defect::Doubled { dx, dy } => {
                for (i, v) in plane.iter_mut().enumerate() {
                    let sx = ((i % w) as isize - dx).clamp(0, w as isize - 1) as usize;
                    let sy = ((i / w) as isize - dy).clamp(0, h as isize - 1) as usize;
                    *v = 0.5 * src[i] + 0.5 * src[sy * w + sx];
                }
            }
        }
    }
    out
}

/// Per-pixel noise from neighbour differences of the green (or only) plane: robust to
/// stars and nebulosity, and comparable between a sub and a stack.
pub fn pixel_noise(frame: &Frame) -> f32 {
    noise_of(frame.channel_data(1.min(frame.channels() - 1)), frame.width())
}

fn noise_of(plane: &[f32], w: usize) -> f32 {
    let mut d: Vec<f32> = plane
        .chunks(w)
        .step_by(3)
        .flat_map(|row| row.windows(2).step_by(2).map(|p| (p[1] - p[0]).abs()))
        .filter(|v| v.is_finite() && *v != 0.0)
        .collect();
    if d.is_empty() {
        return f32::NAN;
    }
    select_median(&mut d) * MAD_TO_SIGMA / std::f32::consts::SQRT_2
}

fn box_blur(plane: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = plane.to_vec();
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (x.saturating_sub(r), (x + r).min(w - 1));
            tmp[y * w + x] = plane[y * w + a..=y * w + b].iter().sum::<f32>() / (b - a + 1) as f32;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (y.saturating_sub(r), (y + r).min(h - 1));
            plane[y * w + x] = (a..=b).map(|yy| tmp[yy * w + x]).sum::<f32>() / (b - a + 1) as f32;
        }
    }
}

struct Gauss(u64);

impl Gauss {
    fn new(seed: u64) -> Self {
        Self(0x9E37_79B9_7F4A_7C15 ^ seed)
    }

    fn uniform(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        ((self.0 >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    fn next(&mut self) -> f32 {
        let (u, v) = (self.uniform(), self.uniform());
        ((-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()) as f32
    }
}

// ---------------------------------------------------------------------------
// Replaying
// ---------------------------------------------------------------------------

/// What one replay left behind.
pub struct Run {
    pub stack: Frame,
    pub depth: usize,
}

/// Replays the first `subs` of `dir` with the stack's probation at `probation`, never
/// offering the subs (1-based) in `skip` and corrupting those in `defects`.
pub fn replay(dir: &Path, subs: usize, probation: usize, skip: &[usize], defects: &[(usize, Defect)]) -> Run {
    let mut replay = Replay::open(dir).with_probation(probation);
    for position in 1..=subs {
        let Some(mut frame) = replay.capture() else { break };
        if skip.contains(&position) {
            continue;
        }
        if let Some(&(_, defect)) = defects.iter().find(|(at, _)| *at == position) {
            frame = inject(&frame, defect, position);
        }
        replay.stack(&frame, false);
    }
    Run { depth: replay.depth(), stack: replay.snapshot() }
}

// ---------------------------------------------------------------------------
// Scoring against the oracle
// ---------------------------------------------------------------------------

/// A stack against the oracle's.
#[derive(Debug, Clone, Copy)]
pub struct Score {
    pub depth: usize,
    /// Median half-flux radius of the stack's own measurable stars, px.
    pub hfr: f32,
    /// Longest straight run of > 5 sigma excess over the oracle, px: a trail left in.
    pub streak: f32,
    /// Median offset from the oracle in the oracle's pixel noise: haze left in.
    pub haze: f32,
    /// Pixel noise against the oracle's (0.9: 10 % quieter): the depth a stack paid for.
    pub noise: f32,
    /// Whether the stack registers onto the oracle at all: a stack built on a trail can
    /// be a smear of misplaced subs with no field left to match.
    pub registered: bool,
}

impl Score {
    /// Star size against `oracle`'s, as a fraction (+0.10: stars 10 % larger).
    pub fn hfr_excess(&self, oracle: &Score) -> f32 {
        self.hfr / oracle.hfr - 1.0
    }
}

/// [`Score`] of `run` against `oracle` (whose own score is that of a perfect match).
pub fn score(run: &Run, oracle: &Run) -> Score {
    let hfr = half_flux_radius(&run.stack).all;
    let noise = pixel_noise(&run.stack) / pixel_noise(&oracle.stack);
    let unregistered = Score { depth: run.depth, hfr, streak: f32::INFINITY, haze: f32::NAN, noise, registered: false };
    let (Ok(theirs), Ok(ours)) = (detect_stars_adaptive(&oracle.stack), detect_stars_adaptive(&run.stack)) else {
        return unregistered;
    };
    let Some(fit) = AdaptiveRegistration::new().register(&theirs, &ours).ok().filter(|fit| fit.is_credible()) else {
        return unregistered;
    };
    let placed = warp_frame(&run.stack, &fit.transform, 0.0).expect("the stack warps");

    let (w, h) = (oracle.stack.width(), oracle.stack.height());
    let (ours, theirs) = (mean_luminance(&placed), mean_luminance(&oracle.stack));
    let residual: Vec<f32> = ours
        .iter()
        .zip(&theirs)
        .map(|(&a, &b)| if a > 0.0 && b > 0.0 { a - b } else { f32::NAN })
        .collect();
    let mut finite: Vec<f32> = residual.iter().copied().filter(|v| v.is_finite()).collect();
    let offset = select_median(&mut finite);
    Score {
        depth: run.depth,
        hfr,
        streak: longest_streak(&residual, w, h),
        haze: offset / noise_of(&theirs, w),
        noise,
        registered: true,
    }
}

/// Bin used to find streaks: a 1.2 px trail is still several sigma at 4x4, the noise far less.
pub const STREAK_BIN: usize = 4;

/// Longest straight run of > 5 sigma positive residual, in full-resolution px: for each
/// direction (1° steps), hot cells binned by their offset across it and chained along it
/// with gaps of at most [`STREAK_GAP`] cells. Crossing trails stay separate lines, where
/// connected components merged three of them into one blob that read as no trail at all.
pub fn longest_streak(residual: &[f32], w: usize, h: usize) -> f32 {
    let (bw, bh) = (w / STREAK_BIN, h / STREAK_BIN);
    let mut binned = vec![f32::NAN; bw * bh];
    for (i, cell) in binned.iter_mut().enumerate() {
        let (bx, by) = (i % bw, i / bw);
        let mut sum = 0.0;
        for y in by * STREAK_BIN..(by + 1) * STREAK_BIN {
            let row = y * w + bx * STREAK_BIN;
            sum += residual[row..row + STREAK_BIN].iter().sum::<f32>();
        }
        *cell = sum / (STREAK_BIN * STREAK_BIN) as f32;
    }
    longest_binned_streak(&binned, bw)
}

/// [`longest_streak`] on a residual already binned [`STREAK_BIN`]x, `bw` cells wide.
pub fn longest_binned_streak(binned: &[f32], bw: usize) -> f32 {
    let mut finite: Vec<f32> = binned.iter().copied().filter(|v| v.is_finite()).collect();
    if finite.is_empty() {
        return 0.0;
    }
    let level = select_median(&mut finite);
    let mut spread: Vec<f32> = finite.iter().map(|v| (v - level).abs()).collect();
    let sigma = select_median(&mut spread) * MAD_TO_SIGMA;
    let hot: Vec<(f32, f32)> = binned
        .iter()
        .enumerate()
        .filter(|(_, &v)| v.is_finite() && v - level > 5.0 * sigma)
        .map(|(i, _)| ((i % bw) as f32, (i / bw) as f32))
        .collect();

    let mut longest = 0.0f32;
    for degree in 0..180 {
        let (s, c) = (degree as f32).to_radians().sin_cos();
        let mut lines: std::collections::HashMap<i32, Vec<f32>> = std::collections::HashMap::new();
        for &(x, y) in &hot {
            lines.entry((x * c + y * s).round() as i32).or_default().push(y * c - x * s);
        }
        for along in lines.values_mut() {
            along.sort_by(f32::total_cmp);
            let (mut start, mut count) = (along[0], 1);
            for pair in along.windows(2) {
                if pair[1] - pair[0] > STREAK_GAP {
                    (start, count) = (pair[1], 1);
                    continue;
                }
                count += 1;
                if count >= STREAK_MIN_CELLS {
                    longest = longest.max((pair[1] - start) * STREAK_BIN as f32);
                }
            }
        }
    }
    longest
}

/// Cells a streak may skip (noise on a faint trail), and the fewest it must hold: a star's
/// blob spans 3-4 cells, so 10 is a line no star makes.
const STREAK_GAP: f32 = 3.0;
const STREAK_MIN_CELLS: usize = 10;

/// Oracle, legacy and probation replays of one case, scored, with a printed row.
pub struct Comparison {
    pub oracle: Score,
    pub legacy: Score,
    pub probation: Score,
}

impl Comparison {
    pub fn print(&self, label: &str) {
        let row = |name: &str, s: &Score| {
            println!(
                "  {label:<28} {name:<10} depth {:>2}  hfr {:>5.2} ({:+6.1} %)  noise {:>5.3}  streak {:>6.0} px  haze {:>+6.2} sigma{}",
                s.depth,
                s.hfr,
                100.0 * s.hfr_excess(&self.oracle),
                s.noise,
                s.streak,
                s.haze,
                if s.registered { "" } else { "  (does not register onto the oracle)" }
            )
        };
        row("oracle", &self.oracle);
        row("legacy", &self.legacy);
        row("probation", &self.probation);
    }
}

/// Replays the first `subs` of `dir` three ways — the oracle without the bad subs the
/// probation covers, then with them, streamed and on probation — and scores each
/// against the oracle. The oracle stacks on probation too: streamed, M27's re-base at
/// sub 8 cost it 7 of 11 clean subs, which no oracle should lose. A bad sub after the
/// probation is the mid-session gate's to judge, so the oracle keeps it.
pub fn compare(dir: &Path, subs: usize, bad: &[usize], defects: &[(usize, Defect)]) -> Comparison {
    let shipped = night_amplifier::session::capture::PROBATION_SUBS;
    let early: Vec<usize> = bad.iter().copied().filter(|&k| k <= shipped).collect();
    let oracle = replay(dir, subs, shipped, &early, &[]);
    let legacy = replay(dir, subs, LEGACY, &[], defects);
    let probation = replay(dir, subs, shipped, &[], defects);
    Comparison { oracle: score(&oracle, &oracle), legacy: score(&legacy, &oracle), probation: score(&probation, &oracle) }
}

// ---------------------------------------------------------------------------
// Expectations
// ---------------------------------------------------------------------------

/// A residual line this long is a trail; star and noise residue reads 30-100 px.
pub const TRAIL_PX: f32 = 200.0;

/// How a case must come out. Each also holds the stream to still showing the defect,
/// so a fixture that stops reproducing it fails rather than passing for nothing.
#[derive(Clone, Copy, Debug)]
pub enum Expect {
    /// Stars within `hfr` (a fraction) of the oracle's; the stream's at least `legacy_hfr`
    /// larger, or its noise at least `legacy_noise` the oracle's.
    Sharp { hfr: f32, legacy_hfr: f32, legacy_noise: f32 },
    /// The stream keeps a trail; the probation leaves none.
    TrailGone,
    /// The probation keeps under `kept` of the trail the stream keeps.
    TrailFainter { kept: f32 },
    /// Haze under `haze` sigma, or under `kept` of the stream's, which is at least `legacy_haze`.
    Clear { haze: f32, kept: f32, legacy_haze: f32 },
    /// A defect the probation cannot see (smear, Community's trails): no worse than the
    /// stream by more than `hfr`, and no longer a streak.
    NoWorse { hfr: f32 },
    /// Nothing bad: the probation changes nothing that matters.
    Untouched,
}

/// Why `c` misses `expect`, if it does. Every case also holds the probation registered on
/// the oracle and at most 10 % noisier than it: no dropping of good subs to look sharp.
pub fn misses(label: &str, c: &Comparison, expect: Expect) -> Vec<String> {
    let (p, l, o) = (&c.probation, &c.legacy, &c.oracle);
    let mut failures = Vec::new();
    let mut fail = |what: String| failures.push(format!("{label}: {what}"));
    if !p.registered {
        fail("the probation's stack does not register onto the oracle".into());
    }
    if p.noise > 1.10 {
        fail(format!("the probation's stack is {:.0} % noisier than the oracle", 100.0 * (p.noise - 1.0)));
    }
    match expect {
        Expect::Sharp { hfr, legacy_hfr, legacy_noise } => {
            if p.hfr_excess(o) > hfr {
                fail(format!("stars {:+.1} % against the oracle, allowed {:+.1} %", 100.0 * p.hfr_excess(o), 100.0 * hfr));
            }
            if l.hfr_excess(o) < legacy_hfr && l.noise < legacy_noise {
                fail(format!(
                    "the stream no longer shows the defect ({:+.1} % stars, noise {:.2})",
                    100.0 * l.hfr_excess(o),
                    l.noise
                ));
            }
        }
        Expect::TrailGone => {
            if p.streak >= TRAIL_PX {
                fail(format!("a {:.0} px trail survived the probation", p.streak));
            }
            if l.streak < TRAIL_PX {
                fail(format!("the stream no longer keeps the trail ({:.0} px)", l.streak));
            }
        }
        Expect::TrailFainter { kept } => {
            if l.streak < TRAIL_PX {
                fail(format!("the stream no longer keeps the trail ({:.0} px)", l.streak));
            }
            if p.streak > kept * l.streak {
                fail(format!("the probation kept {:.0} of the stream's {:.0} px of trail", p.streak, l.streak));
            }
        }
        Expect::Clear { haze, kept, legacy_haze } => {
            if p.haze.abs() > haze && p.haze.abs() > kept * l.haze.abs() {
                fail(format!("haze {:+.2} sigma left, the stream's {:+.2}", p.haze, l.haze));
            }
            if l.haze.abs() < legacy_haze {
                fail(format!("the stream no longer shows the haze ({:+.2} sigma)", l.haze));
            }
        }
        Expect::NoWorse { hfr } => {
            if p.hfr_excess(o) > l.hfr_excess(o) + hfr {
                fail(format!(
                    "stars {:+.1} % against the stream's {:+.1} %",
                    100.0 * p.hfr_excess(o),
                    100.0 * l.hfr_excess(o)
                ));
            }
        }
        Expect::Untouched => {
            if p.depth != l.depth || (p.hfr - l.hfr).abs() > 0.01 * l.hfr || p.streak >= TRAIL_PX {
                fail(format!("a clean start changed: depth {} vs {}, hfr {:.2} vs {:.2}", p.depth, l.depth, p.hfr, l.hfr));
            }
        }
    }
    failures
}

/// The real sessions of [`FIXTURE`] and what each must come to, under any rejection.
pub fn real_cases() -> [(&'static str, Expect); 4] {
    [
        ("m101-trail-first", Expect::TrailGone),
        ("fireworks-bump-first", Expect::Sharp { hfr: 0.02, legacy_hfr: 0.015, legacy_noise: f32::INFINITY }),
        ("fireworks-smear-first", Expect::NoWorse { hfr: 0.005 }),
        ("dumbbell-bump-first", Expect::Sharp { hfr: 0.03, legacy_hfr: 0.08, legacy_noise: 1.3 }),
    ]
}

/// The defects injected into a clean session, at which subs, and what each must come to
/// whether or not the stack rejects pixels. Trails depend on the rejection: each repo
/// adds its own.
pub fn injected_cases() -> Vec<(&'static str, Defect, Vec<usize>, Expect)> {
    let sharp = |hfr, legacy_hfr| Expect::Sharp { hfr, legacy_hfr, legacy_noise: f32::INFINITY };
    vec![
        ("clean", Defect::Trail { sigmas: 0.0 }, vec![], Expect::Untouched),
        ("doubled@1", Defect::Doubled { dx: 9, dy: 4 }, vec![1], sharp(0.03, 0.3)),
        ("doubled@1-3", Defect::Doubled { dx: 9, dy: 4 }, vec![1, 2, 3], sharp(0.03, 0.4)),
        // One of the three sits just inside the gate's 1.8x bloat limit and stays, as it
        // would mid-session: +3 % at 1/18.
        ("defocus@1-3", Defect::Defocus { radius: 2 }, vec![1, 2, 3], sharp(0.04, 0.05)),
        ("cloud@1", Defect::Cloud { t: 0.3, haze: 1.0 }, vec![1], Expect::Clear { haze: 1.0, kept: 0.0, legacy_haze: 3.0 }),
        ("cloud@1-3", Defect::Cloud { t: 0.3, haze: 1.0 }, vec![1, 2, 3], Expect::Clear { haze: 1.0, kept: 0.5, legacy_haze: 8.0 }),
        ("smear@1-3", Defect::Smear { len: 12 }, vec![1, 2, 3], Expect::NoWorse { hfr: 0.015 }),
    ]
}
