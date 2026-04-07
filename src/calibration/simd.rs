//! SIMD-optimized operations for calibration
//!
//! These functions use the `wide` crate for portable SIMD operations
//! that work across x86_64 and ARM architectures.

use wide::f32x4;

/// SIMD-optimized sum computation
#[inline]
pub fn sum_f32_simd(values: &[f32]) -> f32 {
    let len = values.len();

    if len < 8 {
        return values.iter().sum();
    }

    let chunks = values.chunks_exact(4);
    let remainder = chunks.remainder();

    let mut acc = f32x4::ZERO;
    for chunk in chunks {
        let v = f32x4::new([chunk[0], chunk[1], chunk[2], chunk[3]]);
        acc += v;
    }

    let simd_sum = acc.reduce_add();
    let remainder_sum: f32 = remainder.iter().sum();

    simd_sum + remainder_sum
}

/// SIMD-optimized scalar multiplication in-place
#[inline]
pub fn multiply_scalar_simd(values: &mut [f32], scalar: f32) {
    let len = values.len();

    if len < 8 {
        for v in values.iter_mut() {
            *v *= scalar;
        }
        return;
    }

    let scalar_vec = f32x4::splat(scalar);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let v = f32x4::new([
            values[idx],
            values[idx + 1],
            values[idx + 2],
            values[idx + 3],
        ]);
        let result = v * scalar_vec;
        let arr = result.to_array();
        values[idx] = arr[0];
        values[idx + 1] = arr[1];
        values[idx + 2] = arr[2];
        values[idx + 3] = arr[3];
    }

    for v in values[chunks * 4..].iter_mut() {
        *v *= scalar;
    }
}

/// SIMD-optimized minimum clamping in-place
#[inline]
pub fn clamp_min_simd(values: &mut [f32], min_val: f32) {
    let len = values.len();

    if len < 8 {
        for v in values.iter_mut() {
            if *v < min_val {
                *v = min_val;
            }
        }
        return;
    }

    let min_vec = f32x4::splat(min_val);
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let v = f32x4::new([
            values[idx],
            values[idx + 1],
            values[idx + 2],
            values[idx + 3],
        ]);
        let clamped = v.max(min_vec);
        let arr = clamped.to_array();
        values[idx] = arr[0];
        values[idx + 1] = arr[1];
        values[idx + 2] = arr[2];
        values[idx + 3] = arr[3];
    }

    for v in values[chunks * 4..].iter_mut() {
        if *v < min_val {
            *v = min_val;
        }
    }
}

/// SIMD-optimized subtraction with zero clamping (for dark subtraction)
#[inline]
pub fn subtract_clamp_zero_simd(frame: &mut [f32], dark: &[f32]) {
    debug_assert_eq!(frame.len(), dark.len());
    let len = frame.len();

    if len < 8 {
        for (f, &d) in frame.iter_mut().zip(dark.iter()) {
            *f = (*f - d).max(0.0);
        }
        return;
    }

    let zero_vec = f32x4::ZERO;
    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let f_vec = f32x4::new([frame[idx], frame[idx + 1], frame[idx + 2], frame[idx + 3]]);
        let d_vec = f32x4::new([dark[idx], dark[idx + 1], dark[idx + 2], dark[idx + 3]]);
        let diff = f_vec - d_vec;
        let clamped = diff.max(zero_vec);
        let arr = clamped.to_array();
        frame[idx] = arr[0];
        frame[idx + 1] = arr[1];
        frame[idx + 2] = arr[2];
        frame[idx + 3] = arr[3];
    }

    for (f, &d) in frame[chunks * 4..]
        .iter_mut()
        .zip(dark[chunks * 4..].iter())
    {
        *f = (*f - d).max(0.0);
    }
}

/// SIMD-optimized division (for flat field correction)
#[inline]
pub fn divide_simd(frame: &mut [f32], flat: &[f32]) {
    debug_assert_eq!(frame.len(), flat.len());
    let len = frame.len();

    if len < 8 {
        for (f, &fl) in frame.iter_mut().zip(flat.iter()) {
            *f /= fl;
        }
        return;
    }

    let chunks = len / 4;

    for i in 0..chunks {
        let idx = i * 4;
        let f_vec = f32x4::new([frame[idx], frame[idx + 1], frame[idx + 2], frame[idx + 3]]);
        let fl_vec = f32x4::new([flat[idx], flat[idx + 1], flat[idx + 2], flat[idx + 3]]);
        let result = f_vec / fl_vec;
        let arr = result.to_array();
        frame[idx] = arr[0];
        frame[idx + 1] = arr[1];
        frame[idx + 2] = arr[2];
        frame[idx + 3] = arr[3];
    }

    for (f, &fl) in frame[chunks * 4..]
        .iter_mut()
        .zip(flat[chunks * 4..].iter())
    {
        *f /= fl;
    }
}
