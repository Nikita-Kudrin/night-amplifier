//! Fast guided filtering of the two chroma planes, with luminance as the guide.
//!
//! Colour mottle is chroma noise that survived the debayer's interpolation. It
//! is the one defect that can be smoothed hard without visible loss: the eye
//! resolves far less chroma detail than luminance, so a filter that keeps
//! colour *edges* where the luminance has them can flatten everything in
//! between.
//!
//! # Why guided and not a plain blur
//!
//! A blur wide enough to remove the mottle bleeds a star's colour across the
//! sky beside it. The guided filter solves a local linear model
//! `q = a * I + b` against the luminance guide, so the smoothing collapses
//! exactly where the guide has structure and runs at full width where it does
//! not.
//!
//! # Why the fast variant
//!
//! Both the coefficient solve and the box means run on an `s`-times
//! subsampled copy, and only the final `a`/`b` maps are upsampled back — the
//! standard fast guided filter. It costs `1/s²` of the full solve and is
//! visually indistinguishable here, because `a` and `b` are smooth by
//! construction. Separable sliding-window box filters rather than a summed-area
//! table: two running-sum passes stay in cache on ARM, where a full-resolution
//! f64 integral image does not.

use rayon::prelude::*;

/// Guided-filter smoothing of the chroma planes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromaDenoiseConfig {
    pub enabled: bool,
    /// Filter radius in **display** pixels.
    ///
    /// Derived from the angular size of the mottle, not from a sensor pixel
    /// count: this stage runs after the encoder's downsample, so a radius
    /// carried over from full resolution would cover a quarter of the intended
    /// area.
    pub radius: usize,
    /// Regularization. Larger values smooth across weaker luminance edges.
    pub epsilon: f32,
    /// Resolution divisor for the coefficient solve.
    pub subsample: usize,
    /// Blend between the original and filtered chroma, `0..=1`.
    pub strength: f32,
}

impl Default for ChromaDenoiseConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: DEFAULT_RADIUS,
            epsilon: DEFAULT_EPSILON,
            subsample: DEFAULT_SUBSAMPLE,
            strength: 1.0,
        }
    }
}

/// ~8 display pixels: at 1.7 arcmin per pixel that is a quarter-degree window,
/// which covers the mottle blobs without reaching across a star.
pub const DEFAULT_RADIUS: usize = 8;
/// Luminance is linear and sky-dominated here, so edges worth preserving are
/// far above this.
pub const DEFAULT_EPSILON: f32 = 1e-4;
pub const DEFAULT_SUBSAMPLE: usize = 4;

impl ChromaDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        radius: DEFAULT_RADIUS,
        epsilon: DEFAULT_EPSILON,
        subsample: DEFAULT_SUBSAMPLE,
        strength: 1.0,
    };

    pub fn is_enabled(&self) -> bool {
        self.enabled && self.strength > 0.0 && self.radius > 0
    }
}

/// Smooth `cb` and `cr` in place against the `luma` guide.
///
/// Both planes are filtered in one call rather than two: the guide's window mean
/// and variance do not depend on which plane is being filtered, and neither does
/// the bilinear upsample geometry. Sharing them halves the full-resolution work,
/// which is where this filter spends most of its time — the coefficient solve
/// itself runs on a `subsample`-times smaller grid.
pub fn denoise_chroma(
    luma: &[f32],
    cb: &mut [f32],
    cr: &mut [f32],
    width: usize,
    height: usize,
    config: &ChromaDenoiseConfig,
) {
    let Some(guide) = Guide::new(luma, width, height, config) else {
        return;
    };
    guide.apply(luma, cb, cr, config.strength);
}

/// The guide's per-window statistics at the subsampled resolution, computed
/// once and reused for both chroma planes.
struct Guide {
    /// Subsampled guide, and the window mean and variance over it.
    small: Vec<f32>,
    mean_i: Vec<f32>,
    var_i: Vec<f32>,
    sw: usize,
    sh: usize,
    width: usize,
    height: usize,
    subsample: usize,
    radius: usize,
    epsilon: f32,
}

impl Guide {
    fn new(luma: &[f32], width: usize, height: usize, config: &ChromaDenoiseConfig) -> Option<Self> {
        let n = width * height;
        if n == 0 || luma.len() < n {
            return None;
        }

        let subsample = config.subsample.clamp(1, 16);
        let sw = width.div_ceil(subsample);
        let sh = height.div_ceil(subsample);
        if sw == 0 || sh == 0 {
            return None;
        }

        let small = box_subsample(luma, width, height, subsample);
        // The radius shrinks with the subsample so the window covers the same
        // area of the image. A window narrower than one sample would make the
        // filter an identity and defeat the stage.
        let radius = (config.radius / subsample).max(1);

        let mut scratch = vec![0.0f32; sw * sh];
        let mut mean_i = vec![0.0f32; sw * sh];
        box_mean(&small, &mut mean_i, &mut scratch, sw, sh, radius);

        let sq: Vec<f32> = small.iter().map(|v| v * v).collect();
        let mut var_i = vec![0.0f32; sw * sh];
        box_mean(&sq, &mut var_i, &mut scratch, sw, sh, radius);
        for (v, &m) in var_i.iter_mut().zip(mean_i.iter()) {
            *v = (*v - m * m).max(0.0);
        }

        Some(Self {
            small,
            mean_i,
            var_i,
            sw,
            sh,
            width,
            height,
            subsample,
            radius,
            epsilon: config.epsilon.max(0.0),
        })
    }

    /// Solve the local linear model for one chroma plane, returning the
    /// smoothed `a` and `b` coefficient maps at the subsampled resolution.
    fn solve(&self, plane: &[f32], scratch: &mut [f32]) -> (Vec<f32>, Vec<f32>) {
        let ns = self.sw * self.sh;
        let small_p = box_subsample(plane, self.width, self.height, self.subsample);

        let mut mean_p = vec![0.0f32; ns];
        box_mean(&small_p, &mut mean_p, scratch, self.sw, self.sh, self.radius);

        let mut a: Vec<f32> = self
            .small
            .iter()
            .zip(small_p.iter())
            .map(|(&i, &p)| i * p)
            .collect();
        let mut b = vec![0.0f32; ns];
        box_mean(&a, &mut b, scratch, self.sw, self.sh, self.radius);

        // `a` holds `corr_Ip` on the way in and the coefficient on the way out;
        // `b` holds the window mean of `I * p` and then the intercept.
        for k in 0..ns {
            let coeff = (b[k] - self.mean_i[k] * mean_p[k]) / (self.var_i[k] + self.epsilon);
            a[k] = coeff;
            b[k] = mean_p[k] - coeff * self.mean_i[k];
        }

        let mut mean_a = vec![0.0f32; ns];
        let mut mean_b = vec![0.0f32; ns];
        box_mean(&a, &mut mean_a, scratch, self.sw, self.sh, self.radius);
        box_mean(&b, &mut mean_b, scratch, self.sw, self.sh, self.radius);
        (mean_a, mean_b)
    }

    fn apply(&self, luma: &[f32], cb: &mut [f32], cr: &mut [f32], strength: f32) {
        let ns = self.sw * self.sh;
        let mut scratch = vec![0.0f32; ns];
        let (a_cb, b_cb) = self.solve(cb, &mut scratch);
        let (a_cr, b_cr) = self.solve(cr, &mut scratch);

        // Tap tables rather than a float divide per sample per axis: the
        // upsample geometry is separable and identical for every coefficient
        // map, and this loop is the filter's only full-resolution pass.
        let taps_x: Vec<Taps> = (0..self.width)
            .map(|x| upsample_taps(x, self.subsample, self.sw))
            .collect();
        let taps_y: Vec<Taps> = (0..self.height)
            .map(|y| upsample_taps(y, self.subsample, self.sh))
            .collect();

        let amount = strength.clamp(0.0, 1.0);
        let n = self.width * self.height;
        let width = self.width;
        let sw = self.sw;

        cb[..n]
            .par_chunks_mut(width)
            .zip(cr[..n].par_chunks_mut(width))
            .with_min_len(8)
            .enumerate()
            .for_each(|(y, (row_cb, row_cr))| {
                let ty = taps_y[y];
                let guide_row = &luma[y * width..][..width];
                for x in 0..width {
                    let tx = taps_x[x];
                    let i = guide_row[x];

                    let q_cb = bilinear(&a_cb, sw, tx, ty) * i + bilinear(&b_cb, sw, tx, ty);
                    let q_cr = bilinear(&a_cr, sw, tx, ty) * i + bilinear(&b_cr, sw, tx, ty);

                    row_cb[x] += amount * (q_cb - row_cb[x]);
                    row_cr[x] += amount * (q_cr - row_cr[x]);
                }
            });
    }
}

/// Box-average `src` down by `factor` in both axes, edge blocks included at
/// whatever size they end up.
fn box_subsample(src: &[f32], width: usize, height: usize, factor: usize) -> Vec<f32> {
    if factor == 1 {
        return src[..width * height].to_vec();
    }

    let sw = width.div_ceil(factor);
    let sh = height.div_ceil(factor);
    let mut out = vec![0.0f32; sw * sh];

    out.par_chunks_mut(sw)
        .with_min_len(8)
        .enumerate()
        .for_each(|(sy, row)| {
            let y0 = sy * factor;
            let y1 = (y0 + factor).min(height);
            for (sx, o) in row.iter_mut().enumerate() {
                let x0 = sx * factor;
                let x1 = (x0 + factor).min(width);
                let mut acc = 0.0;
                for y in y0..y1 {
                    acc += src[y * width + x0..y * width + x1].iter().sum::<f32>();
                }
                *o = acc / (((y1 - y0) * (x1 - x0)).max(1) as f32);
            }
        });
    out
}

/// Separable sliding-window box mean of radius `r`, normalized by the window
/// size that actually fits — so an edge sample is the mean of its real
/// neighbours rather than a darkened average against implicit zeros.
fn box_mean(
    src: &[f32],
    dst: &mut [f32],
    scratch: &mut [f32],
    width: usize,
    height: usize,
    r: usize,
) {
    box_mean_rows(src, scratch, width, height, r);
    box_mean_cols(scratch, dst, width, height, r);
}

fn box_mean_rows(src: &[f32], dst: &mut [f32], width: usize, height: usize, r: usize) {
    dst[..width * height]
        .par_chunks_mut(width)
        .with_min_len(8)
        .enumerate()
        .for_each(|(y, out_row)| {
            let row = &src[y * width..][..width];
            let mut sum: f32 = row[..(r + 1).min(width)].iter().sum();
            for x in 0..width {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(width - 1);
                out_row[x] = sum / (hi - lo + 1) as f32;

                // Slide to x + 1: drop the sample leaving the window, add the
                // one entering it.
                if x + 1 < width {
                    if x + r + 1 < width {
                        sum += row[x + r + 1];
                    }
                    if x >= r {
                        sum -= row[x - r];
                    }
                }
            }
        });
}

fn box_mean_cols(src: &[f32], dst: &mut [f32], width: usize, height: usize, r: usize) {
    // Column-major running sums would stride through memory; instead each output
    // row is built from the rows in its window. Parallel over output rows keeps
    // the reads sequential per row at the cost of recomputing the sum, which on
    // a subsampled plane is a few hundred rows.
    dst[..width * height]
        .par_chunks_mut(width)
        .with_min_len(8)
        .enumerate()
        .for_each(|(y, out_row)| {
            let lo = y.saturating_sub(r);
            let hi = (y + r).min(height - 1);
            let inv = 1.0 / (hi - lo + 1) as f32;
            out_row.copy_from_slice(&src[lo * width..][..width]);
            for sy in (lo + 1)..=hi {
                let row = &src[sy * width..][..width];
                for (o, &v) in out_row.iter_mut().zip(row.iter()) {
                    *o += v;
                }
            }
            for o in out_row.iter_mut() {
                *o *= inv;
            }
        });
}

/// The two subsampled indices a full-resolution coordinate falls between, and
/// how far it sits toward the second.
type Taps = (usize, usize, f32);

/// Bilinear tap indices and weight for mapping full-resolution coordinate `x`
/// back onto the subsampled grid, using pixel-centre alignment.
#[inline]
fn upsample_taps(x: usize, factor: usize, len: usize) -> Taps {
    let pos = (x as f32 + 0.5) / factor as f32 - 0.5;
    let pos = pos.clamp(0.0, (len - 1) as f32);
    let i0 = pos.floor() as usize;
    let i1 = (i0 + 1).min(len - 1);
    (i0, i1, pos - i0 as f32)
}

/// Sample a subsampled coefficient map at one full-resolution pixel, given the
/// tap pairs [`upsample_taps`] produced for its two axes.
#[inline]
fn bilinear(map: &[f32], sw: usize, tx: Taps, ty: Taps) -> f32 {
    let (x0, x1, fx) = tx;
    let (y0, y1, fy) = ty;
    let top = map[y0 * sw + x0] * (1.0 - fx) + map[y0 * sw + x1] * fx;
    let bottom = map[y1 * sw + x0] * (1.0 - fx) + map[y1 * sw + x1] * fx;
    top * (1.0 - fy) + bottom * fy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The guided filter's defining property: with a constant guide it reduces
    /// to a plain mean filter, so chroma noise on a featureless sky must fall
    /// hard.
    #[test]
    fn chroma_noise_falls_on_a_featureless_sky() {
        let (w, h) = (96, 96);
        let luma = vec![0.2f32; w * h];
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 40) as f32 / 16777216.0) - 0.5
        };
        let noisy: Vec<f32> = (0..w * h).map(|_| rng() * 0.02).collect();
        let mut cb = noisy.clone();
        let mut cr = noisy.clone();

        denoise_chroma(&luma, &mut cb, &mut cr, w, h, &ChromaDenoiseConfig::default());

        let sigma = |v: &[f32]| {
            let mean: f32 = v.iter().sum::<f32>() / v.len() as f32;
            (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
        };
        assert!(
            sigma(&cb) < sigma(&noisy) * 0.3,
            "chroma sigma only fell from {} to {}",
            sigma(&noisy),
            sigma(&cb)
        );
        assert_eq!(cb, cr, "both planes take the same filter for the same input");
    }

    /// A colour edge that coincides with a luminance edge must survive: that is
    /// the whole reason the filter is guided rather than a blur.
    #[test]
    fn a_guided_colour_edge_is_preserved() {
        let (w, h) = (64, 64);
        let mut luma = vec![0.05f32; w * h];
        let mut cb = vec![0.0f32; w * h];
        for y in 0..h {
            for x in (w / 2)..w {
                luma[y * w + x] = 0.8;
                cb[y * w + x] = 0.3;
            }
        }
        let mut cr = vec![0.0f32; w * h];
        let original = cb.clone();

        denoise_chroma(&luma, &mut cb, &mut cr, w, h, &ChromaDenoiseConfig::default());

        // Sample well clear of the transition; the model is local-linear, so a
        // couple of samples either side of the edge do blend.
        for y in 0..h {
            let left = cb[y * w + w / 4];
            let right = cb[y * w + 3 * w / 4];
            assert!(left.abs() < 0.03, "left of the edge drifted to {left}");
            assert!(
                (right - 0.3).abs() < 0.03,
                "right of the edge drifted to {right} from {}",
                original[y * w + 3 * w / 4]
            );
        }
    }

    /// A constant chroma plane has nothing to remove, and the box means must not
    /// darken it at the borders.
    #[test]
    fn constant_chroma_is_unchanged_including_borders() {
        let (w, h) = (48, 40);
        let luma: Vec<f32> = (0..w * h).map(|i| ((i % 31) as f32) * 0.01).collect();
        let mut cb = vec![0.15f32; w * h];
        let mut cr = vec![-0.07f32; w * h];

        denoise_chroma(&luma, &mut cb, &mut cr, w, h, &ChromaDenoiseConfig::default());

        for (i, &v) in cb.iter().enumerate() {
            assert!((v - 0.15).abs() < 1e-3, "cb sample {i} drifted to {v}");
        }
        for (i, &v) in cr.iter().enumerate() {
            assert!((v + 0.07).abs() < 1e-3, "cr sample {i} drifted to {v}");
        }
    }

    /// Zero strength must be an exact identity, so the settings' off switch is
    /// genuinely off rather than nearly so.
    #[test]
    fn zero_strength_is_an_identity() {
        let (w, h) = (32, 24);
        let luma: Vec<f32> = (0..w * h).map(|i| (i % 17) as f32 * 0.01).collect();
        let original: Vec<f32> = (0..w * h).map(|i| (i % 23) as f32 * 0.001).collect();
        let mut cb = original.clone();
        let mut cr = original.clone();

        let config = ChromaDenoiseConfig {
            strength: 0.0,
            ..Default::default()
        };
        denoise_chroma(&luma, &mut cb, &mut cr, w, h, &config);
        assert_eq!(cb, original);
    }

    #[test]
    fn box_mean_of_a_constant_is_the_constant() {
        let (w, h) = (13, 9);
        let src = vec![0.37f32; w * h];
        let mut dst = vec![0.0f32; w * h];
        let mut scratch = vec![0.0f32; w * h];
        box_mean(&src, &mut dst, &mut scratch, w, h, 3);
        for (i, &v) in dst.iter().enumerate() {
            assert!((v - 0.37).abs() < 1e-5, "sample {i} is {v}");
        }
    }

    #[test]
    fn box_mean_rows_matches_a_direct_window_sum() {
        let (w, h) = (11, 3);
        let src: Vec<f32> = (0..w * h).map(|i| i as f32).collect();
        let mut dst = vec![0.0f32; w * h];
        let r = 2;
        box_mean_rows(&src, &mut dst, w, h, r);
        for y in 0..h {
            for x in 0..w {
                let lo = x.saturating_sub(r);
                let hi = (x + r).min(w - 1);
                let want: f32 = (lo..=hi).map(|i| src[y * w + i]).sum::<f32>() / (hi - lo + 1) as f32;
                assert!(
                    (dst[y * w + x] - want).abs() < 1e-4,
                    "({x},{y}): {} != {want}",
                    dst[y * w + x]
                );
            }
        }
    }
}
