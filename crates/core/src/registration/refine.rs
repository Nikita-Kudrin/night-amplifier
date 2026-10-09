//! Refitting a registration over every detected star.
//!
//! The ladder fits from at most 30 stars and accepts RANSAC inliers up to 6–8 px out, so
//! its transform is a good guess rather than a measurement: on real sessions it sat
//! 1–4 px off at the frame corners against a refit over all ~150 stars. Pairing every
//! star under that guess and refitting with a shrinking radius took stacked half-flux
//! radius down by up to 10 % (19 % in the corners), and it never changes how many subs
//! stack, so it costs no noise.

use crate::detection::Star;

use super::ransac::estimate_transform_from_pairs;
use super::transform::AffineTransform;

/// Pairing radii (px), each pass refitting from the last. Starting at 3 px admits every
/// star the ladder's transform put near its partner; ending at 1.5 px — about the
/// centroid noise of a faint star — keeps a mismatched neighbour from steering the fit.
const RADII: [f32; 3] = [3.0, 2.0, 1.5];

/// Pairs below which a refit is noise rather than an improvement.
const MIN_PAIRS: usize = 8;

/// `initial` refined against every star both lists hold, or `initial` itself when the
/// refit is not clearly better supported: it has to pair at least as many stars within
/// the final radius as `initial` does, which keeps a crowded field's mismatches from
/// pulling the fit off.
pub fn refine_transform(
    reference: &[Star],
    target: &[Star],
    initial: &AffineTransform,
) -> AffineTransform {
    let by_x = ReferenceByX::new(reference);
    let mut refined = *initial;
    for radius in RADII {
        let pairs = by_x.nearest_pairs(target, &refined, radius);
        if pairs.len() < MIN_PAIRS {
            return *initial;
        }
        match estimate_transform_from_pairs(reference, target, &pairs) {
            Some(fit) => refined = fit,
            None => return *initial,
        }
    }

    let final_radius = RADII[RADII.len() - 1];
    let supported = |t: &AffineTransform| by_x.nearest_pairs(target, t, final_radius).len();
    if supported(&refined) >= supported(initial) {
        refined
    } else {
        *initial
    }
}

/// The reference stars indexed by x, so pairing a target star scans only the reference
/// stars within `radius` of it on that axis: a brute-force scan of 200x200 stars over
/// five passes cost 0.62 ms per sub (x86), six times the whole ladder; indexed, 26 us.
struct ReferenceByX<'a> {
    stars: &'a [Star],
    /// Indices into `stars`, ascending by x.
    order: Vec<usize>,
}

impl<'a> ReferenceByX<'a> {
    fn new(stars: &'a [Star]) -> Self {
        let mut order: Vec<usize> = (0..stars.len()).collect();
        order.sort_by(|&a, &b| stars[a].x.total_cmp(&stars[b].x));
        Self { stars, order }
    }

    /// `(reference, target)` index pairs whose positions `transform` brings within
    /// `radius`, one-to-one: a reference star claimed twice keeps its closer partner.
    fn nearest_pairs(
        &self,
        target: &[Star],
        transform: &AffineTransform,
        radius: f32,
    ) -> Vec<(usize, usize)> {
        let mut best: Vec<Option<(usize, f32)>> = vec![None; self.stars.len()];
        for (ti, star) in target.iter().enumerate() {
            let (x, y) = transform.transform_point(star.x, star.y);
            let Some((ri, distance)) = self.nearest(x, y, radius) else {
                continue;
            };
            if best[ri].is_none_or(|(_, held)| distance < held) {
                best[ri] = Some((ti, distance));
            }
        }
        best.iter()
            .enumerate()
            .filter_map(|(ri, pair)| pair.map(|(ti, _)| (ri, ti)))
            .collect()
    }

    /// The reference star nearest `(x, y)` and its squared distance, if it lies within
    /// `radius`. Ties go to the lower index, as a scan in index order would give them.
    fn nearest(&self, x: f32, y: f32, radius: f32) -> Option<(usize, f32)> {
        let start = self.order.partition_point(|&ri| self.stars[ri].x <= x - radius);
        let mut nearest: Option<(usize, f32)> = None;
        for &ri in &self.order[start..] {
            let star = &self.stars[ri];
            if star.x >= x + radius {
                break;
            }
            let squared = (star.x - x).powi(2) + (star.y - y).powi(2);
            if nearest.is_none_or(|(held, d)| squared < d || (squared == d && ri < held)) {
                nearest = Some((ri, squared));
            }
        }
        nearest.filter(|&(_, squared)| squared < radius * radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 60-star grid with deterministic jitter, spread over a 3000 px frame.
    fn field() -> Vec<Star> {
        (0..60)
            .map(|i| {
                let (gx, gy) = ((i % 8) as f32, (i / 8) as f32);
                let jitter = ((i * 7919) % 97) as f32 / 97.0 * 40.0;
                Star::new(200.0 + gx * 350.0 + jitter, 200.0 + gy * 350.0 - jitter, 100.0, 0.5, 30.0)
            })
            .collect()
    }

    /// `stars` as a sub displaced by `truth` would see them (target -> reference is `truth`).
    fn observed(stars: &[Star], truth: &AffineTransform) -> Vec<Star> {
        stars
            .iter()
            .map(|s| {
                let (x, y) = truth.inverse_transform_point(s.x, s.y);
                Star::new(x, y, s.flux, s.peak, s.snr)
            })
            .collect()
    }

    fn corner_error(a: &AffineTransform, b: &AffineTransform) -> f32 {
        let (ax, ay) = a.transform_point(3000.0, 3000.0);
        let (bx, by) = b.transform_point(3000.0, 3000.0);
        (ax - bx).hypot(ay - by)
    }

    #[test]
    fn a_rough_transform_is_pulled_onto_the_stars() {
        let reference = field();
        let truth = AffineTransform::new(0.0008, 1.0, -142.0, 110.0);
        let target = observed(&reference, &truth);
        // 0.03 degrees and a pixel off: what the ladder hands over on a real session.
        let rough = AffineTransform::new(0.0008 + 0.0005, 1.0, -141.0, 110.6);

        let refined = refine_transform(&reference, &target, &rough);

        assert!(corner_error(&rough, &truth) > 1.0);
        assert!(corner_error(&refined, &truth) < 0.05, "{refined:?}");
    }

    /// The pairing as first written: every target star against every reference star.
    fn brute_force_pairs(
        reference: &[Star],
        target: &[Star],
        transform: &AffineTransform,
        radius: f32,
    ) -> Vec<(usize, usize)> {
        let mut best: Vec<Option<(usize, f32)>> = vec![None; reference.len()];
        for (ti, star) in target.iter().enumerate() {
            let (x, y) = transform.transform_point(star.x, star.y);
            let nearest = reference
                .iter()
                .enumerate()
                .map(|(ri, r)| (ri, (r.x - x).hypot(r.y - y)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((ri, distance)) = nearest.filter(|&(_, d)| d < radius) else {
                continue;
            };
            if best[ri].is_none_or(|(_, held)| distance < held) {
                best[ri] = Some((ti, distance));
            }
        }
        best.iter()
            .enumerate()
            .filter_map(|(ri, pair)| pair.map(|(ti, _)| (ri, ti)))
            .collect()
    }

    /// The x index only skips stars that cannot pair: on a crowded field, where most stars
    /// have a neighbour inside every radius, it must pair exactly as a full scan does.
    #[test]
    fn indexed_pairing_matches_a_full_scan() {
        let mut state = 17u64;
        let mut next = move || {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (state >> 33) as f32 / (1u64 << 31) as f32
        };
        let reference: Vec<Star> =
            (0..200).map(|_| Star::new(next() * 120.0, next() * 120.0, 100.0, 0.5, 30.0)).collect();
        let target: Vec<Star> =
            (0..200).map(|_| Star::new(next() * 120.0, next() * 120.0, 100.0, 0.5, 30.0)).collect();
        let by_x = ReferenceByX::new(&reference);

        for transform in [
            AffineTransform::identity(),
            AffineTransform::new(0.01, 1.0, 3.5, -2.0),
            AffineTransform::new(-0.02, 1.01, -1.0, 4.0),
        ] {
            for radius in [1.5, 2.0, 3.0, 8.0] {
                let indexed = by_x.nearest_pairs(&target, &transform, radius);
                assert!(!indexed.is_empty());
                assert_eq!(indexed, brute_force_pairs(&reference, &target, &transform, radius));
            }
        }
    }

    #[test]
    fn too_few_stars_leave_the_transform_alone() {
        let reference: Vec<Star> = field().into_iter().take(MIN_PAIRS - 1).collect();
        let truth = AffineTransform::from_translation(5.0, -3.0);
        let target = observed(&reference, &truth);
        let rough = AffineTransform::from_translation(5.5, -3.0);

        assert_eq!(refine_transform(&reference, &target, &rough), rough);
    }

    /// A refit that agrees with fewer stars than the guess it started from is a
    /// mismatch steering the fit, not a better alignment.
    #[test]
    fn a_wrong_starting_guess_is_not_made_worse() {
        let reference = field();
        let target = observed(&reference, &AffineTransform::from_translation(5.0, -3.0));
        let unrelated = AffineTransform::from_translation(170.0, 170.0);

        assert_eq!(refine_transform(&reference, &target, &unrelated), unrelated);
    }
}
