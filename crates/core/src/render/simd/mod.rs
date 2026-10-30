//! SIMD-optimised render-pipeline ops using the `wide` crate, portable across x86_64 and
//! ARM. Rows come planar (`Frame`) and interleaved (streaming encoder), so luminance
//! kernels exist in both shapes ([`interleaved`], [`planar`]), pinned by an equivalence
//! test. Interleaved SIMD shows no measurable gain over scalar on x86-64
//! (`render_benchmark`'s `scale_lut` group — overlapping confidence intervals, since the
//! per-lane LUT lookup stays a scalar gather), but is kept for the Pi 5 target, where
//! NEON's stride-3 gather/scatter may trade differently and deleting on x86 evidence alone
//! would be a guess. Planar is proven: `apply_fused_stretch_frame` runs 12.5M samples in 2.4ms.

use crate::render::stretch::{scale_lut_position, ScaleLut};
use wide::f32x4;

const SIMD_MIN_LEN: usize = 8;

/// Four Rec.709 luminances at once, in [`crate::frame::luminance`]'s summation order so
/// SIMD bodies and their scalar tails agree bit for bit.
#[inline(always)]
pub fn luminance_x4(r: f32x4, g: f32x4, b: f32x4) -> f32x4 {
    let [wr, wg, wb] = crate::frame::LUMA_REC709;
    f32x4::splat(wr) * r + f32x4::splat(wg) * g + f32x4::splat(wb) * b
}

mod interleaved;
mod planar;

pub use interleaved::{
    apply_luminance_preserving_simd, apply_luminance_scale_lut_scalar,
    apply_luminance_scale_lut_simd, apply_max_channel_scale_lut_scalar,
    apply_max_channel_scale_lut_simd,
};

/// An interleaved row through `scale_lut`, read by the brightness the table is indexed by.
/// `color_intensity` is the luminance kernel's alone.
#[inline]
pub fn apply_scale_lut_simd(data: &mut [f32], black_point: f32, scale_lut: &ScaleLut, color_intensity: f32) {
    match scale_lut.basis() {
        crate::render::stretch::ScaleBasis::Luminance => {
            apply_luminance_scale_lut_simd(data, black_point, scale_lut, color_intensity)
        }
        crate::render::stretch::ScaleBasis::MaxChannel => {
            apply_max_channel_scale_lut_simd(data, black_point, scale_lut)
        }
    }
}
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

/// Linearly interpolated lookup into a scale LUT, at [`scale_lut_position`]: entries
/// 0.39 % of the luminance apart at every level, so interpolation keeps the worst case
/// within 0.15 of an 8-bit level of the curve from 1e-9 to 1, at any curve the solver
/// returns (`stretch::tests`). Above 1.0 (calibration overshoot) and NaN read the last
/// entry.
#[inline(always)]
pub(crate) fn scale_lut_lookup(scale_lut: &ScaleLut, lum: f32) -> f32 {
    let (i, frac) = scale_lut_position(lum);
    let entries = scale_lut.entries();
    let lo = entries[i];
    lo + (entries[i + 1] - lo) * frac
}

/// [`scale_lut_lookup`] for four luminances. A gather either way: per lane it costs what the
/// evenly spread table's multiply and convert did.
#[inline(always)]
pub(crate) fn scale_lut_lookup_x4(scale_lut: &ScaleLut, lum: f32x4) -> f32x4 {
    let [a, b, c, d] = lum.to_array();
    f32x4::new([
        scale_lut_lookup(scale_lut, a),
        scale_lut_lookup(scale_lut, b),
        scale_lut_lookup(scale_lut, c),
        scale_lut_lookup(scale_lut, d),
    ])
}

#[cfg(test)]
mod tests {
    //! Cross-layout equivalence, plus the layout-agnostic scalar-op kernels.
    //!
    //! The equivalence tests live here rather than in either submodule because they are
    //! the contract *between* the two.

    use super::*;
    use crate::render::stretch::scale_lut_from;

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

    #[test]
    fn luminance_x4_matches_the_scalar_luminance_bit_for_bit() {
        let samples = noise(4 * 3 * 64, 7);
        for [r, g, b] in samples.as_chunks::<4>().0.as_chunks::<3>().0 {
            let lanes = luminance_x4(f32x4::new(*r), f32x4::new(*g), f32x4::new(*b)).to_array();
            for j in 0..4 {
                assert_eq!(lanes[j].to_bits(), crate::frame::luminance(r[j], g[j], b[j]).to_bits());
            }
        }
    }

    /// Splits interleaved RGB into three planes.
    fn to_planes(interleaved: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let n = interleaved.len() / 3;
        let mut r = Vec::with_capacity(n);
        let mut g = Vec::with_capacity(n);
        let mut b = Vec::with_capacity(n);
        for px in interleaved.as_chunks::<3>().0 {
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

    fn max_channel_lut() -> ScaleLut {
        scale_lut_from(|v| crate::render::planetary::curve(v / 0.6) / v)
            .with_basis(crate::render::stretch::ScaleBasis::MaxChannel)
    }

    #[test]
    fn the_max_channel_kernel_matches_its_scalar_body_bit_for_bit() {
        let lut = max_channel_lut();
        for pixels in [1usize, 3, 4, 7, 64, 4099] {
            let base = noise(pixels * 3, 0x5EED_0001);
            let (mut simd, mut scalar) = (base.clone(), base);
            apply_max_channel_scale_lut_simd(&mut simd, 0.01, &lut);
            apply_max_channel_scale_lut_scalar(&mut scalar, 0.01, &lut);
            let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&simd), bits(&scalar), "{pixels} px");
        }
    }

    /// Every channel takes the one scale its brightest reads: hue exactly, and that channel
    /// lands on the curve, where luminance scaling ran an H-alpha red off it.
    #[test]
    fn the_max_channel_kernel_keeps_hue_and_puts_the_brightest_channel_on_the_curve() {
        let lut = max_channel_lut();
        let mut row = vec![0.30, 0.03, 0.015, 0.2, 0.25, 0.1, 0.0, 0.0, 0.0, 0.55, 0.5, 0.45];
        let input = row.clone();
        apply_scale_lut_simd(&mut row, 0.0, &lut, 1.7);
        for (out, px) in row.as_chunks::<3>().0.iter().zip(input.as_chunks::<3>().0) {
            let brightest = px[0].max(px[1]).max(px[2]);
            if brightest == 0.0 {
                assert_eq!(*out, [0.0; 3]);
                continue;
            }
            let expected = crate::render::planetary::curve(brightest / 0.6);
            assert!((out[0].max(out[1]).max(out[2]) - expected).abs() < 2e-3, "{px:?} -> {out:?}");
            for c in 0..3 {
                assert!((out[c] * brightest - px[c] * expected).abs() < 2e-3, "{px:?} -> {out:?}: hue moved");
            }
        }
    }

    /// The dispatcher sends a luminance table where the encoder always sent it.
    #[test]
    fn a_luminance_table_reads_through_the_dispatcher_as_it_did_directly() {
        let lut = scale_lut_from(|l| 1.0 + l * 3.0);
        let base = noise(4099 * 3, 0xC0FF_EE00);
        let (mut direct, mut dispatched) = (base.clone(), base);
        apply_luminance_scale_lut_simd(&mut direct, 0.05, &lut, 1.2);
        apply_scale_lut_simd(&mut dispatched, 0.05, &lut, 1.2);
        assert_eq!(direct, dispatched);
    }

    #[test]
    fn test_scale_lut_planar_matches_interleaved() {
        let scale_lut = scale_lut_from(|l| 1.0 + l * 3.0);

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
        let scale_lut = scale_lut_from(|l| 1.0 + l * 3.0);

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
        let scale_lut = scale_lut_from(|l| 1.0 + l);

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
        let scale_lut = scale_lut_from(|l| 1.0 + 255.0 * l);

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
