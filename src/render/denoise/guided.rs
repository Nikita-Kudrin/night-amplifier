//! Fast guided filtering of the two chroma planes, with luminance as the guide.
//! Colour mottle is chroma noise surviving debayer interpolation — the one defect
//! smoothable hard without visible loss, since the eye resolves far less chroma
//! detail than luminance.
//!
//! Guided, not a plain blur: a blur wide enough to remove mottle bleeds a star's
//! colour into the sky beside it, while the guided filter's local linear model
//! (`q = a*I + b` against the luminance guide) collapses smoothing exactly where the
//! guide has structure and runs full-width elsewhere.
//!
//! What counts as "structure" is relative to the guide's own noise, measured per frame.
//! The regularisation `epsilon` is the variance below which a window reads as flat; it
//! used to be a constant `1e-4` in linear light, against a sky whose luma variance is
//! ~1e-9 and a faint star's ~1e-8. Every window read as flat, the filter degenerated
//! into a ~40 px box blur of chroma, and each star's colour spread into a halo that
//! size: 32-64 px chroma noise 2-4.6x higher than with the filter off on real IMX533
//! stacks (globular, M27), at exactly the scale a dark-adapted eye is most sensitive to.
//! A fixed value cannot be right at every depth either — `1e-9` fixed the halos on a
//! deep stack and let fine chroma noise back through on a single sub (0.44 -> 0.93
//! output levels), because a single sub's guide noise is itself above it.
//!
//! Fast variant: coefficient solve and box means run on an `s`-times subsampled
//! copy, only `a`/`b` upsampled back — costs `1/s²` of the full solve, visually
//! indistinguishable since `a`/`b` are smooth by construction. Separable
//! sliding-window box filters, not a summed-area table: two running-sum passes stay
//! in cache on ARM, where a full-res f64 integral image doesn't.

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
    /// Edge threshold, in multiples of the guide's noise sigma on the subsampled grid.
    ///
    /// The filter's regularisation is `(noise_k * sigma)^2`, so a window whose
    /// luminance varies by less than `noise_k` noise sigmas is smoothed across and
    /// one with a star in it is not. Measured per frame, so it tracks stack depth.
    pub noise_k: f32,
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
            noise_k: DEFAULT_NOISE_K,
            subsample: DEFAULT_SUBSAMPLE,
            strength: 1.0,
        }
    }
}

/// ~8 display pixels: at 1.7 arcmin per pixel that is a quarter-degree window,
/// which covers the mottle blobs without reaching across a star.
pub const DEFAULT_RADIUS: usize = 8;
/// Swept 2026-09-17 over four real sessions (globular, both M27 sets, the 50 mm guide
/// set) at 1, 8 and full depth, scoring fine chroma noise (2-4 px, what the filter is
/// for) against coarse chroma (32-64 px, the halos) in output levels.
///
/// Fine chroma converges by 3: 0.9-1.5 at `k = 1`, 0.54-0.77 at 2, 0.48-0.67 at 3,
/// and under 0.06 further down to `k = 8`. Coarse chroma is flat across the sweep and
/// at or below the denoise-off figure everywhere, rising slightly past 5 as windows
/// start reading stars as flat again.
pub const DEFAULT_NOISE_K: f32 = 3.0;

/// Regularisation floor, so a noiseless guide (synthetic frames, a flat test
/// pattern) cannot divide by zero. Far below any real sky's guide variance.
const MIN_EPSILON: f32 = 1e-14;

/// Per-window regularisation floor, relative to the window's squared mean.
///
/// `var = E[I^2] - mean^2` in f32 leaves rounding residue of order `mean^2 * 1e-7`
/// even on a perfectly flat window, and `cov` carries the same — which a noise-scaled
/// epsilon on a noiseless guide no longer swamps, so the coefficient became the ratio
/// of two rounding errors (0.34 for 0.30 across a clean colour edge). A real sky
/// (mean 0.002, guide variance ~1e-10 after subsampling) sits well above this.
const ROUNDING_FLOOR: f32 = 1e-6;

/// Guide samples read for the noise estimate. The MAD converges long before this.
///
/// Read by selection (`statistics::select_median`), not `fast_median`, which
/// parallel-sorts from 4096 up — see `wavelet::MAX_SIGMA_SAMPLES`. Unlike the wavelet's
/// this runs once a frame on the subsampled guide, so it is not worth trading sample
/// count against stride here; the count stays where it was.
const NOISE_SAMPLES: usize = 16_384;
pub const DEFAULT_SUBSAMPLE: usize = 4;

impl ChromaDenoiseConfig {
    pub const OFF: Self = Self {
        enabled: false,
        radius: DEFAULT_RADIUS,
        noise_k: DEFAULT_NOISE_K,
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
        let epsilon = guide_epsilon(&small, sw, sh, config.noise_k);
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
            epsilon,
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
            let epsilon = self
                .epsilon
                .max(self.mean_i[k] * self.mean_i[k] * ROUNDING_FLOOR);
            let coeff = (b[k] - self.mean_i[k] * mean_p[k]) / (self.var_i[k] + epsilon);
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

/// The regularisation for a guide: `(noise_k * sigma)^2`, with sigma the guide's noise
/// read from horizontally adjacent differences.
///
/// Differences rather than deviations from a mean so gradients, the target and the
/// background model's residuals drop out; MAD so the stars that do land on a pair
/// cannot drag it. Adjacent samples of the subsampled guide are box means of disjoint
/// blocks, so their noise is close to independent and `diff / sqrt(2)` is the sigma.
fn guide_epsilon(small: &[f32], sw: usize, sh: usize, noise_k: f32) -> f32 {
    if sw < 2 || sh == 0 {
        return MIN_EPSILON;
    }
    let pairs = (sw - 1) * sh;
    let stride = (pairs / NOISE_SAMPLES).max(1);
    let mut diffs: Vec<f32> = (0..pairs)
        .step_by(stride)
        .map(|i| {
            let (y, x) = (i / (sw - 1), i % (sw - 1));
            let row = &small[y * sw..][..sw];
            (row[x + 1] - row[x]).abs()
        })
        .filter(|d| d.is_finite())
        .collect();
    if diffs.is_empty() {
        return MIN_EPSILON;
    }
    let sigma =
        crate::statistics::select_median(&mut diffs) * 1.4826 / std::f32::consts::SQRT_2;
    let epsilon = (noise_k.max(0.0) * sigma).powi(2);
    if epsilon.is_finite() {
        epsilon.max(MIN_EPSILON)
    } else {
        MIN_EPSILON
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

    /// A faint coloured star on a deep-stack sky: luma noise 3e-5, star peak 2e-3.
    /// Both are far below the `1e-4` epsilon this filter used to hold constant, which
    /// read every window as flat and spread the star's colour into a ~40 px halo.
    fn faint_star_ring_bleed(noise_k: f32) -> (f32, f32) {
        let (w, h) = (128, 128);
        let (cx, cy) = (64.0f32, 64.0f32);
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            ((state >> 40) as f32 / 16777216.0) - 0.5
        };
        let mut luma = vec![0.0f32; w * h];
        let mut cb = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let r2 = (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2);
                let star = 2e-3 * (-r2 / 4.0).exp();
                // Uniform noise of this width has sigma 3e-5.
                luma[y * w + x] = 0.002 + star + rng() * 1.04e-4;
                cb[y * w + x] = 0.3 * star;
            }
        }
        let mut cr = cb.clone();
        let config = ChromaDenoiseConfig {
            noise_k,
            ..Default::default()
        };
        denoise_chroma(&luma, &mut cb, &mut cr, w, h, &config);

        let core = cb[64 * w + 64];
        let (mut sum, mut n) = (0.0f32, 0);
        for y in 0..h {
            for x in 0..w {
                let r = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
                if (8.0..20.0).contains(&r) {
                    sum += cb[y * w + x].abs();
                    n += 1;
                }
            }
        }
        (core, sum / n as f32)
    }

    #[test]
    fn a_faint_star_keeps_its_colour_to_itself() {
        let (core, ring) = faint_star_ring_bleed(DEFAULT_NOISE_K);
        assert!(core > 3e-4, "the star lost its colour: core cb {core}");
        assert!(
            ring < core * 0.005,
            "star colour bled into the sky around it: ring {ring} against core {core}"
        );
    }

    /// Guards the test above against passing vacuously: a regularisation far above the
    /// star's own variance (the old constant) must show the halo it is looking for.
    #[test]
    fn an_oversized_regularisation_does_bleed_star_colour() {
        let (core, ring) = faint_star_ring_bleed(1e4);
        assert!(
            ring > core * 0.01,
            "expected the box-blur halo: ring {ring} against core {core}"
        );
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
