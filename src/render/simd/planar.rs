//! Planar luminance kernels.
//!
//! These take one contiguous slice per colour plane, which is how `Frame` stores its
//! samples: loads and stores stay contiguous within a plane instead of the stride-3
//! gathers the [`super::interleaved`] variants need.

use super::scale_lut_lookup;
use wide::f32x4;

/// SIMD-optimized fused scale lookup using a LUT for tone mapping + contrast (Planar version).
#[inline]
pub fn apply_luminance_scale_lut_simd_planar(
    r_plane: &mut [f32],
    g_plane: &mut [f32],
    b_plane: &mut [f32],
    black_point: f32,
    scale_lut: &[f32],
    color_intensity: f32,
) {
    let len = r_plane.len();
    if len < 4 || scale_lut.len() < 2 {
        apply_luminance_scale_lut_scalar_planar(
            r_plane,
            g_plane,
            b_plane,
            black_point,
            scale_lut,
            color_intensity,
        );
        return;
    }

    let wr = f32x4::splat(0.2126);
    let wg = f32x4::splat(0.7152);
    let wb = f32x4::splat(0.0722);
    let zero = f32x4::ZERO;
    let bp = f32x4::splat(black_point);
    let one = f32x4::ONE;
    let ci = f32x4::splat(color_intensity);
    let one_minus_ci = f32x4::splat(1.0 - color_intensity);

    // `as_chunks_mut` expresses the contiguous 4-wide load/store directly, so the
    // compiler emits one vector load and one vector store per plane instead of four
    // bounds-checked scalar accesses each. Getting this wrong is why the planar
    // rewrite measured no faster than the scalar loop it was meant to beat.
    let lut_max_scalar = (scale_lut.len() - 1) as f32;
    let lut_max = f32x4::splat(lut_max_scalar);
    let lut_top = scale_lut.len() - 2;

    let (r_body, r_tail) = r_plane.as_chunks_mut::<4>();
    let (g_body, g_tail) = g_plane.as_chunks_mut::<4>();
    let (b_body, b_tail) = b_plane.as_chunks_mut::<4>();

    for ((rc, gc), bc) in r_body
        .iter_mut()
        .zip(g_body.iter_mut())
        .zip(b_body.iter_mut())
    {
        let r = f32x4::new(*rc);
        let g = f32x4::new(*gc);
        let b = f32x4::new(*bc);

        let r_sub = (r - bp).max(zero);
        let g_sub = (g - bp).max(zero);
        let b_sub = (b - bp).max(zero);

        let lum = wr * r_sub + wg * g_sub + wb * b_sub;
        let pos = (lum * lut_max).min(lut_max);

        let pos_arr = pos.to_array();
        let i0 = [
            (pos_arr[0] as usize).min(lut_top),
            (pos_arr[1] as usize).min(lut_top),
            (pos_arr[2] as usize).min(lut_top),
            (pos_arr[3] as usize).min(lut_top),
        ];

        let i0_vec = f32x4::new([i0[0] as f32, i0[1] as f32, i0[2] as f32, i0[3] as f32]);
        let frac = pos - i0_vec;

        let lo = f32x4::new([
            scale_lut[i0[0]],
            scale_lut[i0[1]],
            scale_lut[i0[2]],
            scale_lut[i0[3]],
        ]);
        let hi = f32x4::new([
            scale_lut[i0[0] + 1],
            scale_lut[i0[1] + 1],
            scale_lut[i0[2] + 1],
            scale_lut[i0[3] + 1],
        ]);

        let scale = lo + (hi - lo) * frac;

        let base_add = lum * scale * one_minus_ci;
        let channel_mul = scale * ci;

        *rc = (r_sub * channel_mul + base_add)
            .max(zero)
            .min(one)
            .to_array();
        *gc = (g_sub * channel_mul + base_add)
            .max(zero)
            .min(one)
            .to_array();
        *bc = (b_sub * channel_mul + base_add)
            .max(zero)
            .min(one)
            .to_array();
    }

    apply_luminance_scale_lut_scalar_planar(
        r_tail,
        g_tail,
        b_tail,
        black_point,
        scale_lut,
        color_intensity,
    );
}

#[inline]
pub fn apply_luminance_scale_lut_scalar_planar(
    r_plane: &mut [f32],
    g_plane: &mut [f32],
    b_plane: &mut [f32],
    black_point: f32,
    scale_lut: &[f32],
    color_intensity: f32,
) {
    if scale_lut.len() < 2 {
        return;
    }

    for i in 0..r_plane.len() {
        let r = (r_plane[i] - black_point).max(0.0);
        let g = (g_plane[i] - black_point).max(0.0);
        let b = (b_plane[i] - black_point).max(0.0);

        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        // reuse scale_lut_lookup
        let lut_max = (scale_lut.len() - 1) as f32;
        let pos = (luminance * lut_max).min(lut_max);
        let i0 = (pos as usize).min(scale_lut.len() - 2);
        let frac = pos - i0 as f32;
        let lo = scale_lut[i0];
        let scale = lo + (scale_lut[i0 + 1] - lo) * frac;

        let lum_stretched = luminance * scale;
        let base_add = lum_stretched * (1.0 - color_intensity);
        let channel_mul = scale * color_intensity;

        r_plane[i] = (r * channel_mul + base_add).clamp(0.0, 1.0);
        g_plane[i] = (g * channel_mul + base_add).clamp(0.0, 1.0);
        b_plane[i] = (b * channel_mul + base_add).clamp(0.0, 1.0);
    }
}

#[inline]
pub fn apply_luminance_preserving_simd_planar(
    r_plane: &mut [f32],
    g_plane: &mut [f32],
    b_plane: &mut [f32],
    chunk_size: usize,
    color_intensity: f32,
    transform_fn: impl Fn(f32) -> f32 + Sync + Send,
) {
    let len = r_plane.len();
    if len < 4 {
        apply_luminance_preserving_scalar_planar(
            r_plane,
            g_plane,
            b_plane,
            color_intensity,
            &transform_fn,
        );
        return;
    }

    use rayon::prelude::*;

    r_plane
        .par_chunks_mut(chunk_size)
        .zip(g_plane.par_chunks_mut(chunk_size))
        .zip(b_plane.par_chunks_mut(chunk_size))
        .for_each(|((r_chunk, g_chunk), b_chunk)| {
            let wr = f32x4::splat(0.2126);
            let wg = f32x4::splat(0.7152);
            let wb = f32x4::splat(0.0722);
            let zero = f32x4::ZERO;
            let one = f32x4::ONE;
            let color_int = f32x4::splat(color_intensity);
            let one_minus_int = f32x4::splat(1.0 - color_intensity);

            // Contiguous 4-wide load/store per plane, as above.
            let (r_body, r_tail) = r_chunk.as_chunks_mut::<4>();
            let (g_body, g_tail) = g_chunk.as_chunks_mut::<4>();
            let (b_body, b_tail) = b_chunk.as_chunks_mut::<4>();

            for ((rc, gc), bc) in r_body
                .iter_mut()
                .zip(g_body.iter_mut())
                .zip(b_body.iter_mut())
            {
                let r = f32x4::new(*rc);
                let g = f32x4::new(*gc);
                let b = f32x4::new(*bc);

                let lum = wr * r + wg * g + wb * b;
                let lum_arr = lum.to_array();

                let mut scale_arr = [0.0f32; 4];
                for j in 0..4 {
                    if lum_arr[j] > 1e-8 {
                        scale_arr[j] = transform_fn(lum_arr[j]) / lum_arr[j];
                    }
                }

                let scale = f32x4::new(scale_arr);
                let base_add = lum * scale * one_minus_int;
                let channel_mul = scale * color_int;

                *rc = (r * channel_mul + base_add).max(zero).min(one).to_array();
                *gc = (g * channel_mul + base_add).max(zero).min(one).to_array();
                *bc = (b * channel_mul + base_add).max(zero).min(one).to_array();
            }

            apply_luminance_preserving_scalar_planar(
                r_tail,
                g_tail,
                b_tail,
                color_intensity,
                &transform_fn,
            );
        });
}

#[inline]
fn apply_luminance_preserving_scalar_planar(
    r_plane: &mut [f32],
    g_plane: &mut [f32],
    b_plane: &mut [f32],
    color_intensity: f32,
    transform_fn: &impl Fn(f32) -> f32,
) {
    for i in 0..r_plane.len() {
        let r = r_plane[i];
        let g = g_plane[i];
        let b = b_plane[i];

        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

        if luminance <= 1e-8 {
            r_plane[i] = 0.0;
            g_plane[i] = 0.0;
            b_plane[i] = 0.0;
            continue;
        }

        let luminance_transformed = transform_fn(luminance);
        let scale = luminance_transformed / luminance;

        let lum_stretched = luminance * scale;
        let base_add = lum_stretched * (1.0 - color_intensity);
        let channel_mul = scale * color_intensity;

        r_plane[i] = (r * channel_mul + base_add).clamp(0.0, 1.0);
        g_plane[i] = (g * channel_mul + base_add).clamp(0.0, 1.0);
        b_plane[i] = (b * channel_mul + base_add).clamp(0.0, 1.0);
    }
}
