//! Triangle-based star matching.
//!
//! Generates triangles from star lists and finds correspondences between frames
//! using scale-invariant triangle descriptors.

use std::collections::HashMap;

use crate::detection::Star;

use super::config::RegistrationConfig;
use super::triangle::{vertex_opposite_sides, Triangle};

/// Star matcher using triangle similarity.
pub struct TriangleMatcher {
    config: RegistrationConfig,
}

impl TriangleMatcher {
    /// Creates a new triangle matcher with the given configuration.
    pub fn new(config: RegistrationConfig) -> Self {
        Self { config }
    }

    /// Creates a matcher with default settings.
    pub fn with_defaults() -> Self {
        Self::new(RegistrationConfig::default())
    }

    /// Generates triangles from a list of stars.
    ///
    /// Only generates triangles with reasonable sizes (not too large or small).
    pub fn generate_triangles(&self, stars: &[Star]) -> Vec<Triangle> {
        let n = stars.len().min(self.config.max_stars);
        let mut triangles = Vec::new();

        for i in 0..n {
            for j in (i + 1)..n {
                let d_ij = stars[i].distance_to(&stars[j]);

                if !self.is_valid_side_length(d_ij) {
                    continue;
                }

                for k in (j + 1)..n {
                    let d_jk = stars[j].distance_to(&stars[k]);
                    let d_ki = stars[k].distance_to(&stars[i]);

                    if !self.is_valid_side_length(d_jk) || !self.is_valid_side_length(d_ki) {
                        continue;
                    }

                    triangles.push(Triangle::from_stars(stars, i, j, k));
                }
            }
        }

        triangles
    }

    /// Generates triangles with adaptive size constraints based on star distribution.
    pub fn generate_triangles_adaptive(&self, stars: &[Star]) -> Vec<Triangle> {
        let n = stars.len().min(self.config.max_stars);
        if n < 3 {
            return Vec::new();
        }

        let (min_side, max_side) = match self.compute_adaptive_bounds(stars, n) {
            Some(bounds) => bounds,
            None => return self.generate_triangles(stars),
        };

        let mut triangles = Vec::new();

        for i in 0..n {
            for j in (i + 1)..n {
                let d_ij = stars[i].distance_to(&stars[j]);
                if d_ij > max_side || d_ij < min_side {
                    continue;
                }

                for k in (j + 1)..n {
                    let d_jk = stars[j].distance_to(&stars[k]);
                    let d_ki = stars[k].distance_to(&stars[i]);

                    if d_jk > max_side || d_jk < min_side {
                        continue;
                    }
                    if d_ki > max_side || d_ki < min_side {
                        continue;
                    }

                    triangles.push(Triangle::from_stars(stars, i, j, k));
                }
            }
        }

        if triangles.len() < 10 {
            return self.generate_triangles(stars);
        }

        triangles
    }

    /// Finds matching triangles between reference and target star lists.
    ///
    /// Returns pairs of (reference_triangle_index, target_triangle_index).
    pub fn match_triangles(
        &self,
        ref_triangles: &[Triangle],
        tgt_triangles: &[Triangle],
    ) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();

        for (ri, ref_tri) in ref_triangles.iter().enumerate() {
            for (ti, tgt_tri) in tgt_triangles.iter().enumerate() {
                if ref_tri.matches(tgt_tri, self.config.descriptor_tolerance) {
                    matches.push((ri, ti));
                }
            }
        }

        matches
    }

    /// Finds matching triangles with progressive tolerance relaxation.
    pub fn match_triangles_adaptive(
        &self,
        ref_triangles: &[Triangle],
        tgt_triangles: &[Triangle],
    ) -> Vec<(usize, usize)> {
        let tolerances = [
            self.config.descriptor_tolerance,
            self.config.descriptor_tolerance * 1.5,
            self.config.descriptor_tolerance * 2.0,
            self.config.descriptor_tolerance * 3.0,
        ];

        for tol in tolerances {
            let matches = self.match_with_tolerance(ref_triangles, tgt_triangles, tol);
            if matches.len() >= 10 {
                return matches;
            }
        }

        self.match_triangles(ref_triangles, tgt_triangles)
    }

    /// Builds star correspondences from matched triangles using voting.
    ///
    /// Each triangle match votes for its constituent star pairs.
    /// Returns the most-voted star correspondences.
    pub fn vote_correspondences(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        ref_triangles: &[Triangle],
        tgt_triangles: &[Triangle],
        triangle_matches: &[(usize, usize)],
    ) -> Vec<(usize, usize)> {
        let votes = self.collect_votes(
            ref_stars,
            tgt_stars,
            ref_triangles,
            tgt_triangles,
            triangle_matches,
        );

        self.extract_one_to_one_correspondences(votes, ref_stars.len(), tgt_stars.len())
    }

    fn is_valid_side_length(&self, length: f32) -> bool {
        length >= self.config.min_triangle_side && length <= self.config.max_triangle_side
    }

    fn compute_adaptive_bounds(&self, stars: &[Star], n: usize) -> Option<(f32, f32)> {
        let sample_size = n.min(20);
        let mut distances: Vec<f32> = Vec::new();

        for i in 0..sample_size {
            for j in (i + 1)..sample_size {
                distances.push(stars[i].distance_to(&stars[j]));
            }
        }

        if distances.is_empty() {
            return None;
        }

        distances.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median_dist = distances[distances.len() / 2];

        let min_side = (median_dist * 0.1).max(self.config.min_triangle_side);
        let max_side = (median_dist * 4.0).min(self.config.max_triangle_side);

        Some((min_side, max_side))
    }

    fn match_with_tolerance(
        &self,
        ref_triangles: &[Triangle],
        tgt_triangles: &[Triangle],
        tolerance: f32,
    ) -> Vec<(usize, usize)> {
        let mut matches = Vec::new();

        for (ri, ref_tri) in ref_triangles.iter().enumerate() {
            for (ti, tgt_tri) in tgt_triangles.iter().enumerate() {
                if ref_tri.matches(tgt_tri, tolerance) {
                    matches.push((ri, ti));
                }
            }
        }

        matches
    }

    fn collect_votes(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        ref_triangles: &[Triangle],
        tgt_triangles: &[Triangle],
        triangle_matches: &[(usize, usize)],
    ) -> HashMap<(usize, usize), usize> {
        let mut votes: HashMap<(usize, usize), usize> = HashMap::new();

        for &(ri, ti) in triangle_matches {
            let ref_tri = &ref_triangles[ri];
            let tgt_tri = &tgt_triangles[ti];

            let ref_vertex_opposite = vertex_opposite_sides(ref_stars, &ref_tri.indices);
            let tgt_vertex_opposite = vertex_opposite_sides(tgt_stars, &tgt_tri.indices);

            self.vote_vertex_correspondences(
                &mut votes,
                ref_tri,
                tgt_tri,
                &ref_vertex_opposite,
                &tgt_vertex_opposite,
            );
        }

        votes
    }

    fn vote_vertex_correspondences(
        &self,
        votes: &mut HashMap<(usize, usize), usize>,
        ref_tri: &Triangle,
        tgt_tri: &Triangle,
        ref_vertex_opposite: &[f32; 3],
        tgt_vertex_opposite: &[f32; 3],
    ) {
        let tolerance = self.config.descriptor_tolerance * 2.0;

        for (rv, r_opp_side) in ref_vertex_opposite.iter().enumerate() {
            for (tv, t_opp_side) in tgt_vertex_opposite.iter().enumerate() {
                let r_ratio = r_opp_side / ref_tri.sides[2].max(1e-6);
                let t_ratio = t_opp_side / tgt_tri.sides[2].max(1e-6);

                if (r_ratio - t_ratio).abs() < tolerance {
                    let ref_star = ref_tri.indices[rv];
                    let tgt_star = tgt_tri.indices[tv];
                    *votes.entry((ref_star, tgt_star)).or_insert(0) += 1;
                }
            }
        }
    }

    fn extract_one_to_one_correspondences(
        &self,
        votes: HashMap<(usize, usize), usize>,
        ref_count: usize,
        tgt_count: usize,
    ) -> Vec<(usize, usize)> {
        let mut vote_list: Vec<_> = votes.into_iter().collect();
        vote_list.sort_by(|a, b| b.1.cmp(&a.1));

        let mut ref_used = vec![false; ref_count];
        let mut tgt_used = vec![false; tgt_count];
        let mut correspondences = Vec::new();

        for ((ri, ti), _count) in vote_list {
            if !ref_used[ri] && !tgt_used[ti] {
                ref_used[ri] = true;
                tgt_used[ti] = true;
                correspondences.push((ri, ti));
            }
        }

        correspondences
    }
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
            Star::new(120.0, 280.0, 600.0, 0.7, 30.0),
        ]
    }

    #[test]
    fn test_generate_triangles() {
        let stars = create_test_stars();
        let matcher = TriangleMatcher::with_defaults();
        let triangles = matcher.generate_triangles(&stars);

        assert!(!triangles.is_empty());
        assert!(triangles.len() <= 10); // C(5,3) = 10
    }

    #[test]
    fn test_match_triangles() {
        let stars = create_test_stars();
        let matcher = TriangleMatcher::with_defaults();
        let triangles = matcher.generate_triangles(&stars);

        let matches = matcher.match_triangles(&triangles, &triangles);
        assert!(!matches.is_empty());
    }
}
