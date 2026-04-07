use crate::registration::AffineTransform;

/// Pre-computed inverse transform coefficients for efficient per-pixel transformation.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InverseTransformCache {
    /// Coefficient for x in source x calculation (inv_scale * cos)
    pub a: f32,
    /// Coefficient for y in source x calculation (inv_scale * sin)
    pub b: f32,
    /// Coefficient for x in source y calculation (-inv_scale * sin)
    pub c: f32,
    /// Coefficient for y in source y calculation (inv_scale * cos)
    pub d: f32,
    /// Translation x
    pub tx: f32,
    /// Translation y
    pub ty: f32,
}

impl InverseTransformCache {
    /// Pre-compute inverse transform coefficients from an AffineTransform.
    #[inline]
    pub fn from_transform(transform: &AffineTransform) -> Self {
        let cos_r = transform.rotation.cos();
        let sin_r = transform.rotation.sin();
        let inv_s = 1.0 / transform.scale;

        Self {
            a: inv_s * cos_r,
            b: inv_s * sin_r,
            c: -inv_s * sin_r,
            d: inv_s * cos_r,
            tx: transform.tx,
            ty: transform.ty,
        }
    }

    /// Apply the inverse transform to a point using cached coefficients.
    #[inline]
    pub fn inverse_transform_point(&self, x: f32, y: f32) -> (f32, f32) {
        let x_t = x - self.tx;
        let y_t = y - self.ty;

        let x_orig = self.a * x_t + self.b * y_t;
        let y_orig = self.c * x_t + self.d * y_t;

        (x_orig, y_orig)
    }

    /// Returns the step values for incremental x computation.
    #[inline]
    pub fn x_step(&self) -> (f32, f32) {
        (self.a, self.c)
    }
}
