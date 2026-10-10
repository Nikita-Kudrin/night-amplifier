//! Refitting a registration over every detected star.
//!
//! The ladder fits from at most 30 stars and accepts RANSAC inliers up to 6–8 px out, so
//! its transform is a good guess rather than a measurement: on real sessions it sat
//! 1–4 px off at the frame corners against a refit over all ~150 stars. Pairing every
//! star under that guess and refitting with a shrinking radius took stacked half-flux
//! radius down by up to 10 % (19 % in the corners), and it never changes how many subs
//! stack, so it costs no noise.

use crate::detection::Star;

use super::neighbours::ReferenceByX;
use super::ransac::{estimate_rigid_transform_from_pairs, estimate_transform_from_pairs};
use super::transform::AffineTransform;

/// Pairing radii (px), each pass refitting from the last. Starting at 3 px admits every
/// star the ladder's transform put near its partner; ending at 1.5 px — about the
/// centroid noise of a faint star — keeps a mismatched neighbour from steering the fit.
const RADII: [f32; 3] = [3.0, 2.0, 1.5];

/// Pairs below which a refit with a free scale is noise rather than an improvement.
const MIN_PAIRS: usize = 8;

/// Pairs below which even a rigid refit (scale held at 1) is. Between the two, the scale
/// is held: a thin sub's ladder fit, kept whole, carried its free scale from a handful of
/// loose pairs (Cat's Eye: 1.0034 against its neighbours' 1.0004–1.001, ~6 px at the far
/// corner).
const MIN_RIGID_PAIRS: usize = 3;

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
        let fit = match pairs.len() {
            n if n >= MIN_PAIRS => estimate_transform_from_pairs(reference, target, &pairs),
            n if n >= MIN_RIGID_PAIRS => estimate_rigid_transform_from_pairs(reference, target, &pairs),
            _ => None,
        };
        match fit {
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

    #[test]
    fn too_few_stars_leave_the_transform_alone() {
        let reference: Vec<Star> = field().into_iter().take(MIN_RIGID_PAIRS - 1).collect();
        let truth = AffineTransform::from_translation(5.0, -3.0);
        let target = observed(&reference, &truth);
        let rough = AffineTransform::from_translation(5.5, -3.0);

        assert_eq!(refine_transform(&reference, &target, &rough), rough);
    }

    /// Too few stars for a free scale, enough for rotation and shift: the rung's loose
    /// scale goes, and the fit lands on the stars.
    #[test]
    fn a_thin_fit_is_refitted_at_unit_scale() {
        let reference: Vec<Star> = field().into_iter().take(MIN_PAIRS - 2).collect();
        let truth = AffineTransform::new(0.0008, 1.0, -142.0, 110.0);
        let target = observed(&reference, &truth);
        let loose = AffineTransform::new(0.0008, 1.0015, -142.5, 109.5);

        let refined = refine_transform(&reference, &target, &loose);

        assert_eq!(refined.scale, 1.0);
        assert!(corner_error(&loose, &truth) > 3.0);
        assert!(corner_error(&refined, &truth) < 0.05, "{refined:?}");
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
