use crate::error::{Result, StackError};
use crate::frame::Frame;
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

    let data = frame.data_mut();

    if channels == 1 {
        data.par_iter_mut().for_each(|pixel| {
            *pixel = mtf(*pixel, midtone);
        });
    } else {
        data.par_chunks_mut(3).for_each(|pixel| {
            let (r, g, b) = mtf_stretch_color_preserving(pixel[0], pixel[1], pixel[2], midtone);
            pixel[0] = r;
            pixel[1] = g;
            pixel[2] = b;
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
