//! Whether a fitted transform is told apart from chance by the stars it pairs.
//!
//! A count or share of matched stars cannot say this: a thin sub holds ~15 real stars
//! among 200 detections, so the true transform of the Cat's Eye fixture pairs 10–19 of
//! them (a 25 % share needs 50), while a coincidence that maps a bright nebula's cluster
//! of noise maxima onto itself pairs up to 12. What separates them is how many pairs
//! chance would have produced *where those stars are*: a cluster expects its own
//! coincidences, an empty sky expects none. `AdaptiveRegistration` measures both the
//! full and the clutter-suppressed lists and keeps the stronger (see its `finish`).

use crate::detection::Star;

use super::neighbours::ReferenceByX;
use super::transform::AffineTransform;

/// Distance (px) within which a mapped target star counts as landing on a reference
/// star: about a faint star's centroid noise, and the refit's own final radius.
const PAIR_RADIUS_PX: f32 = 1.5;

/// Radius (px) over which the reference star density around a mapped target star is
/// measured, to estimate its chance of landing on one. Wide enough to hold several
/// stars on a normal field, narrow enough that a nebula's cluster is its own region.
const DENSITY_RADIUS_PX: f32 = 25.0;

/// The largest log10 chance probability a credible fit may have.
///
/// Measured over eleven fixture sets as `AdaptiveRegistration` scores them (the
/// stronger of the full and clutter-suppressed lists): every coincidental fit scores 0
/// to −5 on either list, while every production fit scores −12 or below except the
/// Cat's Eye's three weakest, at −7.7, −9.3 and −11.3. A voting fit also pays for its
/// trials ([`Support::searched`]): the weakest, a turned Cat's Eye sub, scores −7.0.
const MAX_LOG10_CHANCE: f64 = -6.0;

/// The largest scale change a credible fit may claim. Subs of one session share their
/// optics, so a real scale change is far below this (refraction is ~1e-4); a
/// coincidence picks its scale freely and was off by 1.6–10 % on the Cat's Eye.
const MAX_SCALE_DEVIATION: f32 = 0.01;

/// How well a transform's pairs stand out from chance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Support {
    /// Target stars the transform lands within [`PAIR_RADIUS_PX`] of a reference star,
    /// one-to-one.
    pub pairs: usize,
    /// Pairs chance alone would produce at the local star density.
    pub expected: f32,
    /// log10 of the Poisson probability of at least `pairs` given `expected`.
    pub log10_chance: f64,
    /// log10 of the hypotheses the transform was picked from, when it was searched for
    /// rather than fitted (see [`Support::searched`]); 0 otherwise.
    pub log10_trials: f64,
}

impl Support {
    /// Measures `transform` (target -> reference) against both star lists.
    pub fn measure(reference: &[Star], target: &[Star], transform: &AffineTransform) -> Self {
        let by_x = ReferenceByX::new(reference);
        let pairs = by_x.nearest_pairs(target, transform, PAIR_RADIUS_PX).len();

        // A mapped star with n reference stars around it lands within the pair radius
        // of one with probability n * (r / R)^2, as long as that is small.
        let share = (PAIR_RADIUS_PX / DENSITY_RADIUS_PX).powi(2);
        let expected: f32 = target
            .iter()
            .map(|star| {
                let (x, y) = transform.transform_point(star.x, star.y);
                (by_x.count_within(x, y, DENSITY_RADIUS_PX) as f32 * share).min(1.0)
            })
            .sum();

        Self {
            pairs,
            expected,
            log10_chance: log10_poisson_tail(pairs, f64::from(expected)),
            log10_trials: 0.0,
        }
    }

    /// This support for a transform that was the best of `trials` candidates.
    ///
    /// The chance of one fixed transform pairing this well is not the chance of the best
    /// of many doing so: voting picks its peak from every offset it was offered, and
    /// unrelated sparse fields passed on 3–5 pairs at −6.0 to −7.8 (one pair in ~100 at
    /// 200/50 stars). Scaled by the trials (Bonferroni, ~10^4 offsets over its turns),
    /// none of 8000 unrelated pairs passes, while the fixtures' real voting fits score
    /// −7.0 to −22. The ladder rungs fit from triangle matches rather than trying every
    /// offset, and are not scaled.
    pub fn searched(self, trials: usize) -> Self {
        Self {
            log10_trials: (trials.max(1) as f64).log10(),
            ..self
        }
    }

    /// Whether these pairs are far beyond what chance produces at this density, for as
    /// many transforms as this one was picked from.
    pub fn is_significant(&self) -> bool {
        self.log10_chance + self.log10_trials <= MAX_LOG10_CHANCE
    }

    /// Whether a fit with this support and this transform can be trusted: significant,
    /// and at the scale a sub of the same session must have.
    pub fn is_credible(&self, transform: &AffineTransform) -> bool {
        self.is_significant() && (transform.scale - 1.0).abs() <= MAX_SCALE_DEVIATION
    }
}

/// log10 P(X >= k) for X ~ Poisson(lambda).
///
/// The upper tail is summed from its first term in log space, since for a real fit it
/// lies hundreds of decades below what `1 - cdf` can resolve in f64.
fn log10_poisson_tail(k: usize, lambda: f64) -> f64 {
    if k == 0 {
        return 0.0;
    }
    if lambda <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if k as f64 <= lambda {
        // Past the mean the tail is large, and `1 - cdf` is exact enough.
        let mut term = (-lambda).exp();
        let mut cdf = 0.0;
        for i in 0..k {
            cdf += term;
            term *= lambda / (i + 1) as f64;
        }
        return (1.0 - cdf).max(f64::MIN_POSITIVE).log10();
    }

    let ln_factorial: f64 = (2..=k).map(|i| (i as f64).ln()).sum();
    let ln_first = -lambda + k as f64 * lambda.ln() - ln_factorial;
    // Each further term is the last times lambda / (i + 1) < 1, so the series converges.
    let mut ratio_sum = 1.0;
    let mut ratio = 1.0;
    for i in k + 1.. {
        ratio *= lambda / i as f64;
        ratio_sum += ratio;
        if ratio < 1e-12 * ratio_sum {
            break;
        }
    }
    (ln_first + ratio_sum.ln()) / std::f64::consts::LN_10
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic pseudo-random stars over a `size` x `size` frame.
    fn scatter(count: usize, size: f32, seed: u64) -> Vec<Star> {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        (0..count).map(|_| Star::new(next() * size, next() * size, 1.0, 0.5, 30.0)).collect()
    }

    /// `stars` as a sub shifted by `(dx, dy)` sees them.
    fn shifted(stars: &[Star], dx: f32, dy: f32) -> Vec<Star> {
        stars.iter().map(|s| Star::new(s.x - dx, s.y - dy, s.flux, s.peak, s.snr)).collect()
    }

    #[test]
    fn the_poisson_tail_matches_a_direct_sum() {
        for (k, lambda) in [(1usize, 0.5f64), (3, 0.5), (5, 2.0), (12, 2.5), (2, 4.0)] {
            let mut term = (-lambda).exp();
            let mut cdf = 0.0;
            for i in 0..k {
                cdf += term;
                term *= lambda / (i + 1) as f64;
            }
            let direct = (1.0 - cdf).log10();
            assert!((log10_poisson_tail(k, lambda) - direct).abs() < 1e-9, "k={k} λ={lambda}");
        }
        assert_eq!(log10_poisson_tail(0, 3.0), 0.0);
        assert!(log10_poisson_tail(150, 0.8) < -250.0, "far below f64's 1 - cdf");
    }

    /// The Cat's Eye case in miniature: 15 real stars among 185 detections that do
    /// not repeat. The true shift pairs only the 15, a share no fraction rule accepts.
    #[test]
    fn a_few_real_stars_among_noise_are_credible() {
        let real = scatter(15, 3000.0, 1);
        let mut reference = real.clone();
        reference.extend(scatter(185, 3000.0, 2));
        let mut target = shifted(&real, 4.0, -2.0);
        target.extend(scatter(185, 3000.0, 3));

        let truth = AffineTransform::from_translation(4.0, -2.0);
        let support = Support::measure(&reference, &target, &truth);

        assert!(support.pairs >= 15, "{support:?}");
        assert!(support.is_credible(&truth), "{support:?}");
    }

    #[test]
    fn unrelated_fields_are_not_credible() {
        let reference = scatter(200, 3000.0, 4);
        let target = scatter(200, 3000.0, 5);

        for transform in [
            AffineTransform::identity(),
            AffineTransform::from_translation(40.0, -12.0),
            AffineTransform::new(0.3, 1.0, 100.0, 50.0),
        ] {
            let support = Support::measure(&reference, &target, &transform);
            assert!(!support.is_significant(), "{transform:?}: {support:?}");
        }
    }

    /// Ten pairs mean little where two dense patches overlap, since chance lands about
    /// that many there, and everything where the same detections are spread over the
    /// sky: the expectation has to follow the local density.
    #[test]
    fn pairs_inside_a_cluster_count_for_less_than_on_open_sky() {
        let support_of_ten_pairs = |spread: f32| {
            let mut reference = scatter(45, spread, 6);
            let mut target: Vec<Star> =
                reference.iter().take(10).map(|s| Star::new(s.x + 0.5, s.y, 1.0, 0.5, 30.0)).collect();
            target.extend(scatter(35, spread, 7));
            reference.extend(scatter(150, 3000.0, 8).into_iter().map(|s| Star::new(s.x, s.y + 4000.0, 1.0, 0.5, 30.0)));
            Support::measure(&reference, &target, &AffineTransform::identity())
        };

        let cluster = support_of_ten_pairs(50.0);
        let open_sky = support_of_ten_pairs(3000.0);

        assert!(cluster.pairs >= 10 && open_sky.pairs >= 10, "{cluster:?} {open_sky:?}");
        assert!(!cluster.is_significant(), "{cluster:?}");
        assert!(open_sky.is_significant(), "{open_sky:?}");
    }

    #[test]
    fn a_scale_change_is_not_credible_however_well_it_pairs() {
        let reference = scatter(200, 3000.0, 10);
        let zoom = AffineTransform::new(0.0, 1.05, 0.0, 0.0);
        let target: Vec<Star> = reference
            .iter()
            .map(|s| {
                let (x, y) = zoom.inverse_transform_point(s.x, s.y);
                Star::new(x, y, 1.0, 0.5, 30.0)
            })
            .collect();

        let support = Support::measure(&reference, &target, &zoom);

        assert!(support.is_significant(), "{support:?}");
        assert!(!support.is_credible(&zoom));
    }

    /// Three pairs where chance expects 0.014 are significant for one transform, not
    /// for the best of the ~10 000 offsets a vote picks from.
    #[test]
    fn a_searched_transform_pays_for_its_trials() {
        let support = Support { pairs: 3, expected: 0.0144, log10_chance: log10_poisson_tail(3, 0.0144), log10_trials: 0.0 };

        assert!(support.is_significant(), "{support:?}");
        assert!(!support.searched(10_000).is_significant());
        assert_eq!(support.searched(1), support);
    }
}
