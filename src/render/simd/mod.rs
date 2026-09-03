//! SIMD-optimised operations for the render pipeline, using the `wide` crate for
//! portable SIMD across x86_64 and ARM. `Frame` is planar but the streaming encoder's
//! rows are interleaved, so luminance kernels exist in both shapes — [`interleaved`]
//! and [`planar`] — each pair pinned together by an equivalence test.
//!
//! Does the interleaved variant earn its keep? On x86-64, SIMD and scalar swap places
//! inside each other's confidence intervals (`render_benchmark`'s `scale_lut` group)
//! — no measurable difference, since the per-lane LUT lookup stays a scalar gather.
//! Kept anyway: the deployment target is a Pi 5, and NEON's stride-3 gather/scatter
//! may trade differently — deleting on x86 evidence alone would be a guess. The
//! planar kernels aren't in question: `apply_fused_stretch_frame` runs the same
//! 12.5M samples in 2.4ms.

use wide::f32x4;

const SIMD_MIN_LEN: usize = 8;

mod interleaved;
mod planar;

pub use interleaved::{
    apply_luminance_preserving_simd, apply_luminance_scale_lut_scalar,
    apply_luminance_scale_lut_simd,
};
pub use planar::{
    apply_luminance_preserving_simd_planar, apply_luminance_scale_lut_scalar_planar,
    apply_luminance_scale_lut_simd_planar,
};

/// SIMD-optimized scalar subtraction with clamping to [0, 1].
///
/// Equivalent to: `data[i] = (data[i] - scalar).clamp(0.0, 1.0)`
#[inline]
pub fn subtract_scalar_clamp_simd(data: &mut [f32], scalar: f32) {
    let len = data.len();
    if len < SIMD_MIN_LEN {
        for v in data.iter_mut() {
            *v = (*v - scalar).clamp(0.0, 1.0);
        }
        return;
    }

    let scalar_vec = f32x4::splat(scalar);
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let v = f32x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
        let result = (v - scalar_vec).max(zero).min(one);
        let arr = result.to_array();
        data[idx] = arr[0];
        data[idx + 1] = arr[1];
        data[idx + 2] = arr[2];
        data[idx + 3] = arr[3];
    }

    for v in data[chunks * 4..].iter_mut() {
        *v = (*v - scalar).clamp(0.0, 1.0);
    }
}

/// SIMD-optimized scalar multiplication with clamping to [0, 1].
///
/// Equivalent to: `data[i] = (data[i] * scalar).clamp(0.0, 1.0)`
#[inline]
pub fn multiply_scalar_clamp_simd(data: &mut [f32], scalar: f32) {
    let len = data.len();
    if len < SIMD_MIN_LEN {
        for v in data.iter_mut() {
            *v = (*v * scalar).clamp(0.0, 1.0);
        }
        return;
    }

    let scalar_vec = f32x4::splat(scalar);
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let v = f32x4::new([data[idx], data[idx + 1], data[idx + 2], data[idx + 3]]);
        let result = (v * scalar_vec).max(zero).min(one);
        let arr = result.to_array();
        data[idx] = arr[0];
        data[idx + 1] = arr[1];
        data[idx + 2] = arr[2];
        data[idx + 3] = arr[3];
    }

    for v in data[chunks * 4..].iter_mut() {
        *v = (*v * scalar).clamp(0.0, 1.0);
    }
}

/// Linearly interpolated lookup into a scale LUT indexed by luminance in `[0, 1]`.
///
/// Interpolating rather than truncating matters because the tone curves are steepest
/// exactly where the sky background sits. With an 8192-entry table a truncated lookup
/// is off by up to ~8 LSB of 8-bit output at aggressive midtones (m ≈ 0.001); with
/// interpolation the worst case drops below 0.15 LSB.
///
/// `lum` is clamped into the table, so a luminance above 1.0 (possible after
/// calibration overshoot) saturates at the last entry instead of extrapolating, and a
/// NaN resolves to the last entry rather than wrapping to index 0.
#[inline(always)]
fn scale_lut_lookup(scale_lut: &[f32], lum: f32) -> f32 {
    let lut_max = (scale_lut.len() - 1) as f32;
    let pos = (lum * lut_max).min(lut_max);
    // len >= 2 is guaranteed by the callers; clamping to len-2 keeps `i0 + 1` in range
    // and turns pos == lut_max into frac == 1.0, i.e. exactly the last entry.
    let i0 = (pos as usize).min(scale_lut.len() - 2);
    let frac = pos - i0 as f32;
    let lo = scale_lut[i0];
    lo + (scale_lut[i0 + 1] - lo) * frac
}

#[cfg(test)]
mod tests {
    //! Cross-layout equivalence, plus the layout-agnostic scalar-op kernels.
    //!
    //! The equivalence tests live here rather than in either submodule because they are
    //! the contract *between* the two.

    use super::*;

    /// Deterministic pseudo-random samples in [0, 1).
    fn noise(n: usize, seed_init: u32) -> Vec<f32> {
        let mut seed = seed_init;
        (0..n)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / 16_777_216.0
            })
            .collect()
    }

    /// Splits interleaved RGB into three planes.
    fn to_planes(interleaved: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let n = interleaved.len() / 3;
        let mut r = Vec::with_capacity(n);
        let mut g = Vec::with_capacity(n);
        let mut b = Vec::with_capacity(n);
        for px in interleaved.chunks_exact(3) {
            r.push(px[0]);
            g.push(px[1]);
            b.push(px[2]);
        }
        (r, g, b)
    }

    fn interleave(r: &[f32], g: &[f32], b: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(r.len() * 3);
        for i in 0..r.len() {
            out.push(r[i]);
            out.push(g[i]);
            out.push(b[i]);
        }
        out
    }

    fn assert_close(a: &[f32], b: &[f32], ctx: &str) {
        assert_eq!(a.len(), b.len(), "{ctx}: length mismatch");
        for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
            assert!((x - y).abs() < 1e-6, "{ctx}: index {i} differs: {x} vs {y}");
        }
    }

    #[test]
    fn test_scale_lut_planar_matches_interleaved() {
        const N: usize = 256;
        let scale_lut: Vec<f32> = (0..N).map(|i| 1.0 + (i as f32 / N as f32) * 3.0).collect();

        // 4099 pixels: exercises many full 4-wide blocks plus a 3-pixel scalar tail.
        for pixels in [4usize, 7, 64, 4099] {
            let base = noise(pixels * 3, 0x1234_5678);

            let mut interleaved = base.clone();
            apply_luminance_scale_lut_simd(&mut interleaved, 0.05, &scale_lut, 1.0);

            let (mut r, mut g, mut b) = to_planes(&base);
            apply_luminance_scale_lut_simd_planar(&mut r, &mut g, &mut b, 0.05, &scale_lut, 1.0);

            assert_close(
                &interleave(&r, &g, &b),
                &interleaved,
                &format!("scale_lut planar vs interleaved, {pixels} px"),
            );
        }
    }

    #[test]
    fn test_scale_lut_planar_simd_matches_scalar() {
        const N: usize = 256;
        let scale_lut: Vec<f32> = (0..N).map(|i| 1.0 + (i as f32 / N as f32) * 3.0).collect();

        for pixels in [4usize, 7, 4096, 4099] {
            let base = noise(pixels * 3, 0xDEAD_BEEF);
            let (mut r1, mut g1, mut b1) = to_planes(&base);
            let (mut r2, mut g2, mut b2) = to_planes(&base);

            apply_luminance_scale_lut_simd_planar(&mut r1, &mut g1, &mut b1, 0.02, &scale_lut, 1.5);
            apply_luminance_scale_lut_scalar_planar(
                &mut r2, &mut g2, &mut b2, 0.02, &scale_lut, 1.5,
            );

            assert_close(&r1, &r2, &format!("scale_lut R, {pixels} px"));
            assert_close(&g1, &g2, &format!("scale_lut G, {pixels} px"));
            assert_close(&b1, &b2, &format!("scale_lut B, {pixels} px"));
        }
    }

    #[test]
    fn test_scale_lut_planar_whole_matches_per_row() {
        const N: usize = 256;
        let scale_lut: Vec<f32> = (0..N).map(|i| 1.0 + i as f32 / N as f32).collect();

        // 7 pixels per row exercises both the 4-pixel block and the scalar tail.
        let (row_pixels, rows) = (7usize, 5usize);
        let base = noise(row_pixels * rows * 3, 0x0BAD_F00D);

        let (mut rw, mut gw, mut bw) = to_planes(&base);
        apply_luminance_scale_lut_simd_planar(&mut rw, &mut gw, &mut bw, 0.05, &scale_lut, 1.0);

        let (mut rs, mut gs, mut bs) = to_planes(&base);
        for ((rr, gr), br) in rs
            .chunks_mut(row_pixels)
            .zip(gs.chunks_mut(row_pixels))
            .zip(bs.chunks_mut(row_pixels))
        {
            apply_luminance_scale_lut_simd_planar(rr, gr, br, 0.05, &scale_lut, 1.0);
        }

        assert_close(&rw, &rs, "scale_lut planar row split R");
        assert_close(&gw, &gs, "scale_lut planar row split G");
        assert_close(&bw, &bs, "scale_lut planar row split B");
    }

    #[test]
    fn test_luminance_preserving_planar_matches_interleaved() {
        for pixels in [4usize, 7, 64, 4099] {
            let base = noise(pixels * 3, 0xFEED_C0DE);

            let mut interleaved = base.clone();
            apply_luminance_preserving_simd(&mut interleaved, 1.0, |l| l * 1.7);

            let (mut r, mut g, mut b) = to_planes(&base);
            apply_luminance_preserving_simd_planar(&mut r, &mut g, &mut b, pixels, 1.0, |l| {
                l * 1.7
            });

            assert_close(
                &interleave(&r, &g, &b),
                &interleaved,
                &format!("luminance_preserving planar vs interleaved, {pixels} px"),
            );
        }
    }

    #[test]
    fn test_luminance_preserving_planar_whole_matches_chunked() {
        let (chunk, pixels) = (7usize, 35usize);
        let base = noise(pixels * 3, 0xC0FF_EE00);

        let (mut rw, mut gw, mut bw) = to_planes(&base);
        apply_luminance_preserving_simd_planar(&mut rw, &mut gw, &mut bw, pixels, 1.0, |l| l * 2.0);

        let (mut rc, mut gc, mut bc) = to_planes(&base);
        apply_luminance_preserving_simd_planar(&mut rc, &mut gc, &mut bc, chunk, 1.0, |l| l * 2.0);

        assert_close(&rw, &rc, "luminance_preserving chunk size R");
        assert_close(&gw, &gc, "luminance_preserving chunk size G");
        assert_close(&bw, &bc, "luminance_preserving chunk size B");
    }

    #[test]
    fn test_planar_handles_out_of_range_and_nan() {
        const N: usize = 256;
        let scale_lut: Vec<f32> = (0..N).map(|i| 1.0 + i as f32).collect();

        // Luminance far above 1.0 must clamp, not extrapolate off the table.
        let mut r = vec![5.0f32; 8];
        let mut g = vec![5.0f32; 8];
        let mut b = vec![5.0f32; 8];
        apply_luminance_scale_lut_simd_planar(&mut r, &mut g, &mut b, 0.0, &scale_lut, 1.0);
        for v in r.iter().chain(g.iter()).chain(b.iter()) {
            assert!(v.is_finite() && *v <= 1.0, "got {v}");
        }

        // NaN must not panic or index out of bounds.
        let mut r = vec![f32::NAN, 0.5, 0.5, 0.5, 0.1, 0.1];
        let mut g = vec![0.5f32; 6];
        let mut b = vec![0.5f32; 6];
        apply_luminance_scale_lut_simd_planar(&mut r, &mut g, &mut b, 0.0, &scale_lut, 1.0);
        assert!(r[4].is_finite());
    }
    #[test]
    fn test_subtract_scalar_clamp_simd() {
        let mut data = vec![0.5f32, 0.3, 0.1, 0.05, 0.8, 0.0, 0.02, 0.9];
        subtract_scalar_clamp_simd(&mut data, 0.1);
        assert!((data[0] - 0.4).abs() < 1e-5);
        assert!((data[1] - 0.2).abs() < 1e-5);
        assert!((data[2] - 0.0).abs() < 1e-5);
        assert!((data[3] - 0.0).abs() < 1e-5);
        assert!((data[4] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn test_multiply_scalar_clamp_simd_matches_scalar() {
        for len in [1usize, 4, 7, 8, 4099] {
            let base = noise(len, 0xABCD_1234);
            let mut simd = base.clone();
            multiply_scalar_clamp_simd(&mut simd, 1.9);
            let expected: Vec<f32> = base.iter().map(|v| (v * 1.9).clamp(0.0, 1.0)).collect();
            assert_close(
                &simd,
                &expected,
                &format!("multiply_scalar_clamp len {len}"),
            );
        }
    }
}
