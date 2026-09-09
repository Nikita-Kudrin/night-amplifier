//! Incremental pixel statistics using Welford's Online Algorithm.

/// How many recent samples the scale estimate effectively remembers.
///
/// The first `SCALE_WINDOW` offered samples are averaged evenly (`alpha = 1/offered`),
/// after which the estimate becomes exponential with that time constant. Warm-up
/// deviations are measured against a mean that is still moving and so read high; letting
/// them keep permanent weight would leave the window permanently wide.
pub const SCALE_WINDOW: f32 = 32.0;

/// Shorter memory applied to a sample the rejector clipped.
///
/// A clip is evidence the window may be wrong, so it should move the estimate faster
/// than an ordinary sample does. It also sets how fast a collapsed scale climbs back:
/// each winsorised sample contributes `k^2` times the current variance, which at the
/// 32-sample window grows it 16 % a frame — 40 frames to reach a realistic spread, most
/// of a session. At 8 it is 12 frames. Clips are ~1 % of samples in normal operation, so
/// the faster rate costs the steady-state estimate nothing measurable.
pub const CLIPPED_SCALE_WINDOW: f32 = 8.0;

/// Smallest scale the rejector will clip against. Only has to be non-zero — see
/// [`IncrementalPixel::scale`].
pub const SCALE_FLOOR: f32 = 1e-6;

/// `1 + 1/count` for the first `MEAN_ERROR_TABLE_LEN` counts, 1.0 beyond.
///
/// The running mean is itself an estimate from `count` samples, so the gap under test has
/// variance `sigma^2 * (1 + 1/count)`. Computing that per pixel costs a float divide in a
/// loop that runs 27 million times a frame; past 64 samples the correction is under 1.6 %
/// and is dropped entirely.
pub const MEAN_ERROR_TABLE_LEN: usize = 64;

/// Builds the table above. Called once per frame, not once per pixel.
pub fn mean_error_table() -> [f32; MEAN_ERROR_TABLE_LEN] {
    let mut t = [1.0f32; MEAN_ERROR_TABLE_LEN];
    for (n, slot) in t.iter_mut().enumerate().skip(1) {
        *slot = 1.0 + 1.0 / n as f32;
    }
    t
}

/// `alpha` for the first `SCALE_WINDOW` scale observations; constant `1/SCALE_WINDOW`
/// beyond. Same reason as [`mean_error_table`]: hoists a divide out of the pixel loop.
pub fn scale_alpha_table() -> [f32; SCALE_WINDOW as usize + 1] {
    let mut t = [1.0 / SCALE_WINDOW; SCALE_WINDOW as usize + 1];
    for (n, slot) in t.iter_mut().enumerate().skip(1) {
        *slot = (1.0 / n as f32).max(1.0 / SCALE_WINDOW);
    }
    t
}

/// Maintains running statistics and the final blended pixel value in O(1) space.
///
/// Two counters, not one: `count` is the samples that made it into `mean`, `offered` is
/// every sample the rejector looked at. They differ exactly by what was rejected, and
/// both are needed — `mean`'s standard error comes from the first, the scale estimate's
/// averaging schedule from the second. `offered` costs nothing: `count` left two bytes of
/// tail padding in a 16-byte struct, and at 3008x3008x3 this accumulator is 434 MB, so
/// growing it was not an option.
#[derive(Clone, Copy)]
pub struct IncrementalPixel {
    /// Total accumulated weight (W_n) of *accepted* samples
    pub weight_sum: f32,
    /// Running weighted mean (M_n) - This IS your final pixel value!
    pub mean: f32,
    /// Running mean of squared deviations: the scale the rejector clips against.
    ///
    /// Not Welford's sum-of-squares any more, and deliberately not maintained by
    /// `blend`. Estimating the scale from accepted samples only made it the variance of
    /// an already-truncated distribution — self-confirming, since an early underestimate
    /// rejects the very samples that would widen it. Measured on a 71-frame stack that
    /// discarded 15 % of all samples where 2.5 sigma predicts 1.2 %, and left the result
    /// 34 % noisier than a plain mean. `observe_scale` now folds in *every* offered
    /// sample, so this is a variance of what arrived rather than of what survived.
    pub m2: f32,
    /// Number of samples blended into `mean`
    pub count: u16,
    /// Number of samples offered to this pixel, accepted or not
    pub offered: u16,
}

impl IncrementalPixel {
    #[inline]
    pub fn new() -> Self {
        Self {
            weight_sum: 0.0,
            mean: 0.0,
            m2: 0.0,
            count: 0,
            offered: 0,
        }
    }

    /// West's algorithm for incrementally updating the weighted mean in O(1).
    #[inline]
    pub fn blend(&mut self, value: f32, weight: f32) {
        self.count += 1;
        let temp_weight_sum = self.weight_sum + weight;

        let diff = value - self.mean;
        let r = diff * weight / temp_weight_sum;

        self.mean += r;
        self.weight_sum = temp_weight_sum;
    }

    /// Folds one offered sample into the scale estimate.
    ///
    /// `deviation` is that sample's distance from the running mean, **winsorised by the
    /// caller** at the rejection threshold. Winsorising rather than skipping is what
    /// keeps a cosmic ray from inflating the window while still letting a pixel escape a
    /// scale that has collapsed: a rejected sample contributes `k * sigma` instead of its
    /// real distance, which is `k^2` times the current variance, so a window that is far
    /// too tight widens geometrically and recovers within a few frames instead of
    /// latching shut for the rest of the session.
    ///
    /// Call once per offered sample, after `offered` has been incremented. `clipped`
    /// says whether the caller winsorised `deviation`, which shortens the memory —
    /// see [`CLIPPED_SCALE_WINDOW`].
    ///
    /// The first offered sample is ignored: `mean` is still its initial zero, so that
    /// sample's "deviation" is the pixel's absolute level, not a deviation at all. On a
    /// sky sitting at 0.0024 with a sigma of 2e-5 that seeds the scale 120x too wide, and
    /// it decays only as `1/offered` — measured across a 71-frame stack the window never
    /// closed and the rejector clipped **nothing**, which is worse than the defect it
    /// replaced. Deviations are only meaningful from the second sample on, so the
    /// averaging counts from there too.
    #[inline]
    pub fn observe_scale(&mut self, deviation: f32, clipped: bool) {
        self.observe_scale_with(deviation, clipped, &scale_alpha_table());
    }

    /// [`Self::observe_scale`] against a caller-hoisted alpha table — the form the
    /// per-pixel loop uses, so the table is built once per frame rather than per pixel.
    #[inline]
    pub fn observe_scale_with(
        &mut self,
        deviation: f32,
        clipped: bool,
        alphas: &[f32; SCALE_WINDOW as usize + 1],
    ) {
        let observations = self.offered.saturating_sub(1) as usize;
        if observations == 0 {
            return;
        }
        let mut alpha = alphas[observations.min(SCALE_WINDOW as usize)];
        if clipped {
            alpha = alpha.max(1.0 / CLIPPED_SCALE_WINDOW);
        }
        self.m2 += alpha * (deviation * deviation - self.m2);
    }

    /// Variance the rejector clips against, floored to stay positive.
    ///
    /// The threshold test is done on squared quantities so the common path needs no
    /// square root — over 27 million pixels a frame, one avoidable `sqrt` and one
    /// avoidable divide per pixel measured 2.9x on the whole kernel. [`Self::scale`] is
    /// the same value rooted, for the rare winsorising branch and for tests.
    #[inline]
    pub fn variance(&self) -> f32 {
        self.m2.max(SCALE_FLOOR * SCALE_FLOOR)
    }

    /// Standard deviation the rejector clips against, floored to stay positive.
    ///
    /// The floor only has to be non-zero. It is not sized to the data because it does not
    /// have to be: `observe_scale` grows the estimate out of the floor within about a
    /// dozen frames, which is what the predecessor — where rejected samples updated
    /// nothing at all — could not do. A pixel whose first samples were identical (0.95 %
    /// of them on 14-bit data) pinned at the floor and rejected every later sample for
    /// the rest of the session: 3 of 71 frames kept, forever.
    #[inline]
    pub fn scale(&self) -> f32 {
        self.variance().sqrt()
    }

    #[inline]
    pub fn reset(&mut self) {
        self.count = 0;
        self.offered = 0;
        self.weight_sum = 0.0;
        self.mean = 0.0;
        self.m2 = 0.0;
    }
}

impl Default for IncrementalPixel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `offered` has to live in `count`'s tail padding. At 3008x3008x3 every extra byte
    /// is 27 MB of resident memory on a machine that may only have 8 GB.
    #[test]
    fn the_accumulator_did_not_grow() {
        assert_eq!(std::mem::size_of::<IncrementalPixel>(), 16);
    }

    #[test]
    fn blend_tracks_a_weighted_mean() {
        let mut p = IncrementalPixel::new();
        for v in [0.2f32, 0.4, 0.6] {
            p.blend(v, 1.0);
        }
        assert!((p.mean - 0.4).abs() < 1e-6, "{}", p.mean);
        assert_eq!(p.count, 3);
    }

    /// The scale is a variance of what arrived, so equal-sized deviations either side of
    /// the mean settle on their own magnitude.
    #[test]
    fn scale_converges_on_the_spread_it_is_shown() {
        let mut p = IncrementalPixel::new();
        for i in 0..200 {
            let deviation = if i % 2 == 0 { 0.01 } else { -0.01 };
            p.offered += 1;
            p.observe_scale(deviation, false);
        }
        assert!(
            (p.scale() - 0.01).abs() < 1e-4,
            "scale {} should approach 0.01",
            p.scale()
        );
    }

    /// A collapsed scale must climb back out rather than latch shut.
    #[test]
    fn a_collapsed_scale_recovers() {
        let mut p = IncrementalPixel::new();
        p.offered = 40; // past warm-up, so alpha is at its floor
        assert_eq!(p.scale(), 1e-6, "test needs a collapsed starting scale");

        // Every sample lands outside the window and is winsorised to it, which is the
        // worst case: the scale only ever sees the threshold itself.
        for _ in 0..15 {
            p.offered += 1;
            let winsorised = 2.5 * p.scale();
            p.observe_scale(winsorised, true);
        }
        assert!(
            p.scale() > 2e-5,
            "scale stayed at {} after 15 winsorised samples — a pixel that starts \
             degenerate has to climb out within a few frames, not a whole session",
            p.scale()
        );
    }
}
