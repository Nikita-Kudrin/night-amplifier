//! Nearest-neighbour queries over a star list, shared by the refit and the support
//! measurement.

use crate::detection::Star;

use super::transform::AffineTransform;

/// The reference stars indexed by x, so a query scans only the reference stars within
/// `radius` of it on that axis: a brute-force scan of 200x200 stars over the refit's
/// five passes cost 0.62 ms per sub (x86), six times the whole ladder; indexed, 26 us.
pub(super) struct ReferenceByX<'a> {
    stars: &'a [Star],
    /// Indices into `stars`, ascending by x.
    order: Vec<usize>,
}

impl<'a> ReferenceByX<'a> {
    pub(super) fn new(stars: &'a [Star]) -> Self {
        let mut order: Vec<usize> = (0..stars.len()).collect();
        order.sort_by(|&a, &b| stars[a].x.total_cmp(&stars[b].x));
        Self { stars, order }
    }

    /// `(reference, target)` index pairs whose positions `transform` brings within
    /// `radius`, one-to-one: a reference star claimed twice keeps its closer partner.
    pub(super) fn nearest_pairs(
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

    /// Reference stars within `radius` of `(x, y)`.
    pub(super) fn count_within(&self, x: f32, y: f32, radius: f32) -> usize {
        let start = self.order.partition_point(|&ri| self.stars[ri].x <= x - radius);
        self.order[start..]
            .iter()
            .map(|&ri| &self.stars[ri])
            .take_while(|star| star.x < x + radius)
            .filter(|star| (star.x - x).powi(2) + (star.y - y).powi(2) < radius * radius)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn counting_matches_a_full_scan() {
        let reference: Vec<Star> = (0..200)
            .map(|i| Star::new((i * 37 % 120) as f32 + 0.3, (i * 53 % 120) as f32 + 0.7, 1.0, 0.5, 30.0))
            .collect();
        let by_x = ReferenceByX::new(&reference);
        for (x, y, radius) in [(60.0, 60.0, 10.0), (0.0, 0.0, 25.0), (119.0, 3.0, 4.0), (500.0, 500.0, 25.0)] {
            let full = reference.iter().filter(|s| (s.x - x).powi(2) + (s.y - y).powi(2) < radius * radius).count();
            assert_eq!(by_x.count_within(x, y, radius), full);
        }
    }
}
