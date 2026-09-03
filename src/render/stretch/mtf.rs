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
pub fn mtf_stretch_color_preserving(
    r: f32,
    g: f32,
    b: f32,
    midtone: f32,
    color_intensity: f32,
) -> (f32, f32, f32) {
    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if luminance <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }

    let luminance_stretched = mtf(luminance, midtone);
    let scale = luminance_stretched / luminance;

    let lum_stretched = luminance * scale;
    let base_add = lum_stretched * (1.0 - color_intensity);
    let channel_mul = scale * color_intensity;

    (
        (r * channel_mul + base_add).clamp(0.0, 1.0),
        (g * channel_mul + base_add).clamp(0.0, 1.0),
        (b * channel_mul + base_add).clamp(0.0, 1.0),
    )
}

/// Entries in a per-channel MTF lookup table.
const LUT_SIZE: usize = 65536;

/// Tabulate `mtf(x, midtone)` over `[0, 1]`, so the per-pixel work is one index.
fn build_mtf_lut(midtone: f32) -> Vec<f32> {
    (0..LUT_SIZE)
        .map(|i| mtf(i as f32 / (LUT_SIZE - 1) as f32, midtone))
        .collect()
}

/// Index a 65536-entry LUT by a normalised sample.
///
/// The `as usize` cast saturates, so a negative sample lands at 0 rather than wrapping.
#[inline]
fn lut_lookup(lut: &[f32], value: f32) -> f32 {
    lut[((value * 65535.0) as usize).min(LUT_SIZE - 1)]
}

/// Apply MTF to an entire frame in-place. `midtone == 0.5` is the identity (see
/// `mtf`), skipping the pass and its incidental `[0,1]` clamp — safe since pixel
/// data is normalised to `[0,1]` by contract, and the 1e-6 window keeps any residual
/// curve error four orders of magnitude below one 8-bit LSB.
///
/// No `color_intensity` parameter: unlike
/// [`asinh_stretch_frame`](crate::render::stretch::asinh_stretch_frame), this applies
/// the curve to each channel independently rather than scaling by a luminance ratio.
/// Used to *accept* one, bound as `_color_intensity` — silently ignored with no sign
/// in the signature. [`mtf_stretch_color_preserving`] is the colour-preserving
/// per-pixel variant that does take one.
pub fn mtf_stretch_frame(frame: &mut Frame, midtones: [f32; 3]) -> Result<()> {
    if (midtones[0] - 0.5).abs() < 1e-6
        && (midtones[1] - 0.5).abs() < 1e-6
        && (midtones[2] - 0.5).abs() < 1e-6
    {
        return Ok(());
    }

    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "mtf_stretch_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    // One LUT per channel the frame actually has. Building all three unconditionally
    // allocated and filled 768 KB for a mono frame that reads only the first.
    if channels == 1 {
        let lut = build_mtf_lut(midtones[0]);
        frame
            .data_mut()
            .par_iter_mut()
            .for_each(|pixel| *pixel = lut_lookup(&lut, *pixel));
        return Ok(());
    }

    let lut_r = build_mtf_lut(midtones[0]);
    let lut_g = build_mtf_lut(midtones[1]);
    let lut_b = build_mtf_lut(midtones[2]);

    let (r, g, b) = frame.planes_mut();
    r.par_iter_mut()
        .zip_eq(g.par_iter_mut())
        .zip_eq(b.par_iter_mut())
        .for_each(|((p_r, p_g), p_b)| {
            *p_r = lut_lookup(&lut_r, *p_r);
            *p_g = lut_lookup(&lut_g, *p_g);
            *p_b = lut_lookup(&lut_b, *p_b);
        });

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
    fn test_mtf_color_intensity() {
        let r = 0.5;
        let g = 0.3;
        let b = 0.7;
        let m = 0.2;

        let (r_out_1, g_out_1, b_out_1) = mtf_stretch_color_preserving(r, g, b, m, 1.0);
        let (r_out_high, g_out_high, b_out_high) = mtf_stretch_color_preserving(r, g, b, m, 2.0);
        let (r_out_zero, g_out_zero, b_out_zero) = mtf_stretch_color_preserving(r, g, b, m, 0.0);

        // Zero intensity means luminance only
        assert!((r_out_zero - g_out_zero).abs() < 1e-4);
        assert!((r_out_zero - b_out_zero).abs() < 1e-4);

        // High intensity means channels are further from luminance
        let l_out = 0.2126 * r_out_1 + 0.7152 * g_out_1 + 0.0722 * b_out_1;
        let dr_1 = r_out_1 - l_out;

        let l_out_high = 0.2126 * r_out_high + 0.7152 * g_out_high + 0.0722 * b_out_high;
        let dr_high = r_out_high - l_out_high;

        assert!(dr_high.abs() > dr_1.abs());
    }

    #[test]
    fn test_mtf_stretch_frame() {
        // `set_pixel` rather than hand-computed plane offsets: a fixture that encodes the
        // layout cannot detect a layout bug.
        let original_pixel = (0.1f32, 0.2, 0.3);
        let mut frame = Frame::zeros(32, 32, 3).unwrap();
        for y in 0..32 {
            for x in 0..32 {
                frame.set_pixel(x, y, 0, original_pixel.0);
                frame.set_pixel(x, y, 1, original_pixel.1);
                frame.set_pixel(x, y, 2, original_pixel.2);
            }
        }

        mtf_stretch_frame(&mut frame, [0.2, 0.2, 0.2]).unwrap();

        let r_out = frame.get_pixel(16, 16, 0);
        let g_out = frame.get_pixel(16, 16, 1);
        let b_out = frame.get_pixel(16, 16, 2);

        // Should be boosted
        assert!(r_out > original_pixel.0);
        assert!(g_out > original_pixel.1);
        assert!(b_out > original_pixel.2);

        // Color ratios are no longer strictly preserved by MTF because we apply it independently
        // to each channel to allow autonomous divergence control.
    }

    /// Each channel gets its own curve, so distinct midtones must produce distinct
    /// results — and the plane a midtone lands on must be the one it was written for.
    /// A constant-plane fixture swept over the whole interior, because an interleaved
    /// read lands correctly wherever `p % 3 == 0`.
    #[test]
    fn per_channel_midtones_reach_their_own_planes() {
        let (w, h) = (19usize, 13);
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        for y in 0..h {
            for x in 0..w {
                frame.set_pixel(x, y, 0, 0.25);
                frame.set_pixel(x, y, 1, 0.25);
                frame.set_pixel(x, y, 2, 0.25);
            }
        }

        let midtones = [0.2f32, 0.3, 0.4];
        mtf_stretch_frame(&mut frame, midtones).unwrap();

        for (c, &m) in midtones.iter().enumerate() {
            let want = mtf(0.25, m);
            for y in 0..h {
                for x in 0..w {
                    let got = frame.get_pixel(x, y, c);
                    assert!(
                        (got - want).abs() < 1e-3,
                        "channel {c} at ({x}, {y}) is {got}, expected ~{want} for midtone {m}"
                    );
                }
            }
        }
    }

    /// The mono arm builds and reads only one LUT; it must still be the one for
    /// `midtones[0]`.
    #[test]
    fn a_mono_frame_uses_the_first_midtone() {
        let mut frame = Frame::filled(8, 8, 1, 0.25).unwrap();
        mtf_stretch_frame(&mut frame, [0.2, 0.9, 0.9]).unwrap();
        let want = mtf(0.25, 0.2);
        for y in 0..8 {
            for x in 0..8 {
                let got = frame.get_pixel(x, y, 0);
                assert!((got - want).abs() < 1e-3, "({x}, {y}) is {got}, expected ~{want}");
            }
        }
    }

    /// A negative sample (possible after a calibration overshoot) must land at the
    /// bottom of the LUT rather than wrapping the `as usize` cast to a huge index.
    #[test]
    fn out_of_range_samples_stay_in_the_table() {
        let mut frame = Frame::filled(4, 4, 1, -0.5).unwrap();
        mtf_stretch_frame(&mut frame, [0.2, 0.2, 0.2]).unwrap();
        assert_eq!(frame.get_pixel(0, 0, 0), mtf(0.0, 0.2));

        let mut frame = Frame::filled(4, 4, 1, 5.0).unwrap();
        mtf_stretch_frame(&mut frame, [0.2, 0.2, 0.2]).unwrap();
        assert_eq!(frame.get_pixel(0, 0, 0), mtf(1.0, 0.2));
    }
}
