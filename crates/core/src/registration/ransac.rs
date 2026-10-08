//! RANSAC-based transform estimation.
//!
//! Provides robust outlier rejection for computing affine transforms from
//! star correspondences.

use crate::detection::Star;

use super::transform::AffineTransform;

/// RANSAC-based transform estimator for robust outlier rejection.
pub struct RansacEstimator {
    iterations: usize,
    threshold: f32,
}

impl RansacEstimator {
    /// Creates a new RANSAC estimator.
    pub fn new(iterations: usize, threshold: f32) -> Self {
        Self {
            iterations,
            threshold,
        }
    }

    /// Estimates transform using RANSAC to handle outliers.
    ///
    /// Returns the best transform and the indices of inlier correspondences.
    pub fn estimate(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
    ) -> Option<(AffineTransform, Vec<usize>)> {
        if correspondences.len() < 3 {
            return None;
        }

        let mut best_transform = AffineTransform::identity();
        let mut best_inliers: Vec<usize> = Vec::new();
        let mut rng = LcgRng::new(12345);

        for _ in 0..self.iterations {
            let sample = self.sample_correspondences(&mut rng, correspondences);
            if sample.is_none() {
                continue;
            }
            let sample = sample.unwrap();

            if let Some(transform) = estimate_transform_from_pairs(ref_stars, tgt_stars, &sample) {
                let inliers = self.find_inliers(ref_stars, tgt_stars, correspondences, &transform);

                if inliers.len() > best_inliers.len() {
                    best_inliers = inliers;
                    best_transform = transform;
                }
            }
        }

        self.refine_with_inliers(
            ref_stars,
            tgt_stars,
            correspondences,
            best_transform,
            best_inliers,
        )
    }

    fn sample_correspondences(
        &self,
        rng: &mut LcgRng,
        correspondences: &[(usize, usize)],
    ) -> Option<Vec<(usize, usize)>> {
        if correspondences.len() < 3 {
            return None;
        }

        let i0 = rng.next() % correspondences.len();
        let mut i1 = rng.next() % correspondences.len();
        let mut i2 = rng.next() % correspondences.len();

        while i1 == i0 {
            i1 = rng.next() % correspondences.len();
        }
        while i2 == i0 || i2 == i1 {
            i2 = rng.next() % correspondences.len();
        }

        Some(vec![
            correspondences[i0],
            correspondences[i1],
            correspondences[i2],
        ])
    }

    fn find_inliers(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
        transform: &AffineTransform,
    ) -> Vec<usize> {
        correspondences
            .iter()
            .enumerate()
            .filter(|(_, &(ri, ti))| {
                transform.residual(&tgt_stars[ti], &ref_stars[ri]) < self.threshold
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    fn refine_with_inliers(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
        best_transform: AffineTransform,
        best_inliers: Vec<usize>,
    ) -> Option<(AffineTransform, Vec<usize>)> {
        if best_inliers.len() < 3 {
            return if best_inliers.len() >= 3 {
                Some((best_transform, best_inliers))
            } else {
                None
            };
        }

        let inlier_correspondences: Vec<(usize, usize)> =
            best_inliers.iter().map(|&i| correspondences[i]).collect();

        if let Some(refined) =
            estimate_transform_from_pairs(ref_stars, tgt_stars, &inlier_correspondences)
        {
            let final_inliers = self.find_inliers(ref_stars, tgt_stars, correspondences, &refined);
            return Some((refined, final_inliers));
        }

        if best_inliers.len() >= 3 {
            Some((best_transform, best_inliers))
        } else {
            None
        }
    }
}

/// Simple linear congruential generator for deterministic random selection.
struct LcgRng {
    state: u64,
}

impl LcgRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next(&mut self) -> usize {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 33) as usize
    }
}

/// Estimates transform from a set of star correspondences using least squares.
pub fn estimate_transform_from_pairs(
    ref_stars: &[Star],
    tgt_stars: &[Star],
    correspondences: &[(usize, usize)],
) -> Option<AffineTransform> {
    if correspondences.len() < 2 {
        return None;
    }

    let (ref_cx, ref_cy) = compute_centroid(ref_stars, correspondences.iter().map(|&(r, _)| r));
    let (tgt_cx, tgt_cy) = compute_centroid(tgt_stars, correspondences.iter().map(|&(_, t)| t));

    let (sum_tgt_sq, sum_cross_1, sum_cross_2) = compute_procrustes_sums(
        ref_stars,
        tgt_stars,
        correspondences,
        ref_cx,
        ref_cy,
        tgt_cx,
        tgt_cy,
    );

    if sum_tgt_sq < 1e-10 {
        return None;
    }

    let rotation = (sum_cross_2.atan2(sum_cross_1)) as f32;
    let denom = sum_tgt_sq.max(1e-10);
    let scale = ((sum_cross_1 * sum_cross_1 + sum_cross_2 * sum_cross_2).sqrt() / denom) as f32;
    let scale = scale.clamp(0.90, 1.10);

    let cos_r = rotation.cos();
    let sin_r = rotation.sin();
    let tx = ref_cx - scale * (cos_r * tgt_cx - sin_r * tgt_cy);
    let ty = ref_cy - scale * (sin_r * tgt_cx + cos_r * tgt_cy);

    Some(AffineTransform::new(rotation, scale, tx, ty))
}

fn compute_centroid<I: Iterator<Item = usize> + Clone>(stars: &[Star], indices: I) -> (f32, f32) {
    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    let mut count = 0;

    for i in indices {
        sum_x += stars[i].x;
        sum_y += stars[i].y;
        count += 1;
    }

    if count > 0 {
        (sum_x / count as f32, sum_y / count as f32)
    } else {
        (0.0, 0.0)
    }
}

fn compute_procrustes_sums(
    ref_stars: &[Star],
    tgt_stars: &[Star],
    correspondences: &[(usize, usize)],
    ref_cx: f32,
    ref_cy: f32,
    tgt_cx: f32,
    tgt_cy: f32,
) -> (f64, f64, f64) {
    let mut sum_tgt_sq = 0.0f64;
    let mut sum_cross_1 = 0.0f64;
    let mut sum_cross_2 = 0.0f64;

    for &(ri, ti) in correspondences {
        let rx = (ref_stars[ri].x - ref_cx) as f64;
        let ry = (ref_stars[ri].y - ref_cy) as f64;
        let tx = (tgt_stars[ti].x - tgt_cx) as f64;
        let ty = (tgt_stars[ti].y - tgt_cy) as f64;

        sum_tgt_sq += tx * tx + ty * ty;
        sum_cross_1 += tx * rx + ty * ry;
        sum_cross_2 += tx * ry - ty * rx;
    }

    (sum_tgt_sq, sum_cross_1, sum_cross_2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_stars() -> Vec<Star> {
        vec![
            Star::new(100.0, 100.0, 1000.0, 0.9, 50.0),
            Star::new(200.0, 100.0, 900.0, 0.85, 45.0),
            Star::new(150.0, 200.0, 800.0, 0.8, 40.0),
            Star::new(250.0, 180.0, 700.0, 0.75, 35.0),
        ]
    }

    #[test]
    fn test_estimate_transform_identity() {
        let stars = create_test_stars();
        let correspondences: Vec<(usize, usize)> = (0..stars.len()).map(|i| (i, i)).collect();

        let transform = estimate_transform_from_pairs(&stars, &stars, &correspondences).unwrap();

        assert!(transform.rotation.abs() < 0.01);
        assert!((transform.scale - 1.0).abs() < 0.01);
        assert!(transform.tx.abs() < 1.0);
        assert!(transform.ty.abs() < 1.0);
    }

    #[test]
    fn test_ransac_with_outliers() {
        let ref_stars = create_test_stars();
        let mut tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| Star::new(s.x + 5.0, s.y - 3.0, s.flux, s.peak, s.snr))
            .collect();

        tgt_stars.push(Star::new(500.0, 500.0, 1000.0, 0.95, 100.0));

        let correspondences: Vec<(usize, usize)> = (0..ref_stars.len()).map(|i| (i, i)).collect();

        let ransac = RansacEstimator::new(100, 5.0);
        let result = ransac.estimate(&ref_stars, &tgt_stars, &correspondences);

        assert!(result.is_some());
        let (transform, inliers) = result.unwrap();
        assert!(inliers.len() >= 3);
        assert!((transform.tx - (-5.0)).abs() < 3.0);
        assert!((transform.ty - 3.0).abs() < 3.0);
    }
}
