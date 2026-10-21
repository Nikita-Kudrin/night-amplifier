//! Diagnostic: a table of every sub of every local fixture session, to find real bad subs
//! (trails, cloud, smear, bumps) — `early-bad-subs-imx533` was cut from what it found.
//!
//! Per sub: star count, FWHM, SNR, doubled share, roundness and how much the stars' long
//! axes agree (1: smeared one way), sky and noise, star flux against a clean reference
//! (transparency; a bump splits it too), pairs and motion against it, sky offset, the
//! longest straight residual against the median stack (a trail), and production's verdict
//! (provisional until the probation commits, whose row says what it changed).
//!
//! `cargo test --release --test integration_pipeline bad_subs_scan -- --ignored --nocapture`
//! `SCAN_ONLY=a,b` names sessions, `SCAN_DIRS=<dir>,..` any directories, and
//! `SCAN_DUMP=21,22 SCAN_OUT=<dir>` writes those subs, binned and against the median, as PNG.

use std::path::{Path, PathBuf};

use night_amplifier::detection::{
    compute_median_fwhm, compute_median_snr, detect_stars_adaptive, doubled_star_share, mean_luminance, Star,
};
use night_amplifier::registration::{AdaptiveRegistration, AffineTransform};
use night_amplifier::statistics::{select_median, MAD_TO_SIGMA};

use crate::integration::common::FIXTURES_DIR;
use crate::integration::early_subs::{longest_binned_streak, FIXTURE, STREAK_BIN};
use crate::integration::instruments::Replay;

const BIN: usize = STREAK_BIN;

struct Sub {
    stars: Vec<Star>,
    fwhm: f32,
    snr: f32,
    doubling: f32,
    roundness: f32,
    coherence: f32,
    sky: f32,
    noise: f32,
    binned: Vec<f32>,
    verdict: String,
}

fn median_of(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    select_median(&mut v)
}

/// Median roundness (minor/major axis) of the brightest unsaturated stars, and how much
/// their long axes agree (1: all one way, a smear; ~0: random).
fn shape(lum: &[f32], w: usize, h: usize, stars: &[Star], sky: f32) -> (f32, f32) {
    let mut best: Vec<&Star> = stars.iter().filter(|s| s.peak < 0.9).collect();
    best.sort_by(|a, b| b.snr.total_cmp(&a.snr));
    let (mut rounds, mut c2, mut s2, mut n) = (Vec::new(), 0.0f32, 0.0f32, 0.0f32);
    for s in best.into_iter().take(60) {
        let r = (s.fwhm.unwrap_or(3.0) * 1.5).clamp(3.0, 12.0) as i64;
        let (cx, cy) = (s.x.round() as i64, s.y.round() as i64);
        if cx < r || cy < r || cx + r >= w as i64 || cy + r >= h as i64 {
            continue;
        }
        let px: Vec<(f32, f32, f32)> = (cy - r..=cy + r)
            .flat_map(|y| (cx - r..=cx + r).map(move |x| (x, y)))
            .map(|(x, y)| (x as f32, y as f32, (lum[y as usize * w + x as usize] - sky).max(0.0)))
            .collect();
        let m: f32 = px.iter().map(|p| p.2).sum();
        if m <= 0.0 {
            continue;
        }
        let (mx, my) = px.iter().fold((0.0, 0.0), |a, p| (a.0 + p.2 * p.0 / m, a.1 + p.2 * p.1 / m));
        let (xx, yy, xy) = px.iter().fold((0.0, 0.0, 0.0), |a, p| {
            (a.0 + p.2 * (p.0 - mx).powi(2) / m, a.1 + p.2 * (p.1 - my).powi(2) / m, a.2 + p.2 * (p.0 - mx) * (p.1 - my) / m)
        });
        let half_trace = (xx + yy) / 2.0;
        let disc = (half_trace * half_trace - (xx * yy - xy * xy)).max(0.0).sqrt();
        let (major, minor) = (half_trace + disc, (half_trace - disc).max(1e-6));
        rounds.push((minor / major).sqrt());
        let e = (major - minor) / (major + minor);
        let theta2 = (2.0 * xy).atan2(xx - yy);
        c2 += e * theta2.cos();
        s2 += e * theta2.sin();
        n += e;
    }
    (median_of(rounds), if n > 0.0 { c2.hypot(s2) / n } else { f32::NAN })
}

fn bin(lum: &[f32], w: usize, h: usize) -> (Vec<f32>, usize, usize) {
    let (bw, bh) = (w / BIN, h / BIN);
    let binned = (0..bw * bh)
        .map(|i| {
            let (bx, by) = (i % bw, i / bw);
            let sum: f32 = (0..BIN).flat_map(|y| (0..BIN).map(move |x| (x, y))).map(|(x, y)| lum[(by * BIN + y) * w + bx * BIN + x]).sum();
            sum / (BIN * BIN) as f32
        })
        .collect();
    (binned, bw, bh)
}

/// A sub's binned luminance resampled into the reference's bin grid, NaN outside.
fn warp_bin(src: &[f32], w: usize, h: usize, t: &AffineTransform) -> Vec<f32> {
    let b = BIN as f32;
    (0..w * h)
        .map(|i| {
            let (rx, ry) = (((i % w) as f32 + 0.5) * b - 0.5, ((i / w) as f32 + 0.5) * b - 0.5);
            let (tx, ty) = t.inverse_transform_point(rx, ry);
            let (sx, sy) = ((tx + 0.5) / b - 0.5, (ty + 0.5) / b - 0.5);
            if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
                return f32::NAN;
            }
            let (x0, y0) = (sx as usize, sy as usize);
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let at = |x: usize, y: usize| src[y * w + x];
            at(x0, y0) * (1.0 - fx) * (1.0 - fy) + at(x0 + 1, y0) * fx * (1.0 - fy) + at(x0, y0 + 1) * (1.0 - fx) * fy + at(x0 + 1, y0 + 1) * fx * fy
        })
        .collect()
}

/// Median flux ratio target/reference over unsaturated stars the transform pairs within 2 px.
fn transparency(reference: &[Star], target: &[Star], t: &AffineTransform) -> (f32, usize) {
    let ratios: Vec<f32> = target
        .iter()
        .filter(|s| s.peak < 0.9)
        .filter_map(|s| {
            let (x, y) = t.transform_point(s.x, s.y);
            let nearest = reference.iter().filter(|r| r.peak < 0.9).min_by(|a, b| {
                ((a.x - x).powi(2) + (a.y - y).powi(2)).total_cmp(&((b.x - x).powi(2) + (b.y - y).powi(2)))
            })?;
            ((nearest.x - x).powi(2) + (nearest.y - y).powi(2) < 4.0 && nearest.flux > 0.0).then(|| s.flux / nearest.flux)
        })
        .collect();
    let n = ratios.len();
    (median_of(ratios), n)
}

fn dump(name: &str, sub: &[f32], residual: &[f32], w: usize, h: usize, sky: f32, noise: f32) {
    let out = std::env::var("SCAN_OUT").unwrap_or_else(|_| ".".into());
    let stretch = |v: f32| {
        let x = ((v - sky) / (noise / BIN as f32) / 40.0).clamp(-0.05, 1.0) + 0.05;
        if v.is_finite() { ((x.asinh() / 1.05f32.asinh()).clamp(0.0, 1.0) * 255.0) as u8 } else { 0 }
    };
    let save = |suffix: &str, pixels: Vec<u8>| {
        let path = format!("{out}/{name}-{suffix}.png");
        image::GrayImage::from_raw(w as u32, h as u32, pixels).expect("sized").save(&path).expect("writes");
    };
    save("sub", sub.iter().map(|&v| stretch(v)).collect());
    save("residual", residual.iter().map(|&v| if v.is_finite() { (128.0 + v / noise * 48.0).clamp(0.0, 255.0) as u8 } else { 0 }).collect());
}

fn scan(dir: &Path) {
    let mut replay = Replay::open(dir);
    let mut subs = Vec::new();
    let (mut w, mut h, mut bw, mut bh) = (0, 0, 0, 0);
    while let Some(frame) = replay.capture() {
        // Not a session (`background-gradient-stacks` holds stacks of many sizes).
        if !subs.is_empty() && (frame.width(), frame.height()) != (w, h) {
            println!("\n== {}: subs of more than one size, not a session", dir.display());
            return;
        }
        let lum = mean_luminance(&frame);
        let stars = detect_stars_adaptive(&frame).unwrap_or_default();
        let outcome = replay.stack(&frame, false);
        let mut verdict = match (&outcome.admission, outcome.frame_added) {
            (None, added) => if added { "ref" } else { "fail" }.to_string(),
            (Some(a), true) => format!("ok r{:.2}", a.mean_residual),
            (Some(a), false) => format!("NO:{:?}", a.rejected_because.expect("a rejection says why")),
        };
        // Verdicts before a probation commits are provisional; the commit says what moved.
        if let Some(a) = outcome.admission.and_then(|a| a.restacked) {
            verdict += &format!("  [commit: {} out, {} in, new reference: {}]", a.retracted, a.reinstated, a.rereferenced);
        }
        let mut sample: Vec<f32> = lum.iter().step_by(7).copied().collect();
        let sky = select_median(&mut sample);
        let mut deviations: Vec<f32> = sample.iter().map(|v| (v - sky).abs()).collect();
        let noise = select_median(&mut deviations) * MAD_TO_SIGMA;
        (w, h) = (frame.width(), frame.height());
        let (roundness, coherence) = shape(&lum, w, h, &stars, sky);
        let binned;
        (binned, bw, bh) = bin(&lum, w, h);
        subs.push(Sub {
            fwhm: compute_median_fwhm(&stars).unwrap_or(f32::NAN),
            snr: compute_median_snr(&stars).unwrap_or(f32::NAN),
            doubling: doubled_star_share(&stars),
            stars,
            roundness,
            coherence,
            sky,
            noise,
            binned,
            verdict,
        });
    }

    // A clean reference: the sharpest single-imaged sub among those with a typical count.
    let typical = median_of(subs.iter().map(|s| s.stars.len() as f32).collect());
    let reference = (0..subs.len())
        .filter(|&i| subs[i].stars.len() as f32 >= 0.8 * typical && subs[i].fwhm.is_finite() && subs[i].doubling < 0.3)
        .min_by(|&a, &b| subs[a].fwhm.total_cmp(&subs[b].fwhm))
        .unwrap_or(0);
    let registration = AdaptiveRegistration::new();
    let transforms: Vec<Option<AffineTransform>> = subs
        .iter()
        .enumerate()
        .map(|(i, s)| match i == reference {
            true => Some(AffineTransform::identity()),
            false => registration.register(&subs[reference].stars, &s.stars).ok().filter(|r| r.is_credible()).map(|r| r.transform),
        })
        .collect();
    let warped: Vec<Option<Vec<f32>>> =
        subs.iter().zip(&transforms).map(|(s, t)| t.map(|t| warp_bin(&s.binned, bw, bh, &t))).collect();
    let median_stack: Vec<f32> = (0..bw * bh)
        .map(|p| median_of(warped.iter().flatten().map(|v| v[p]).filter(|v| v.is_finite()).collect()))
        .collect();
    let stack_sky = median_of(median_stack.iter().step_by(3).copied().filter(|v| v.is_finite()).collect());
    let dumped: Vec<usize> = std::env::var("SCAN_DUMP")
        .map(|d| d.split(',').filter_map(|k| k.parse().ok()).collect())
        .unwrap_or_default();
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

    println!("\n== {name} ({} subs, {w}x{h}, reference for the scan: #{}) ==", subs.len(), reference + 1);
    println!(
        "{:>3} {:>5} {:>5} {:>6} {:>5} {:>5} {:>5} {:>8} {:>7} {:>6} {:>5} {:>9} {:>6} {:>6}  verdict",
        "#", "stars", "fwhm", "snr", "dbl", "round", "coh", "sky", "noise", "transp", "pairs", "rot/shift", "dsky", "streak"
    );
    for (i, s) in subs.iter().enumerate() {
        let t = transforms[i];
        let (transp, pairs) = t.map_or((f32::NAN, 0), |t| transparency(&subs[reference].stars, &s.stars, &t));
        let residual: Option<Vec<f32>> = warped[i].as_ref().map(|v| v.iter().zip(&median_stack).map(|(a, b)| a - b).collect());
        let streak = residual.as_ref().map_or(0.0, |r| longest_binned_streak(r, bw));
        if let (Some(r), true) = (&residual, dumped.contains(&(i + 1))) {
            dump(&format!("{name}-{}", i + 1), warped[i].as_ref().expect("residual implies warped"), r, bw, bh, stack_sky, s.noise);
        }
        let dsky = if residual.is_some() { (s.sky - stack_sky) / s.noise.max(1e-9) } else { f32::NAN };
        let motion = t.map_or("-".to_string(), |t| format!("{:.2}/{:.0}", t.rotation.to_degrees(), t.tx.hypot(t.ty)));
        println!(
            "{:>3} {:>5} {:>5.2} {:>6.1} {:>5.2} {:>5.2} {:>5.2} {:>8.5} {:>7.5} {:>6.2} {:>5} {:>9} {:>6.1} {:>6.0}  {}",
            i + 1, s.stars.len(), s.fwhm, s.snr, s.doubling, s.roundness, s.coherence, s.sky, s.noise, transp, pairs, motion, dsky, streak, s.verdict
        );
    }
}

#[test]
#[ignore = "diagnostic: reads every local fixture session"]
fn bad_subs_scan() {
    let only: Option<Vec<String>> = std::env::var("SCAN_ONLY").ok().map(|s| s.split(',').map(str::to_string).collect());
    let dirs: Vec<PathBuf> = match std::env::var("SCAN_DIRS") {
        Ok(explicit) => explicit.split(',').map(PathBuf::from).collect(),
        Err(_) => {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(FIXTURES_DIR)
                .expect("the fixtures directory")
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir() && crate::integration::instruments::session_files(p).is_some())
                .filter(|p| p.file_name().is_some_and(|n| n != FIXTURE && only.as_ref().is_none_or(|o| o.iter().any(|x| n == x.as_str()))))
                .collect();
            dirs.sort();
            dirs
        }
    };
    for dir in dirs {
        scan(&dir);
    }
}
