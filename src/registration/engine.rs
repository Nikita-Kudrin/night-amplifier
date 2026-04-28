//! Core image registration engine.
//!
//! The main registration pipeline that combines triangle matching, voting,
//! and transform estimation.

use crate::detection::Star;
use crate::error::{Result, StackError};
use tracing::{instrument, Span};

use super::config::RegistrationConfig;
use super::matcher::TriangleMatcher;
use super::ransac::{estimate_transform_from_pairs, RansacEstimator};
use super::transform::AffineTransform;

/// Image registration engine.
pub struct ImageRegistration {
    config: RegistrationConfig,
    matcher: TriangleMatcher,
}

impl ImageRegistration {
    /// Creates a new registration engine with the given configuration.
    pub fn new(config: RegistrationConfig) -> Self {
        let matcher = TriangleMatcher::new(config.clone());
        Self { config, matcher }
    }

    /// Creates a registration engine with default settings.
    pub fn with_defaults() -> Self {
        Self::new(RegistrationConfig::default())
    }

    /// Registers a target frame to a reference frame.
    ///
    /// # Arguments
    /// * `ref_stars` - Stars detected in the reference frame
    /// * `tgt_stars` - Stars detected in the target frame to align
    ///
    /// # Returns
    /// The affine transformation that maps target coordinates to reference coordinates.
    #[instrument(skip(self, ref_stars, tgt_stars), fields(
        ref_stars_count = ref_stars.len(),
        tgt_stars_count = tgt_stars.len(),
        use_ransac = self.config.use_ransac,
        matched_stars = tracing::field::Empty,
        rotation_deg = tracing::field::Empty,
        scale = tracing::field::Empty
    ))]
    pub fn register(&self, ref_stars: &[Star], tgt_stars: &[Star]) -> Result<AffineTransform> {
        self.validate_input(ref_stars, tgt_stars)?;

        let (ref_triangles, tgt_triangles) = {
            let _span = tracing::info_span!("generate_triangles").entered();
            let ref_tri = self.matcher.generate_triangles_adaptive(ref_stars);
            let tgt_tri = self.matcher.generate_triangles_adaptive(tgt_stars);
            (ref_tri, tgt_tri)
        };

        self.validate_triangles(&ref_triangles, &tgt_triangles)?;

        let triangle_matches = {
            let _span = tracing::info_span!("match_triangles").entered();
            self.matcher
                .match_triangles_adaptive(&ref_triangles, &tgt_triangles)
        };

        if triangle_matches.is_empty() {
            return Err(StackError::Registration(
                "No matching triangles found between frames".to_string(),
            ));
        }

        let correspondences = {
            let _span = tracing::info_span!("vote_correspondences").entered();
            self.matcher.vote_correspondences(
                ref_stars,
                tgt_stars,
                &ref_triangles,
                &tgt_triangles,
                &triangle_matches,
            )
        };

        if correspondences.len() < self.config.min_matches {
            return Err(StackError::Registration(format!(
                "Not enough matched stars ({}, need {})",
                correspondences.len(),
                self.config.min_matches
            )));
        }

        let (transform, final_correspondences) =
            self.estimate_with_optional_ransac(ref_stars, tgt_stars, &correspondences)?;

        self.validate_transform(ref_stars, tgt_stars, &transform, &final_correspondences)?;

        let span = Span::current();
        span.record("matched_stars", final_correspondences.len());
        span.record("rotation_deg", transform.rotation.to_degrees());
        span.record("scale", transform.scale);

        Ok(transform)
    }

    /// Returns the configuration.
    pub fn config(&self) -> &RegistrationConfig {
        &self.config
    }

    fn validate_input(&self, ref_stars: &[Star], tgt_stars: &[Star]) -> Result<()> {
        if ref_stars.len() < 3 {
            return Err(StackError::Registration(
                "Not enough stars in reference frame (need at least 3)".to_string(),
            ));
        }
        if tgt_stars.len() < 3 {
            return Err(StackError::Registration(
                "Not enough stars in target frame (need at least 3)".to_string(),
            ));
        }
        Ok(())
    }

    fn validate_triangles(
        &self,
        ref_triangles: &[super::triangle::Triangle],
        tgt_triangles: &[super::triangle::Triangle],
    ) -> Result<()> {
        if ref_triangles.len() < self.config.min_triangles {
            return Err(StackError::Registration(format!(
                "Not enough triangles in reference ({}, need {})",
                ref_triangles.len(),
                self.config.min_triangles
            )));
        }
        if tgt_triangles.len() < self.config.min_triangles {
            return Err(StackError::Registration(format!(
                "Not enough triangles in target ({}, need {})",
                tgt_triangles.len(),
                self.config.min_triangles
            )));
        }
        Ok(())
    }

    #[instrument(skip(self, ref_stars, tgt_stars, correspondences))]
    fn estimate_with_optional_ransac(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
    ) -> Result<(AffineTransform, Vec<(usize, usize)>)> {
        if self.config.use_ransac && correspondences.len() >= 4 {
            self.estimate_with_ransac(ref_stars, tgt_stars, correspondences)
        } else {
            let transform = self.estimate_transform(ref_stars, tgt_stars, correspondences)?;
            Ok((transform, correspondences.to_vec()))
        }
    }

    #[instrument(skip(self, ref_stars, tgt_stars, correspondences))]
    fn estimate_with_ransac(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
    ) -> Result<(AffineTransform, Vec<(usize, usize)>)> {
        let ransac =
            RansacEstimator::new(self.config.ransac_iterations, self.config.ransac_threshold);

        match ransac.estimate(ref_stars, tgt_stars, correspondences) {
            Some((transform, inliers)) => {
                if inliers.len() < self.config.min_matches {
                    return Err(StackError::Registration(format!(
                        "RANSAC found only {} inliers, need {}",
                        inliers.len(),
                        self.config.min_matches
                    )));
                }
                let inlier_corrs: Vec<(usize, usize)> =
                    inliers.iter().map(|&i| correspondences[i]).collect();
                Ok((transform, inlier_corrs))
            }
            None => {
                let transform = self.estimate_transform(ref_stars, tgt_stars, correspondences)?;
                Ok((transform, correspondences.to_vec()))
            }
        }
    }

    #[instrument(skip(self, ref_stars, tgt_stars, correspondences))]
    fn estimate_transform(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        correspondences: &[(usize, usize)],
    ) -> Result<AffineTransform> {
        estimate_transform_from_pairs(ref_stars, tgt_stars, correspondences).ok_or_else(|| {
            StackError::Registration(
                "Failed to estimate transform from correspondences".to_string(),
            )
        })
    }

    fn validate_transform(
        &self,
        ref_stars: &[Star],
        tgt_stars: &[Star],
        transform: &AffineTransform,
        correspondences: &[(usize, usize)],
    ) -> Result<()> {
        let mean_residual =
            self.compute_mean_residual(tgt_stars, ref_stars, correspondences, transform);

        if mean_residual > self.config.max_residual {
            return Err(StackError::Registration(format!(
                "Registration failed: mean residual {:.2} > max {:.2}",
                mean_residual, self.config.max_residual
            )));
        }

        Ok(())
    }

    fn compute_mean_residual(
        &self,
        tgt_stars: &[Star],
        ref_stars: &[Star],
        correspondences: &[(usize, usize)],
        transform: &AffineTransform,
    ) -> f32 {
        if correspondences.is_empty() {
            return f32::MAX;
        }

        let sum: f32 = correspondences
            .iter()
            .map(|&(ri, ti)| transform.residual(&tgt_stars[ti], &ref_stars[ri]))
            .sum();

        sum / correspondences.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

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
    fn test_registration_identity() {
        let ref_stars = create_test_stars();
        let tgt_stars = ref_stars.clone();

        let registration = ImageRegistration::with_defaults();
        let transform = registration.register(&ref_stars, &tgt_stars).unwrap();

        assert!(transform.rotation.abs() < 0.01);
        assert!((transform.scale - 1.0).abs() < 0.01);
        assert!(transform.tx.abs() < 1.0);
        assert!(transform.ty.abs() < 1.0);
    }

    #[test]
    fn test_registration_translation() {
        let ref_stars = create_test_stars();
        let tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| Star::new(s.x - 10.0, s.y + 5.0, s.flux, s.peak, s.snr))
            .collect();

        let registration = ImageRegistration::with_defaults();
        let transform = registration.register(&ref_stars, &tgt_stars).unwrap();

        assert!(transform.rotation.abs() < 0.05);
        assert!((transform.scale - 1.0).abs() < 0.05);
        assert!((transform.tx - 10.0).abs() < 2.0);
        assert!((transform.ty - (-5.0)).abs() < 2.0);
    }

    #[test]
    fn test_registration_rotation() {
        let ref_stars = create_test_stars();
        let angle = 5.0 * PI / 180.0;
        let cx = 150.0;
        let cy = 150.0;

        let tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| {
                let dx = s.x - cx;
                let dy = s.y - cy;
                let x = cx + dx * angle.cos() - dy * angle.sin();
                let y = cy + dx * angle.sin() + dy * angle.cos();
                Star::new(x, y, s.flux, s.peak, s.snr)
            })
            .collect();

        let config = RegistrationConfig::default().with_max_residual(5.0);
        let registration = ImageRegistration::new(config);
        let transform = registration.register(&ref_stars, &tgt_stars).unwrap();

        assert!((transform.rotation.abs() - angle).abs() < 0.05);
    }

    #[test]
    fn test_registration_not_enough_stars() {
        let ref_stars = vec![
            Star::new(100.0, 100.0, 1000.0, 0.9, 50.0),
            Star::new(200.0, 100.0, 900.0, 0.85, 45.0),
        ];
        let tgt_stars = ref_stars.clone();

        let registration = ImageRegistration::with_defaults();
        let result = registration.register(&ref_stars, &tgt_stars);

        assert!(result.is_err());
    }

    #[test]
    fn test_registration_with_outliers() {
        let ref_stars = create_test_stars();
        let mut tgt_stars: Vec<Star> = ref_stars
            .iter()
            .map(|s| Star::new(s.x + 5.0, s.y - 3.0, s.flux, s.peak, s.snr))
            .collect();

        tgt_stars.push(Star::new(500.0, 500.0, 1000.0, 0.95, 100.0));

        let config = RegistrationConfig::default().with_ransac(true);
        let registration = ImageRegistration::new(config);
        let transform = registration.register(&ref_stars, &tgt_stars).unwrap();

        assert!((transform.tx - (-5.0)).abs() < 3.0);
        assert!((transform.ty - 3.0).abs() < 3.0);
    }
}
