//! 2D Affine transformation for image registration.
//!
//! Represents rotation, scale, and translation transformations used to align
//! astronomical frames.

use crate::detection::Star;

/// 2D Affine transformation matrix.
///
/// Represents the transformation: x' = R·s·x + t
/// where R is a rotation matrix, s is scale, and t is translation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineTransform {
    /// Rotation angle in radians.
    pub rotation: f32,
    /// Scale factor (usually ~1.0 for astronomical images).
    pub scale: f32,
    /// X translation (shift).
    pub tx: f32,
    /// Y translation (shift).
    pub ty: f32,
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl AffineTransform {
    /// Creates an identity transform (no change).
    pub fn identity() -> Self {
        Self {
            rotation: 0.0,
            scale: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    /// Creates a transform from rotation (radians), scale, and translation.
    pub fn new(rotation: f32, scale: f32, tx: f32, ty: f32) -> Self {
        Self {
            rotation,
            scale,
            tx,
            ty,
        }
    }

    /// Creates a translation-only transform.
    pub fn from_translation(tx: f32, ty: f32) -> Self {
        Self {
            rotation: 0.0,
            scale: 1.0,
            tx,
            ty,
        }
    }

    /// Applies the transform to a point.
    pub fn transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        let cos_r = self.rotation.cos();
        let sin_r = self.rotation.sin();
        let s = self.scale;

        let x_new = s * (cos_r * x - sin_r * y) + self.tx;
        let y_new = s * (sin_r * x + cos_r * y) + self.ty;

        (x_new, y_new)
    }

    /// Applies the inverse transform to a point.
    pub fn inverse_transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        let x_t = x - self.tx;
        let y_t = y - self.ty;

        let cos_r = self.rotation.cos();
        let sin_r = self.rotation.sin();
        let inv_s = 1.0 / self.scale;

        let x_orig = inv_s * (cos_r * x_t + sin_r * y_t);
        let y_orig = inv_s * (-sin_r * x_t + cos_r * y_t);

        (x_orig, y_orig)
    }

    /// Returns the transformation as a 3x3 matrix (row-major).
    ///
    /// ```text
    /// | m[0] m[1] m[2] |   | cos·s  -sin·s  tx |
    /// | m[3] m[4] m[5] | = | sin·s   cos·s  ty |
    /// | m[6] m[7] m[8] |   |   0       0     1 |
    /// ```
    pub fn to_matrix(&self) -> [f32; 9] {
        let cos_r = self.rotation.cos();
        let sin_r = self.rotation.sin();
        let s = self.scale;

        [
            s * cos_r,
            -s * sin_r,
            self.tx,
            s * sin_r,
            s * cos_r,
            self.ty,
            0.0,
            0.0,
            1.0,
        ]
    }

    /// Creates a transform from a 3x3 matrix (row-major).
    pub fn from_matrix(m: &[f32; 9]) -> Self {
        let rotation = m[3].atan2(m[0]);
        let scale = (m[0] * m[0] + m[3] * m[3]).sqrt();

        Self {
            rotation,
            scale,
            tx: m[2],
            ty: m[5],
        }
    }

    /// Computes the residual error for a star pair after transformation.
    pub fn residual(&self, src: &Star, dst: &Star) -> f32 {
        let (tx, ty) = self.transform_point(src.x, src.y);
        let dx = tx - dst.x;
        let dy = ty - dst.y;
        (dx * dx + dy * dy).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn test_affine_transform_identity() {
        let transform = AffineTransform::identity();
        let (x, y) = transform.transform_point(10.0, 20.0);

        assert!((x - 10.0).abs() < 1e-6);
        assert!((y - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_affine_transform_translation() {
        let transform = AffineTransform::new(0.0, 1.0, 5.0, -3.0);
        let (x, y) = transform.transform_point(10.0, 20.0);

        assert!((x - 15.0).abs() < 1e-6);
        assert!((y - 17.0).abs() < 1e-6);
    }

    #[test]
    fn test_affine_transform_rotation() {
        let transform = AffineTransform::new(PI / 2.0, 1.0, 0.0, 0.0);
        let (x, y) = transform.transform_point(10.0, 0.0);

        assert!(x.abs() < 1e-5);
        assert!((y - 10.0).abs() < 1e-5);
    }

    #[test]
    fn test_affine_inverse() {
        let transform = AffineTransform::new(PI / 6.0, 1.02, 15.0, -8.0);
        let (x1, y1) = transform.transform_point(100.0, 150.0);
        let (x2, y2) = transform.inverse_transform_point(x1, y1);

        assert!((x2 - 100.0).abs() < 0.01);
        assert!((y2 - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_matrix_roundtrip() {
        let original = AffineTransform::new(PI / 4.0, 1.0, 10.0, -5.0);
        let matrix = original.to_matrix();
        let recovered = AffineTransform::from_matrix(&matrix);

        assert!((original.rotation - recovered.rotation).abs() < 1e-5);
        assert!((original.scale - recovered.scale).abs() < 1e-5);
        assert!((original.tx - recovered.tx).abs() < 1e-5);
        assert!((original.ty - recovered.ty).abs() < 1e-5);
    }

    #[test]
    fn test_transform_residual() {
        let transform = AffineTransform::identity();
        let src = Star::new(100.0, 100.0, 1000.0, 0.9, 50.0);
        let dst = Star::new(103.0, 104.0, 1000.0, 0.9, 50.0);

        let residual = transform.residual(&src, &dst);
        assert!((residual - 5.0).abs() < 1e-5);
    }
}
