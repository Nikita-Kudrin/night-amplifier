//! Monotone cubic (PCHIP) interpolation through the node grid: Fritsch-Carlson slopes, so
//! the curve never leaves the range of the two nodes either side of it. Bilinear cut every
//! peak between nodes: the 2026-10-24 M57 sky rose to 6.2 sigma at x = 0.89 and fell to 0
//! by 0.98, between lines at 0.82, 0.91 and 0.96, and kept 0.6 sigma of it; a smooth
//! spline keeps less but rings beside a target's hole, which a monotone one cannot.

/// Slope at each node of `values` at positions `xs` (strictly increasing), as SciPy's
/// `PchipInterpolator` sets them: the weighted harmonic mean of the secants either side,
/// zero where they change sign; the ends by a three-point estimate kept monotone.
pub fn slopes(xs: &[f32], values: &[f32], out: &mut [f32]) {
    let n = values.len();
    if n < 2 {
        out.iter_mut().for_each(|d| *d = 0.0);
        return;
    }
    let h = |k: usize| xs[k + 1] - xs[k];
    let secant = |k: usize| (values[k + 1] - values[k]) / h(k);
    if n == 2 {
        out[0] = secant(0);
        out[1] = secant(0);
        return;
    }
    for (k, d) in out.iter_mut().enumerate().take(n - 1).skip(1) {
        let (s0, s1) = (secant(k - 1), secant(k));
        *d = if s0 * s1 <= 0.0 {
            0.0
        } else {
            let (w1, w2) = (2.0 * h(k) + h(k - 1), h(k) + 2.0 * h(k - 1));
            (w1 + w2) / (w1 / s0 + w2 / s1)
        };
    }
    out[0] = end_slope(h(0), h(1), secant(0), secant(1));
    out[n - 1] = end_slope(h(n - 2), h(n - 3), secant(n - 2), secant(n - 3));
}

fn end_slope(h0: f32, h1: f32, s0: f32, s1: f32) -> f32 {
    let d = ((2.0 * h0 + h1) * s0 - h0 * s1) / (h0 + h1);
    if d * s0 <= 0.0 {
        0.0
    } else if s0 * s1 < 0.0 && d.abs() > 3.0 * s0.abs() {
        3.0 * s0
    } else {
        d
    }
}

/// The cubic on one segment `[x0, x0 + h]` in the offset `s = x - x0`, Horner order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    c0: f32,
    c1: f32,
    c2: f32,
    c3: f32,
}

impl Segment {
    /// Hermite cubic from the two ends' values and slopes.
    pub fn new(h: f32, (v0, d0): (f32, f32), (v1, d1): (f32, f32)) -> Self {
        let secant = (v1 - v0) / h;
        Self {
            c0: v0,
            c1: d0,
            c2: (3.0 * secant - 2.0 * d0 - d1) / h,
            c3: (d0 + d1 - 2.0 * secant) / (h * h),
        }
    }

    /// The same cubic `by` higher.
    pub fn raised(mut self, by: f32) -> Self {
        self.c0 += by;
        self
    }

    pub fn at(&self, s: f32) -> f32 {
        self.c0 + s * (self.c1 + s * (self.c2 + s * self.c3))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(xs: &[f32], values: &[f32]) -> impl Fn(f32) -> f32 {
        let mut d = vec![0.0; values.len()];
        slopes(xs, values, &mut d);
        let (xs, values) = (xs.to_vec(), values.to_vec());
        move |x| {
            let k = xs.windows(2).position(|w| x <= w[1]).unwrap_or(xs.len() - 2);
            Segment::new(xs[k + 1] - xs[k], (values[k], d[k]), (values[k + 1], d[k + 1])).at(x - xs[k])
        }
    }

    #[test]
    fn it_passes_through_every_node() {
        let xs = [0.0, 3.0, 4.0, 9.0, 10.0];
        let values = [1.0, 2.5, 2.0, 7.0, 6.5];
        let f = curve(&xs, &values);
        for (x, v) in xs.iter().zip(values) {
            assert!((f(*x) - v).abs() < 1e-5, "f({x}) = {}, node {v}", f(*x));
        }
    }

    #[test]
    fn a_linear_run_stays_linear() {
        let xs = [0.0, 2.0, 5.0, 6.0];
        let f = curve(&xs, &xs.map(|x| 0.5 + 0.25 * x));
        for i in 0..=60 {
            let x = i as f32 * 0.1;
            assert!((f(x) - (0.5 + 0.25 * x)).abs() < 1e-5, "f({x}) = {}", f(x));
        }
    }

    /// The point of the method: a peak between nodes neither overshoots nor dips, so a
    /// target's refilled hole is never undercut beside it.
    #[test]
    fn it_never_leaves_the_range_of_a_segments_nodes() {
        let xs = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let values = [0.0, 0.0, 1.0, 1.0, 0.2, 0.2];
        let f = curve(&xs, &values);
        for i in 0..=500 {
            let x = i as f32 * 0.01;
            let k = (x.floor() as usize).min(4);
            let (lo, hi) = (values[k].min(values[k + 1]), values[k].max(values[k + 1]));
            assert!(f(x) >= lo - 1e-6 && f(x) <= hi + 1e-6, "f({x}) = {} outside [{lo}, {hi}]", f(x));
        }
    }

    /// Against SciPy 1.15's `PchipInterpolator` on uneven nodes, ends included.
    #[test]
    fn it_matches_scipy() {
        let xs = [0.0, 1.0, 3.0, 4.0, 7.0];
        let values = [0.0, 1.0, 1.5, 3.0, 3.2];
        let mut d = [0.0; 5];
        slopes(&xs, &values, &mut d);
        let want = [1.25, 0.4285714, 0.4655172, 0.1506276, 0.0];
        for (got, want) in d.iter().zip(want) {
            assert!((got - want).abs() < 1e-5, "slopes {d:?}, SciPy {want:?}");
        }
    }

    #[test]
    fn two_nodes_are_a_straight_line() {
        let f = curve(&[0.0, 4.0], &[1.0, 3.0]);
        assert!((f(1.0) - 1.5).abs() < 1e-6);
    }
}
