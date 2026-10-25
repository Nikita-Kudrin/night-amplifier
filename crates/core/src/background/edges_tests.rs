use super::*;

const SKY: f32 = 0.05;
const SIGMA: f32 = 1e-4;

/// Deterministic noise in [-1, 1].
fn jitter(i: usize) -> f32 {
    let h = (i as u32).wrapping_mul(2_654_435_761).rotate_left(13).wrapping_mul(2_246_822_519);
    (h >> 8) as f32 / (1u32 << 23) as f32 - 1.0
}

/// One noisy channel, `sky(x, y)` in frame fractions on a flat level.
fn frame(size: usize, sky: impl Fn(f32, f32) -> f32) -> Frame {
    let mut f = Frame::zeros(size, size, 1).unwrap();
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 / (size - 1) as f32, y as f32 / (size - 1) as f32);
            f.set_pixel(x, y, 0, SKY + sky(fx, fy) + SIGMA * jitter(y * size + x));
        }
    }
    f
}

/// A model that is the flat sky alone, so the profiles see everything else.
fn flat(size: usize) -> BackgroundModel {
    BackgroundModel::new(vec![vec![SKY; 4]], 2, 2, size, size, 1, false, 0.1, 1.0)
}

/// The M57 instrument band: up to 20 sigma at 0.9 of the width, gone by the edge.
fn band(x: f32) -> f32 {
    20.0 * SIGMA * (-((x - 0.9) / 0.04).powi(2)).exp()
}

#[test]
fn a_band_along_an_edge_is_measured() {
    let size = 512;
    let f = frame(size, |x, _| band(x));
    let edges = EdgeProfiles::estimate(&f, &flat(size), &[]);
    let right = &edges.profiles[0][1];
    assert!(!right.is_empty(), "a 512 px frame is large enough to measure");
    for (d, &p) in right.iter().enumerate().take(right.len() * 7 / 10).skip(4) {
        let x = 1.0 - d as f32 / (size - 1) as f32;
        let missed = (p - band(x)).abs() / SIGMA;
        assert!(missed < 3.0, "{d} px in: profile {:.1} sigma, band {:.1}", p / SIGMA, band(x) / SIGMA);
    }
}

/// A nebula touching the edge fills one segment of five: the profile must not take it.
#[test]
fn a_target_at_the_edge_is_left_alone() {
    let size = 512;
    let nebula = |x: f32, y: f32| 20.0 * SIGMA * (-((x - 0.97).powi(2) + (y - 0.5).powi(2)) / (2.0 * 0.06f32.powi(2))).exp();
    let edges = EdgeProfiles::estimate(&frame(size, nebula), &flat(size), &[]);
    assert!(edges.peak() < 2.0 * SIGMA, "took {:.1} sigma of a 20 sigma nebula", edges.peak() / SIGMA);
}

/// A slope across the band is the grid's to follow: its straight line is taken out.
#[test]
fn a_smooth_gradient_leaves_no_profile() {
    let size = 512;
    let edges = EdgeProfiles::estimate(&frame(size, |x, y| 30.0 * SIGMA * (0.7 * x + 0.3 * y)), &flat(size), &[]);
    assert!(edges.peak() < 1.0 * SIGMA, "a ramp left a {:.2} sigma profile", edges.peak() / SIGMA);
}

#[test]
fn a_frame_too_small_to_measure_gets_none() {
    let edges = EdgeProfiles::estimate(&frame(48, |x, _| band(x)), &flat(48), &[]);
    assert_eq!(edges.peak(), 0.0);
}

/// Through the model: subtraction takes the band too, and the edges follow each channel.
#[test]
fn a_model_with_edges_subtracts_the_band() {
    let size = 512;
    let mut f = frame(size, |x, _| band(x));
    let model = flat(size);
    let edges = EdgeProfiles::estimate(&f, &model, &[]);
    model.with_edges(edges).subtract_from(&mut f);
    let column = |x: usize| (0..size).map(|y| f.get_pixel(x, y, 0)).sum::<f32>() / size as f32;
    let peak_x = (0.9 * (size - 1) as f32) as usize;
    let left = (column(peak_x) - column(size / 2)) / SIGMA;
    assert!(left.abs() < 2.0, "the band's peak keeps {left:.1} sigma of 20");
}

/// Noise is not a band: a stack re-measures its profiles every few frames, and a profile
/// made of noise moved the render with each.
#[test]
fn noise_alone_leaves_no_profile() {
    let size = 512;
    let f = frame(size, |x, y| 20.0 * SIGMA * jitter(((x * 7919.0) as usize) ^ ((y * 104_729.0) as usize)));
    let edges = EdgeProfiles::estimate(&f, &flat(size), &[]);
    assert_eq!(edges.peak(), 0.0, "noise left a {:.2} sigma profile", edges.peak() / SIGMA);
}

/// A profile is sampled in steps of 0.5 % of the frame (13 px at 2712 wide), and each
/// pixel must not take its step's value whole: a correction that jumps where the band only
/// slopes prints a line the length of the edge. Here the band itself climbs at most
/// 0.21 sigma a pixel; the correction may climb no more than twice that.
#[test]
fn the_profile_has_no_steps() {
    let (w, h) = (2048usize, 512usize);
    let mut f = Frame::zeros(w, h, 1).unwrap();
    for y in 0..h {
        for x in 0..w {
            f.set_pixel(x, y, 0, SKY + band(x as f32 / (w - 1) as f32) + SIGMA * jitter(y * w + x));
        }
    }
    let model = BackgroundModel::new(vec![vec![SKY; 4]], 2, 2, w, h, 1, false, 0.1, 1.0);
    let edges = EdgeProfiles::estimate(&f, &model, &[]);
    let right = &edges.profiles[0][1];
    let jump = right.windows(2).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max) / SIGMA;
    assert!(jump < 0.45, "the correction jumps {jump:.2} sigma between neighbouring pixels");
}

/// Ring-2's registration border: a strip a few steps deep reached by few subs, 5x as noisy
/// and on its own level. Profiled through, its step to the sky became a bright line; it is
/// left as the grid has it, and a band just inside it is still measured.
#[test]
fn a_thin_coverage_strip_is_left_alone() {
    let size = 1024;
    let band = |y: f32| 10.0 * SIGMA * (-((y - 0.07) / 0.02).powi(2)).exp();
    let mut f = Frame::zeros(size, size, 1).unwrap();
    for y in 0..size {
        let (thin, fy) = (y < 12, y as f32 / (size - 1) as f32);
        for x in 0..size {
            let (offset, noise) = if thin { (3.0 * SIGMA, 5.0) } else { (0.0, 1.0) };
            f.set_pixel(x, y, 0, SKY + offset + band(fy) + noise * SIGMA * jitter(y * size + x));
        }
    }
    let edges = EdgeProfiles::estimate(&f, &flat(size), &[]);
    let top = &edges.profiles[0][2];
    let inside = top[..8].iter().fold(0.0f32, |m, p| m.max(p.abs())) / SIGMA;
    assert!(inside < 0.1, "the strip got a {inside:.2} sigma correction");
    let peak = (0.07 * (size - 1) as f32) as usize;
    let missed = (top[peak] - band(peak as f32 / (size - 1) as f32)).abs() / SIGMA;
    assert!(missed < 2.5, "the band beside the strip was missed by {missed:.1} sigma of 10");
}

/// The model is rendered only on the rows [`Plan::rows`] lists, so it must list every row a
/// cell samples, odd steps (whose stride restarts per step) included.
#[test]
fn the_planned_rows_hold_every_sample() {
    for (w, h) in [(512, 512), (1504, 1504), (2712, 1538), (1433, 977)] {
        for edge in EDGES {
            let Some(plan) = Plan::new(w, h, edge, &[]) else { continue };
            let rows: std::collections::HashSet<usize> = plan.rows().into_iter().collect();
            for segment in 0..SEGMENTS {
                for k in 0..plan.steps {
                    let mut ys = Vec::new();
                    plan.gather(segment, k, &mut ys, |_, y| y as f32);
                    assert!(ys.iter().all(|&y| rows.contains(&(y as usize))), "{edge:?} at {w}x{h}: cell ({segment}, {k})");
                }
            }
        }
    }
}
