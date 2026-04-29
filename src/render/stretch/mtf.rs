use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::apply_luminance_preserving_simd;
use rayon::prelude::*;

/// Midtones Transfer Function (MTF)
///
/// `m` is the midtone balance parameter (0.0 to 1.0).
/// m = 0.5 results in no change (linear).
/// m < 0.5 boosts shadows (astrophotography standard).
#[inline]
pub fn mtf(x: f32, m: f32) -> f32 {
    if m == 0.5 {
        return x;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Rational MTF formula used by PixInsight / Siril
    ((m - 1.0) * x) / ((2.0 * m - 1.0) * x - m)
}

/// Apply color-preserving MTF stretch to an RGB pixel
#[inline]
pub fn mtf_stretch_color_preserving(r: f32, g: f32, b: f32, midtone: f32) -> (f32, f32, f32) {
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luminance <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }

    let luminance_stretched = mtf(luminance, midtone);
    let scale = luminance_stretched / luminance;

    (
        (r * scale).clamp(0.0, 1.0),
        (g * scale).clamp(0.0, 1.0),
        (b * scale).clamp(0.0, 1.0),
    )
}

/// Apply MTF to an entire frame in-place
pub fn mtf_stretch_frame(frame: &mut Frame, midtone: f32) -> Result<()> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "mtf_stretch_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    let width = frame.width();
    let data = frame.data_mut();

    // Pre-compute LUT to avoid expensive division per pixel
    const LUT_SIZE: usize = 65536;
    let mut lut = vec![0.0f32; LUT_SIZE];
    for i in 0..LUT_SIZE {
        let x = i as f32 / (LUT_SIZE - 1) as f32;
        lut[i] = mtf(x, midtone);
    }

    if channels == 1 {
        data.par_iter_mut().for_each(|pixel| {
            let idx = (*pixel * 65535.0) as usize;
            *pixel = lut[idx.min(65535)];
        });
    } else {
        let row_len = width * 3;
        data.par_chunks_mut(row_len).for_each(|row| {
            apply_luminance_preserving_simd(row, |l| {
                let idx = (l * 65535.0) as usize;
                lut[idx.min(65535)]
            });
        });
    }

    Ok(())
}

/// Solves for the MTF midtone parameter `m` algebraically
/// (No iterative solver needed like Asinh)
pub fn solve_mtf_midtone(input_median: f32, target_output: f32) -> f32 {
    let x = input_median;
    let t = target_output;

    if x <= 0.0 || t <= 0.0 {
        return 0.5;
    }

    // Algebraic solution to: t = (m-1)x / ((2m-1)x - m)
    let denominator = 2.0 * t * x - t - x;
    if denominator.abs() < 1e-6 {
        return 0.5;
    }

    let m = (x * (t - 1.0)) / denominator;
    m.clamp(0.0001, 0.9999)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mtf_values() {
        let m = 0.2; // Shadow boost
        assert!((mtf(0.0, m) - 0.0).abs() < 1e-6);
        assert!((mtf(1.0, m) - 1.0).abs() < 1e-6);
        
        let mid = mtf(0.1, m);
        assert!(mid > 0.1); // Shadows boosted

        let no_stretch = mtf(0.5, 0.5);
        assert!((no_stretch - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_mtf_stretch_frame() {
        let mut data = vec![0.0f32; 32 * 32 * 3];
        for i in 0..(32 * 32) {
            data[i * 3] = 0.1;
            data[i * 3 + 1] = 0.2;
            data[i * 3 + 2] = 0.3;
        }
        let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();
        let original_pixel = (0.1, 0.2, 0.3);

        mtf_stretch_frame(&mut frame, 0.2).unwrap();

        let r_out = frame.get_pixel(16, 16, 0);
        let g_out = frame.get_pixel(16, 16, 1);
        let b_out = frame.get_pixel(16, 16, 2);

        // Should be boosted
        assert!(r_out > original_pixel.0);
        assert!(g_out > original_pixel.1);
        assert!(b_out > original_pixel.2);

        // Color ratios should be preserved
        let orig_rg = original_pixel.0 / original_pixel.1;
        let new_rg = r_out / g_out;
        assert!((orig_rg - new_rg).abs() < 1e-4);
    }
}
