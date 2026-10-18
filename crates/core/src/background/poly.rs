//! The smooth surface the node classifier measures against: least-squares polynomials in
//! frame fractions, and a ladder that adds degrees only while they earn their terms.

use nalgebra::{DMatrix, DVector};

use super::grid::median;
use crate::statistics::MAD_TO_SIGMA;

/// Highest polynomial degree tried: 4 follows a corner glow, 15 terms on >= 144 nodes.
pub(super) const MAX_DEGREE: usize = 4;

/// A higher degree is kept only if it cuts the survivors' residual RMS below this share
/// of the previous one; otherwise the extra terms are bending into a target.
pub(super) const DEGREE_GAIN: f64 = 0.8;

/// Sky nodes per term a degree needs before it is tried: a frame-filling target (M27 on
/// a 1024 px crop) leaves 24 sky nodes in the corners, which a quartic fits and then
/// swings through the middle.
const EVIDENCE: usize = 3;

pub(super) fn terms(degree: usize) -> usize {
    (degree + 1) * (degree + 2) / 2
}

/// Monomials up to [`MAX_DEGREE`].
const TERMS: usize = (MAX_DEGREE + 1) * (MAX_DEGREE + 2) / 2;

/// A node in frame fractions, with its monomials up to [`MAX_DEGREE`] in coordinates
/// scaled to [-1, 1] (for conditioning). Lower degrees come first, so a degree-`d` fit
/// reads the first [`terms`]`(d)`. Computed once per node, not per fit: rebuilt for
/// every matrix cell and every evaluation, they doubled the bilinear estimate's cost.
#[derive(Debug, Clone, Copy)]
pub(super) struct Point {
    pub(super) x: f64,
    pub(super) y: f64,
    monomials: [f64; TERMS],
}

impl Point {
    pub(super) fn new((x, y): (f64, f64)) -> Self {
        let (u, w) = (2.0 * x - 1.0, 2.0 * y - 1.0);
        let mut monomials = [0.0; TERMS];
        let all = (0..=MAX_DEGREE).flat_map(|d| (0..=d).map(move |i| u.powi((d - i) as i32) * w.powi(i as i32)));
        for (m, value) in monomials.iter_mut().zip(all) {
            *m = value;
        }
        Self { x, y, monomials }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Poly {
    coefficients: Vec<f64>,
    pub(super) degree: usize,
}

impl Poly {
    pub(super) fn fit(pos: &[Point], v: &[Option<f64>], on: &[usize], degree: usize) -> Option<Self> {
        if on.len() < terms(degree) + 2 {
            return None;
        }
        let a = DMatrix::from_fn(on.len(), terms(degree), |r, c| pos[on[r]].monomials[c]);
        let b = DVector::from_iterator(on.len(), on.iter().map(|&i| v[i].expect("fitted nodes have values")));
        let solution = a.svd(true, true).solve(&b, 1e-12).ok()?;
        Some(Self {
            coefficients: solution.as_slice().to_vec(),
            degree,
        })
    }

    pub(super) fn at(&self, p: &Point) -> f64 {
        p.monomials.iter().zip(&self.coefficients).map(|(a, b)| a * b).sum()
    }

    pub(super) fn rms(&self, pos: &[Point], v: &[Option<f64>], on: &[usize]) -> f64 {
        let sum: f64 = on.iter().map(|&i| (v[i].unwrap_or(0.0) - self.at(&pos[i])).powi(2)).sum();
        (sum / on.len().max(1) as f64).sqrt()
    }

    /// The highest degree up to [`MAX_DEGREE`] that earns its terms on `on`.
    pub(super) fn best(pos: &[Point], v: &[Option<f64>], on: &[usize]) -> Option<Self> {
        let mut best = Self::fit(pos, v, on, 1)?;
        let mut best_rms = best.rms(pos, v, on);
        for degree in (2..=MAX_DEGREE).take_while(|&d| on.len() >= EVIDENCE * terms(d)) {
            let Some(next) = Self::fit(pos, v, on, degree) else { break };
            let rms = next.rms(pos, v, on);
            if rms >= DEGREE_GAIN * best_rms {
                break;
            }
            (best, best_rms) = (next, rms);
        }
        Some(best)
    }
}

pub(super) fn robust_sigma(values: &[f64]) -> f64 {
    let mut v: Vec<f32> = values.iter().map(|&x| x as f32).collect();
    let centre = median(&mut v) as f64;
    let mut dev: Vec<f32> = values.iter().map(|&x| (x - centre).abs() as f32).collect();
    median(&mut dev) as f64 * MAD_TO_SIGMA as f64
}
