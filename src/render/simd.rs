//! SIMD-optimized operations for the render pipeline
//!
//! These functions use the `wide` crate for portable SIMD operations
//! that work across x86_64 and ARM architectures.

use wide::f32x4;

const SIMD_MIN_LEN: usize = 8;

/// SIMD-optimized per-channel RGB subtraction with clamping to [0, 1].
///
/// Subtracts `offsets[0..3]` from repeating R,G,B triplets across the slice.
/// Equivalent to: `data[i*3+c] = (data[i*3+c] - offsets[c]).clamp(0.0, 1.0)`
#[inline]
pub fn subtract_rgb_clamp_simd(data: &mut [f32], offsets: &[f32; 3]) {
    let len = data.len();
    if len < SIMD_MIN_LEN {
        subtract_rgb_clamp_scalar(data, offsets);
        return;
    }

    // Process 4 pixels (12 floats) at a time
    // Load offsets as repeating pattern: [R,G,B,R, G,B,R,G, B,R,G,B]
    let off_a = f32x4::new([offsets[0], offsets[1], offsets[2], offsets[0]]);
    let off_b = f32x4::new([offsets[1], offsets[2], offsets[0], offsets[1]]);
    let off_c = f32x4::new([offsets[2], offsets[0], offsets[1], offsets[2]]);
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;

    let chunks = len / 12;

    for i in 0..chunks {
        let base = i * 12;

        let a = f32x4::new([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        let b = f32x4::new([data[base + 4], data[base + 5], data[base + 6], data[base + 7]]);
        let c = f32x4::new([data[base + 8], data[base + 9], data[base + 10], data[base + 11]]);

        let ra = (a - off_a).max(zero).min(one);
        let rb = (b - off_b).max(zero).min(one);
        let rc = (c - off_c).max(zero).min(one);

        let arr_a = ra.to_array();
        let arr_b = rb.to_array();
        let arr_c = rc.to_array();

        data[base] = arr_a[0];
        data[base + 1] = arr_a[1];
        data[base + 2] = arr_a[2];
        data[base + 3] = arr_a[3];
        data[base + 4] = arr_b[0];
        data[base + 5] = arr_b[1];
        data[base + 6] = arr_b[2];
        data[base + 7] = arr_b[3];
        data[base + 8] = arr_c[0];
        data[base + 9] = arr_c[1];
        data[base + 10] = arr_c[2];
        data[base + 11] = arr_c[3];
    }

    subtract_rgb_clamp_scalar(&mut data[chunks * 12..], offsets);
}

#[inline]
fn subtract_rgb_clamp_scalar(data: &mut [f32], offsets: &[f32; 3]) {
    for chunk in data.chunks_exact_mut(3) {
        chunk[0] = (chunk[0] - offsets[0]).clamp(0.0, 1.0);
        chunk[1] = (chunk[1] - offsets[1]).clamp(0.0, 1.0);
        chunk[2] = (chunk[2] - offsets[2]).clamp(0.0, 1.0);
    }
}

/// SIMD-optimized per-channel RGB multiplication with clamping to [0, 1].
///
/// Multiplies repeating R,G,B triplets by `multipliers[0..3]`.
/// Equivalent to: `data[i*3+c] = (data[i*3+c] * multipliers[c]).clamp(0.0, 1.0)`
#[inline]
pub fn multiply_rgb_clamp_simd(data: &mut [f32], multipliers: &[f32; 3]) {
    let len = data.len();
    if len < SIMD_MIN_LEN {
        multiply_rgb_clamp_scalar(data, multipliers);
        return;
    }

    let mul_a = f32x4::new([multipliers[0], multipliers[1], multipliers[2], multipliers[0]]);
    let mul_b = f32x4::new([multipliers[1], multipliers[2], multipliers[0], multipliers[1]]);
    let mul_c = f32x4::new([multipliers[2], multipliers[0], multipliers[1], multipliers[2]]);
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;

    let chunks = len / 12;

    for i in 0..chunks {
        let base = i * 12;

        let a = f32x4::new([data[base], data[base + 1], data[base + 2], data[base + 3]]);
        let b = f32x4::new([data[base + 4], data[base + 5], data[base + 6], data[base + 7]]);
        let c = f32x4::new([data[base + 8], data[base + 9], data[base + 10], data[base + 11]]);

        let ra = (a * mul_a).max(zero).min(one);
        let rb = (b * mul_b).max(zero).min(one);
        let rc = (c * mul_c).max(zero).min(one);

        let arr_a = ra.to_array();
        let arr_b = rb.to_array();
        let arr_c = rc.to_array();

        data[base] = arr_a[0];
        data[base + 1] = arr_a[1];
        data[base + 2] = arr_a[2];
        data[base + 3] = arr_a[3];
        data[base + 4] = arr_b[0];
        data[base + 5] = arr_b[1];
        data[base + 6] = arr_b[2];
        data[base + 7] = arr_b[3];
        data[base + 8] = arr_c[0];
        data[base + 9] = arr_c[1];
        data[base + 10] = arr_c[2];
        data[base + 11] = arr_c[3];
    }

    multiply_rgb_clamp_scalar(&mut data[chunks * 12..], multipliers);
}

#[inline]
fn multiply_rgb_clamp_scalar(data: &mut [f32], multipliers: &[f32; 3]) {
    for chunk in data.chunks_exact_mut(3) {
        chunk[0] = (chunk[0] * multipliers[0]).clamp(0.0, 1.0);
        chunk[1] = (chunk[1] * multipliers[1]).clamp(0.0, 1.0);
        chunk[2] = (chunk[2] * multipliers[2]).clamp(0.0, 1.0);
    }
}

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

/// SIMD-optimized luminance-preserving transform for RGB pixel data.
///
/// Applies a scalar transform function to the luminance of each pixel,
/// then scales all RGB channels by the same factor to preserve color ratios.
///
/// The surrounding math (luminance dot product, division, multiplication, clamping)
/// is vectorized even though the transform function itself runs scalar.
///
/// Equivalent to:
/// ```text
/// L = 0.2126*R + 0.7152*G + 0.0722*B
/// L' = transform_fn(L)
/// scale = L' / L
/// R' = (R * scale).clamp(0, 1)
/// ```
#[inline]
pub fn apply_luminance_preserving_simd(
    data: &mut [f32],
    transform_fn: impl Fn(f32) -> f32,
) {
    let len = data.len();
    if len < 12 {
        apply_luminance_preserving_scalar(data, &transform_fn);
        return;
    }

    let wr = f32x4::splat(0.2126);
    let wg = f32x4::splat(0.7152);
    let wb = f32x4::splat(0.0722);
    let zero = f32x4::ZERO;
    let one = f32x4::ONE;

    // Process 4 pixels (12 floats) at a time
    let num_pixels = len / 3;
    let chunks = num_pixels / 4;

    for i in 0..chunks {
        let base = i * 12;

        // Gather R, G, B for 4 pixels
        let r = f32x4::new([data[base], data[base + 3], data[base + 6], data[base + 9]]);
        let g = f32x4::new([data[base + 1], data[base + 4], data[base + 7], data[base + 10]]);
        let b = f32x4::new([data[base + 2], data[base + 5], data[base + 8], data[base + 11]]);

        // Compute 4 luminances in SIMD
        let lum = wr * r + wg * g + wb * b;
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

        // Apply scale to RGB channels and clamp
        let r_out = (r * scale).max(zero).min(one);
        let g_out = (g * scale).max(zero).min(one);
        let b_out = (b * scale).max(zero).min(one);

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
    apply_luminance_preserving_scalar(&mut data[chunks * 12..], &transform_fn);
}

#[inline]
fn apply_luminance_preserving_scalar(
    data: &mut [f32],
    transform_fn: &impl Fn(f32) -> f32,
) {
    for pixel in data.chunks_exact_mut(3) {
        let r = pixel[0];
        let g = pixel[1];
        let b = pixel[2];

        let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

        if luminance <= 1e-8 {
            pixel[0] = 0.0;
            pixel[1] = 0.0;
            pixel[2] = 0.0;
            continue;
        }

        let luminance_transformed = transform_fn(luminance);
        let scale = luminance_transformed / luminance;

        pixel[0] = (r * scale).clamp(0.0, 1.0);
        pixel[1] = (g * scale).clamp(0.0, 1.0);
        pixel[2] = (b * scale).clamp(0.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtract_rgb_clamp_simd() {
        let mut data = vec![0.5, 0.6, 0.7, 0.3, 0.4, 0.5, 0.1, 0.2, 0.3, 0.05, 0.1, 0.15];
        let offsets = [0.1, 0.2, 0.3];
        subtract_rgb_clamp_simd(&mut data, &offsets);
        assert!((data[0] - 0.4).abs() < 1e-5);
        assert!((data[1] - 0.4).abs() < 1e-5);
        assert!((data[2] - 0.4).abs() < 1e-5);
        assert!((data[3] - 0.2).abs() < 1e-5);
        // Clamp to 0
        assert!((data[6] - 0.0).abs() < 1e-5);
    }

    #[test]
    fn test_subtract_rgb_clamp_simd_small() {
        let mut data = vec![0.5, 0.6, 0.7];
        let offsets = [0.1, 0.2, 0.3];
        subtract_rgb_clamp_simd(&mut data, &offsets);
        assert!((data[0] - 0.4).abs() < 1e-5);
        assert!((data[1] - 0.4).abs() < 1e-5);
        assert!((data[2] - 0.4).abs() < 1e-5);
    }

    #[test]
    fn test_multiply_rgb_clamp_simd() {
        let mut data = vec![0.5, 0.6, 0.7, 0.3, 0.4, 0.5, 0.8, 0.9, 0.7, 0.2, 0.3, 0.4];
        let multipliers = [1.2, 0.8, 1.0];
        multiply_rgb_clamp_simd(&mut data, &multipliers);
        assert!((data[0] - 0.6).abs() < 1e-5);
        assert!((data[1] - 0.48).abs() < 1e-5);
        assert!((data[2] - 0.7).abs() < 1e-5);
        // Clamped to 1.0
        assert!((data[6] - 0.96).abs() < 1e-5);
    }

    #[test]
    fn test_subtract_scalar_clamp_simd() {
        let mut data = vec![0.5, 0.3, 0.1, 0.05, 0.8, 0.0, 0.02, 0.9];
        subtract_scalar_clamp_simd(&mut data, 0.1);
        assert!((data[0] - 0.4).abs() < 1e-5);
        assert!((data[1] - 0.2).abs() < 1e-5);
        assert!((data[2] - 0.0).abs() < 1e-5);
        assert!((data[3] - 0.0).abs() < 1e-5);
        assert!((data[4] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn test_luminance_preserving_identity() {
        let mut data = vec![0.4, 0.2, 0.1, 0.8, 0.7, 0.6, 0.0, 0.0, 0.0, 0.3, 0.5, 0.2];
        let original = data.clone();
        apply_luminance_preserving_simd(&mut data, |l| l);
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

        apply_luminance_preserving_simd(&mut data, |l| l * 2.0);

        let new_rg = data[0] / data[1];
        let new_rb = data[0] / data[2];
        assert!((orig_rg - new_rg).abs() < 1e-4);
        assert!((orig_rb - new_rb).abs() < 1e-4);
    }
}
