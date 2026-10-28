//! Interleaved-row luminance kernels.
//!
//! These take genuinely interleaved `[R, G, B, R, G, B, ...]` f32 rows, which is what
//! the streaming encoder assembles per row in `render::display`. For the planar
//! `Frame` equivalents see [`super::planar`].

use super::{luminance_x4, scale_lut_lookup, scale_lut_lookup_x4};
use crate::frame::luminance;
use wide::f32x4;

/// SIMD-optimized fused scale lookup using a LUT for tone mapping + contrast.
///
/// Fuses black point subtraction, luminance calculation, and tone mapping/contrast
/// scaling into a single pass. The `scale_lut` holds the curve's scale at each
/// `stretch::scale_lut_luminance`, as `stretch::cached_scale_lut` builds it, and at least
/// two entries.
///
/// Operates on whatever slice it is given — callers drive it over `par_chunks_mut` of whole rows; see `apply_fused_stretch_frame`.
#[inline]
pub fn apply_luminance_scale_lut_simd(
    data: &mut [f32],
    black_point: f32,
    scale_lut: &[f32],
    color_intensity: f32,
) {
    let len = data.len();
    if len < 12 || scale_lut.len() < 2 {
        apply_luminance_scale_lut_scalar(data, black_point, scale_lut, color_intensity);
        return;
    }

    let zero = f32x4::ZERO;
    let bp = f32x4::splat(black_point);
    let one = f32x4::ONE;
    let ci = f32x4::splat(color_intensity);
    let one_minus_ci = f32x4::splat(1.0 - color_intensity);

    let num_pixels = len / 3;
    let chunks = num_pixels / 4;

    for i in 0..chunks {
        let base = i * 12;

        let r = f32x4::new([data[base], data[base + 3], data[base + 6], data[base + 9]]);
        let g = f32x4::new([
            data[base + 1],
            data[base + 4],
            data[base + 7],
            data[base + 10],
        ]);
        let b = f32x4::new([
            data[base + 2],
            data[base + 5],
            data[base + 8],
            data[base + 11],
        ]);

        let r_sub = (r - bp).max(zero);
        let g_sub = (g - bp).max(zero);
        let b_sub = (b - bp).max(zero);

        let lum = luminance_x4(r_sub, g_sub, b_sub);

        let scale = scale_lut_lookup_x4(scale_lut, lum);

        let lum_stretched = lum * scale;
        let base_add = lum_stretched * one_minus_ci;
        let channel_mul = scale * ci;

        let r_out = (r_sub * channel_mul + base_add).max(zero).min(one);
        let g_out = (g_sub * channel_mul + base_add).max(zero).min(one);
        let b_out = (b_sub * channel_mul + base_add).max(zero).min(one);

        let r_arr = r_out.to_array();
        let g_arr = g_out.to_array();
        let b_arr = b_out.to_array();

        data[base] = r_arr[0];
        data[base + 1] = g_arr[0];
        data[base + 2] = b_arr[0];
        data[base + 3] = r_arr[1];
        data[base + 4] = g_arr[1];
        data[base + 5] = b_arr[1];
        data[base + 6] = r_arr[2];
        data[base + 7] = g_arr[2];
        data[base + 8] = b_arr[2];
        data[base + 9] = r_arr[3];
        data[base + 10] = g_arr[3];
        data[base + 11] = b_arr[3];
    }

    apply_luminance_scale_lut_scalar(
        &mut data[chunks * 12..],
        black_point,
        scale_lut,
        color_intensity,
    );
}

#[inline]
pub fn apply_luminance_scale_lut_scalar(
    data: &mut [f32],
    black_point: f32,
    scale_lut: &[f32],
    color_intensity: f32,
) {
    if scale_lut.len() < 2 {
        return;
    }

    for pixel in data.as_chunks_mut::<3>().0 {
        let r = (pixel[0] - black_point).max(0.0);
        let g = (pixel[1] - black_point).max(0.0);
        let b = (pixel[2] - black_point).max(0.0);

        let lum = luminance(r, g, b);
        let scale = scale_lut_lookup(scale_lut, lum);

        let lum_stretched = lum * scale;
        let base_add = lum_stretched * (1.0 - color_intensity);
        let channel_mul = scale * color_intensity;

        pixel[0] = (r * channel_mul + base_add).clamp(0.0, 1.0);
        pixel[1] = (g * channel_mul + base_add).clamp(0.0, 1.0);
        pixel[2] = (b * channel_mul + base_add).clamp(0.0, 1.0);
    }
}

/// SIMD-optimized luminance-preserving transform for RGB pixel data: applies a
/// scalar transform to each pixel's luminance, then scales all RGB channels by the
/// same factor to preserve colour ratios. The surrounding math (dot product,
/// division, multiplication, clamping) is vectorized even though the transform
/// itself runs scalar. Equivalent to `L = luminance(R, G, B); L' =
/// transform_fn(L); scale = L'/L; R' = (R*scale).clamp(0,1)` (and same for G, B).
#[inline]
pub fn apply_luminance_preserving_simd(
    data: &mut [f32],
    color_intensity: f32,
    transform_fn: impl Fn(f32) -> f32,
) {
    let len = data.len();
    if len < 12 {
        apply_luminance_preserving_scalar(data, color_intensity, &transform_fn);
        return;
    }

    let zero = f32x4::ZERO;
    let one = f32x4::ONE;

    // Process 4 pixels (12 floats) at a time
    let num_pixels = len / 3;
    let chunks = num_pixels / 4;

    for i in 0..chunks {
        let base = i * 12;

        // Gather R, G, B for 4 pixels
        let r = f32x4::new([data[base], data[base + 3], data[base + 6], data[base + 9]]);
        let g = f32x4::new([
            data[base + 1],
            data[base + 4],
            data[base + 7],
            data[base + 10],
        ]);
        let b = f32x4::new([
            data[base + 2],
            data[base + 5],
            data[base + 8],
            data[base + 11],
        ]);

        // Compute 4 luminances in SIMD
        let lum = luminance_x4(r, g, b);
        let lum_arr = lum.to_array();

        // Apply transform (scalar) and compute scales
        // scale=0 for dark pixels naturally zeros out RGB via multiplication
        let mut scale_arr = [0.0f32; 4];
        for j in 0..4 {
            if lum_arr[j] > 1e-8 {
                scale_arr[j] = transform_fn(lum_arr[j]) / lum_arr[j];
            }
        }

        let scale = f32x4::new(scale_arr);

        // Compute color intensity factors
        let color_int = f32x4::splat(color_intensity);
        let one_minus_int = f32x4::splat(1.0 - color_intensity);
        let lum_stretched = lum * scale;
        let base_add = lum_stretched * one_minus_int;
        let channel_mul = scale * color_int;

        // Apply scale to RGB channels and clamp
        let r_out = (r * channel_mul + base_add).max(zero).min(one);
        let g_out = (g * channel_mul + base_add).max(zero).min(one);
        let b_out = (b * channel_mul + base_add).max(zero).min(one);

        let r_arr = r_out.to_array();
        let g_arr = g_out.to_array();
        let b_arr = b_out.to_array();

        // Scatter back to interleaved RGB
        data[base] = r_arr[0];
        data[base + 1] = g_arr[0];
        data[base + 2] = b_arr[0];
        data[base + 3] = r_arr[1];
        data[base + 4] = g_arr[1];
        data[base + 5] = b_arr[1];
        data[base + 6] = r_arr[2];
        data[base + 7] = g_arr[2];
        data[base + 8] = b_arr[2];
        data[base + 9] = r_arr[3];
        data[base + 10] = g_arr[3];
        data[base + 11] = b_arr[3];
    }

    // Handle remainder pixels
    apply_luminance_preserving_scalar(&mut data[chunks * 12..], color_intensity, &transform_fn);
}

#[inline]
fn apply_luminance_preserving_scalar(
    data: &mut [f32],
    color_intensity: f32,
    transform_fn: &impl Fn(f32) -> f32,
) {
    for pixel in data.as_chunks_mut::<3>().0 {
        let r = pixel[0];
        let g = pixel[1];
        let b = pixel[2];

        let lum = luminance(r, g, b);

        if lum <= 1e-8 {
            pixel[0] = 0.0;
            pixel[1] = 0.0;
            pixel[2] = 0.0;
            continue;
        }

        let luminance_transformed = transform_fn(lum);
        let scale = luminance_transformed / lum;

        let lum_stretched = lum * scale;
        let base_add = lum_stretched * (1.0 - color_intensity);
        let channel_mul = scale * color_intensity;

        pixel[0] = (r * channel_mul + base_add).clamp(0.0, 1.0);
        pixel[1] = (g * channel_mul + base_add).clamp(0.0, 1.0);
        pixel[2] = (b * channel_mul + base_add).clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::stretch::scale_lut_from;

    #[test]
    fn test_luminance_preserving_identity() {
        let mut data = vec![0.4, 0.2, 0.1, 0.8, 0.7, 0.6, 0.0, 0.0, 0.0, 0.3, 0.5, 0.2];
        let original = data.clone();
        apply_luminance_preserving_simd(&mut data, 1.0, |l| l);
        for (a, b) in data.iter().zip(original.iter()) {
            // Dark pixels get zeroed, others stay the same
            if *b < 1e-6 {
                assert!(a.abs() < 1e-5);
            } else {
                assert!(
                    (a - b).abs() < 1e-4,
                    "identity transform changed value: {} -> {}",
                    b,
                    a
                );
            }
        }
    }

    #[test]
    fn test_luminance_preserving_color_ratios() {
        let r = 0.4f32;
        let g = 0.2;
        let b = 0.1;
        let mut data = vec![r, g, b];
        let orig_rg = r / g;
        let orig_rb = r / b;

        apply_luminance_preserving_simd(&mut data, 1.0, |l| l * 2.0);

        let new_rg = data[0] / data[1];
        let new_rb = data[0] / data[2];
        assert!((orig_rg - new_rg).abs() < 1e-4);
        assert!((orig_rb - new_rb).abs() < 1e-4);
    }

    #[test]
    fn test_luminance_preserving_color_intensity_simd() {
        let r = 0.5f32;
        let g = 0.3;
        let b = 0.7;

        let mut data_1 = vec![r, g, b, r, g, b, r, g, b, r, g, b];
        let mut data_2 = data_1.clone();
        let mut data_0 = data_1.clone();

        // Intensity = 1.0 (normal)
        apply_luminance_preserving_simd(&mut data_1, 1.0, |l| l * 2.0);

        // Intensity = 2.0 (boosted)
        apply_luminance_preserving_simd(&mut data_2, 2.0, |l| l * 2.0);

        // Intensity = 0.0 (monochrome)
        apply_luminance_preserving_simd(&mut data_0, 0.0, |l| l * 2.0);

        let l_out_1 = luminance(data_1[0], data_1[1], data_1[2]);
        let dr_1 = data_1[0] - l_out_1;

        let l_out_2 = luminance(data_2[0], data_2[1], data_2[2]);
        let dr_2 = data_2[0] - l_out_2;

        // Boosted intensity should have channels further from luminance
        assert!(dr_2.abs() > dr_1.abs());

        // Zero intensity should make channels equal (grayscale)
        assert!((data_0[0] - data_0[1]).abs() < 1e-4);
        assert!((data_0[0] - data_0[2]).abs() < 1e-4);
    }
    #[test]
    fn test_luminance_scale_lut_simd() {
        // 5 pixels: 4 go through the SIMD block, the 5th through the scalar tail.
        let mut data = vec![
            0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 0.9, 0.7, 0.2, 0.3, 0.4, 0.05, 0.05, 0.05,
        ];

        let black_point = 0.1;
        let scale_lut = scale_lut_from(|_| 2.0);

        apply_luminance_scale_lut_simd(&mut data, black_point, &scale_lut, 1.0);

        // Pixel 1: [0.1, 0.2, 0.3] -> sub bp -> [0.0, 0.1, 0.2]. Scale = 2.0. Out = [0.0, 0.2, 0.4]
        assert!((data[0] - 0.0).abs() < 1e-5);
        assert!((data[1] - 0.2).abs() < 1e-5);
        assert!((data[2] - 0.4).abs() < 1e-5);

        // Pixel 2: [0.4, 0.5, 0.6] -> sub bp -> [0.3, 0.4, 0.5]. Scale = 2.0. Out = [0.6, 0.8, 1.0]
        assert!((data[3] - 0.6).abs() < 1e-5);
        assert!((data[4] - 0.8).abs() < 1e-5);
        assert!((data[5] - 1.0).abs() < 1e-5);

        // Pixel 3: [0.8, 0.9, 0.7] -> sub bp -> [0.7, 0.8, 0.6]. Out = min([1.4, 1.6, 1.2], 1.0)
        assert!((data[6] - 1.0).abs() < 1e-5);
        assert!((data[7] - 1.0).abs() < 1e-5);
        assert!((data[8] - 1.0).abs() < 1e-5);

        // Pixel 4: [0.2, 0.3, 0.4] -> sub bp -> [0.1, 0.2, 0.3]. Out = [0.2, 0.4, 0.6]
        assert!((data[9] - 0.2).abs() < 1e-5);
        assert!((data[10] - 0.4).abs() < 1e-5);
        assert!((data[11] - 0.6).abs() < 1e-5);

        // Pixel 5 (scalar tail): [0.05, 0.05, 0.05] -> sub bp -> [0.0, 0.0, 0.0]
        assert!((data[12] - 0.0).abs() < 1e-5);
        assert!((data[13] - 0.0).abs() < 1e-5);
        assert!((data[14] - 0.0).abs() < 1e-5);
    }

    /// A ramp LUT makes truncation visible: with truncation every luminance inside a bin
    /// collapses to that bin's entry, so a mid-bin sample reads back the bin's left edge
    /// instead of the interpolated value.
    #[test]
    fn test_luminance_scale_lut_interpolates_between_entries() {
        use crate::render::stretch::{scale_lut_luminance, SCALE_LUT_SIZE};
        let scale_lut: Vec<f32> = (0..SCALE_LUT_SIZE).map(|i| i as f32 / SCALE_LUT_SIZE as f32).collect();

        // Halfway between entries 5000 and 5001, in luminance (0.05 % apart there).
        let lum = (scale_lut_luminance(5000) + scale_lut_luminance(5001)) / 2.0;
        // A grey pixel has luminance equal to its channel value (weights sum to 1.0).
        let mut data = vec![lum; 3];
        apply_luminance_scale_lut_simd(&mut data, 0.0, &scale_lut, 1.0);

        let interpolated = lum * 5000.5 / SCALE_LUT_SIZE as f32;
        let truncated = lum * 5000.0 / SCALE_LUT_SIZE as f32;
        assert!(
            (data[0] / interpolated - 1.0).abs() < 1e-5,
            "expected interpolated {interpolated}, got {} (truncated would be {truncated})",
            data[0]
        );
    }

    /// Regression guard for the shadow-crush bug: `scale_lut[0]` must carry the `L -> 0`
    /// limit of the curve, not 0.0, or every pixel below one bin width goes pure black.
    #[test]
    fn test_luminance_scale_lut_does_not_crush_faint_signal() {
        // A steep MTF-like scale curve: large near zero, falling to 1.0 at white.
        let scale_lut = scale_lut_from(|l| (0.02f32 / l).clamp(1.0, 200.0));

        // Under the first entry's luminance, which every darker pixel reads.
        let lum = 0.5 * crate::render::stretch::scale_lut_luminance(0);
        let mut data = vec![lum; 3];
        apply_luminance_scale_lut_simd(&mut data, 0.0, &scale_lut, 1.0);

        assert!(
            data[0] > 0.0,
            "sub-bin luminance was crushed to pure black: {}",
            data[0]
        );
        assert!((data[0] - data[1]).abs() < 1e-9 && (data[1] - data[2]).abs() < 1e-9);
    }

    /// The fused kernel is driven over `par_chunks_mut` of whole rows, so it has to give
    /// the same answer whether it sees the buffer whole or split. This is the guard for the
    /// parallelisation of `apply_fused_stretch_frame`.
    #[test]
    fn test_luminance_scale_lut_is_invariant_to_row_splitting() {
        let scale_lut = scale_lut_from(|l| 1.0 + l * 3.0);

        // 7 pixels per row exercises both the 4-pixel SIMD block and the scalar tail.
        let row_pixels = 7;
        let rows = 5;
        let mut seed = 0x9E37_79B9u32;
        let base: Vec<f32> = (0..row_pixels * rows * 3)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as f32 / 16_777_216.0
            })
            .collect();

        let mut whole = base.clone();
        apply_luminance_scale_lut_simd(&mut whole, 0.05, &scale_lut, 1.0);

        let mut split = base.clone();
        for row in split.chunks_mut(row_pixels * 3) {
            apply_luminance_scale_lut_simd(row, 0.05, &scale_lut, 1.0);
        }

        assert_eq!(whole, split, "row splitting changed the result");
    }

    /// Out-of-range and NaN luminance must saturate inside the table rather than
    /// extrapolate off the end or wrap to index 0.
    #[test]
    fn test_luminance_scale_lut_handles_out_of_range_input() {
        let scale_lut = scale_lut_from(|l| 1.0 + 255.0 * l);

        // Luminance well above 1.0 must clamp to the last entry, not extrapolate.
        let mut data = vec![5.0f32; 3];
        apply_luminance_scale_lut_simd(&mut data, 0.0, &scale_lut, 1.0);
        for v in &data {
            assert!(v.is_finite() && *v <= 1.0, "got {v}");
        }

        // NaN must not panic or index out of bounds.
        let mut data = vec![f32::NAN, 0.5, 0.5, 0.1, 0.1, 0.1];
        apply_luminance_scale_lut_simd(&mut data, 0.0, &scale_lut, 1.0);
        assert!(data[3].is_finite());
    }
}
