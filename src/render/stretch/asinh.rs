use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::render::simd::apply_luminance_preserving_simd_planar;
use rayon::prelude::*;

/// Inverse hyperbolic sine function
///
/// asinh(x) = ln(x + sqrt(x² + 1))
///
/// This is the core of the non-linear stretch. Properties:
/// - Linear near zero: asinh(x) ≈ x for small x
/// - Logarithmic for large x: asinh(x) ≈ ln(2x) for large x
/// - Preserves color ratios when applied to luminance
#[inline]
pub fn asinh(x: f32) -> f32 {
    (x + (x * x + 1.0).sqrt()).ln()
}

/// Apply asinh stretch to a single value
///
/// # Arguments
/// * `value` - Input value in [0, 1] range
/// * `stretch` - Stretch factor (higher = more aggressive)
///
/// # Returns
/// Stretched value normalized to [0, 1]
#[inline]
pub fn asinh_stretch(value: f32, stretch: f32) -> f32 {
    if stretch <= 0.0 {
        return value;
    }
    let norm = 1.0 / asinh(stretch);
    asinh(stretch * value) * norm
}

/// Apply colour-preserving Asinh stretch to an RGB pixel: boosts faint signal
/// (shadow detail) while preventing bright stars from blowing out, and critically
/// **preserves RGB channel ratios** for natural star colours. Stretches luminance
/// only (`L_out = asinh(L_in × stretch_factor) / asinh(stretch_factor)`), then
/// scales all channels by `L_out / L_in`. `r`/`g`/`b` and `stretch_factor` (typical
/// 0.1-50.0) in, `(r_out, g_out, b_out)` clamped to [0.0, 1.0] out.
#[inline]
pub fn asinh_stretch_color_preserving(
    r: f32,
    g: f32,
    b: f32,
    stretch_factor: f32,
    color_intensity: f32,
) -> (f32, f32, f32) {
    if stretch_factor <= 0.0 {
        return (r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
    }

    let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;

    if luminance <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }

    let asinh_norm = 1.0 / asinh(stretch_factor);
    let luminance_stretched = asinh(stretch_factor * luminance) * asinh_norm;
    let scale = luminance_stretched / luminance;

    let lum_stretched = luminance * scale;
    let base_add = lum_stretched * (1.0 - color_intensity);
    let channel_mul = scale * color_intensity;

    let r_out = (r * channel_mul + base_add).clamp(0.0, 1.0);
    let g_out = (g * channel_mul + base_add).clamp(0.0, 1.0);
    let b_out = (b * channel_mul + base_add).clamp(0.0, 1.0);

    (r_out, g_out, b_out)
}

/// Apply colour-preserving Asinh stretch to an entire frame in-place — the
/// recommended way to stretch astronomical images, preserving RGB ratios per pixel
/// for natural star colours. Per pixel: `L = 0.2126R + 0.7152G + 0.0722B`, `L' =
/// asinh(L × stretch) / asinh(stretch)`, `s = L'/L`, then `R' = R×s` etc.
/// `stretch_factor` typical range 1.0-20.0.
///
/// # Errors
/// `InvalidConfiguration` if the frame is not 1 or 3 channels.
pub fn asinh_stretch_frame(
    frame: &mut Frame,
    stretch_factor: f32,
    color_intensity: f32,
) -> Result<()> {
    let channels = frame.channels();
    if channels != 1 && channels != 3 {
        return Err(StackError::InvalidConfiguration(format!(
            "asinh_stretch_frame requires 1 or 3 channels, got {}",
            channels
        )));
    }

    if stretch_factor <= 0.0 {
        return Ok(());
    }

    let asinh_norm = 1.0 / asinh(stretch_factor);
    let width = frame.width();
    let data = frame.data_mut();

    if channels == 1 {
        data.par_iter_mut().for_each(|pixel| {
            let value = *pixel;
            if value <= 1e-8 {
                *pixel = 0.0;
                return;
            }
            *pixel = (asinh(stretch_factor * value) * asinh_norm).clamp(0.0, 1.0);
        });
    } else {
        let (r, g, b) = frame.planes_mut();
        apply_luminance_preserving_simd_planar(r, g, b, width, color_intensity, |l| {
            asinh(stretch_factor * l) * asinh_norm
        });
    }

    Ok(())
}

/// Estimate stretch factor to map input median to target output
pub(crate) fn estimate_stretch_factor(input_median: f32, target_output: f32) -> f32 {
    let mut low = 0.1f32;
    let mut high = 100.0f32;

    for _ in 0..20 {
        let mid = (low + high) / 2.0;
        let output = asinh_stretch(input_median, mid);

        if output < target_output {
            low = mid;
        } else {
            high = mid;
        }
    }

    (low + high) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asinh_properties() {
        assert!((asinh(0.0)).abs() < 1e-6);

        let x = 1.5;
        assert!((asinh(-x) + asinh(x)).abs() < 1e-6);

        let small = 0.01;
        assert!((asinh(small) - small).abs() < 0.001);

        assert!((asinh(1.0) - 0.8814).abs() < 0.001);
    }

    #[test]
    fn test_asinh_stretch_normalization() {
        for stretch in [0.5, 1.0, 2.0, 5.0, 10.0] {
            let result = asinh_stretch(1.0, stretch);
            assert!(
                (result - 1.0).abs() < 1e-5,
                "stretch={}, result={}",
                stretch,
                result
            );
        }

        assert!((asinh_stretch(0.0, 5.0)).abs() < 1e-6);
    }

    #[test]
    fn test_asinh_stretch_shadow_boost() {
        let dark_pixel = 0.1;

        let low_stretch = asinh_stretch(dark_pixel, 1.0);
        let high_stretch = asinh_stretch(dark_pixel, 10.0);

        assert!(
            high_stretch > low_stretch,
            "Higher stretch should boost shadows: {} vs {}",
            high_stretch,
            low_stretch
        );
    }

    #[test]
    fn test_asinh_stretch_color_preserving_preserves_ratios() {
        let r = 0.4;
        let g = 0.2;
        let b = 0.1;

        let orig_rg = r / g;
        let orig_rb = r / b;
        let orig_gb = g / b;

        let (r_out, g_out, b_out) = asinh_stretch_color_preserving(r, g, b, 10.0, 1.0);

        let new_rg = r_out / g_out;
        let new_rb = r_out / b_out;
        let new_gb = g_out / b_out;

        assert!((orig_rg - new_rg).abs() < 1e-5);
        assert!((orig_rb - new_rb).abs() < 1e-5);
        assert!((orig_gb - new_gb).abs() < 1e-5);
    }

    #[test]
    fn test_asinh_stretch_color_preserving_boosts_shadows() {
        let dark = (0.05, 0.03, 0.04);
        let bright = (0.8, 0.7, 0.6);

        let (d_r, d_g, d_b) = asinh_stretch_color_preserving(dark.0, dark.1, dark.2, 10.0, 1.0);
        let (b_r, b_g, b_b) =
            asinh_stretch_color_preserving(bright.0, bright.1, bright.2, 10.0, 1.0);

        let dark_lum_in = 0.2126 * dark.0 + 0.7152 * dark.1 + 0.0722 * dark.2;
        let dark_lum_out = 0.2126 * d_r + 0.7152 * d_g + 0.0722 * d_b;
        let bright_lum_in = 0.2126 * bright.0 + 0.7152 * bright.1 + 0.0722 * bright.2;
        let bright_lum_out = 0.2126 * b_r + 0.7152 * b_g + 0.0722 * b_b;

        let dark_boost = dark_lum_out / dark_lum_in;
        let bright_boost = bright_lum_out / bright_lum_in;

        assert!(dark_boost > bright_boost);
    }

    #[test]
    fn test_asinh_stretch_color_preserving_black_pixel() {
        let (r, g, b) = asinh_stretch_color_preserving(0.0, 0.0, 0.0, 10.0, 1.0);
        assert_eq!(r, 0.0);
        assert_eq!(g, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_asinh_stretch_color_preserving_zero_stretch() {
        let r = 0.5;
        let g = 0.3;
        let b = 0.7;

        let (r_out, g_out, b_out) = asinh_stretch_color_preserving(r, g, b, 0.0, 1.0);

        assert!((r_out - r).abs() < 1e-6);
        assert!((g_out - g).abs() < 1e-6);
        assert!((b_out - b).abs() < 1e-6);
    }

    #[test]
    fn test_asinh_stretch_color_intensity() {
        let r = 0.5;
        let g = 0.3;
        let b = 0.7;
        let stretch_factor = 10.0;

        let (r_out_1, g_out_1, b_out_1) =
            asinh_stretch_color_preserving(r, g, b, stretch_factor, 1.0);
        let (r_out_high, g_out_high, b_out_high) =
            asinh_stretch_color_preserving(r, g, b, stretch_factor, 2.0);
        let (r_out_low, g_out_low, b_out_low) =
            asinh_stretch_color_preserving(r, g, b, stretch_factor, 0.5);
        let (r_out_zero, g_out_zero, b_out_zero) =
            asinh_stretch_color_preserving(r, g, b, stretch_factor, 0.0);

        // When intensity is 0, the output should be monochromatic (luminance only)
        // Check that the three channels are very close to each other.
        assert!((r_out_zero - g_out_zero).abs() < 1e-4);
        assert!((r_out_zero - b_out_zero).abs() < 1e-4);

        // Calculate differences between channel and luminance for intensity=1
        let l_out = 0.2126 * r_out_1 + 0.7152 * g_out_1 + 0.0722 * b_out_1;
        let dr_1 = r_out_1 - l_out;
        let dg_1 = g_out_1 - l_out;
        let db_1 = b_out_1 - l_out;

        // Calculate for intensity=2
        let l_out_high = 0.2126 * r_out_high + 0.7152 * g_out_high + 0.0722 * b_out_high;
        let dr_high = r_out_high - l_out_high;
        let dg_high = g_out_high - l_out_high;
        let db_high = b_out_high - l_out_high;

        // Calculate for intensity=0.5. This case was computed but never asserted on,
        // so the monotonicity claim below only ever covered the 1.0 -> 2.0 half.
        let l_out_low = 0.2126 * r_out_low + 0.7152 * g_out_low + 0.0722 * b_out_low;
        let dr_low = r_out_low - l_out_low;
        let dg_low = g_out_low - l_out_low;
        let db_low = b_out_low - l_out_low;

        // Channel spread from luminance must increase monotonically with intensity.
        assert!(dr_low.abs() < dr_1.abs());
        assert!(dg_low.abs() < dg_1.abs());
        assert!(db_low.abs() < db_1.abs());

        assert!(dr_high.abs() > dr_1.abs());
        assert!(dg_high.abs() > dg_1.abs());
        assert!(db_high.abs() > db_1.abs());
    }

    #[test]
    fn test_asinh_stretch_frame_basic() {
        let mut data = vec![0.0f32; 32 * 32 * 3];
        let plane = 32 * 32;
        for i in 0..plane {
            data[i] = 0.3;
            data[plane + i] = 0.15;
            data[plane * 2 + i] = 0.2;
        }
        let mut frame = Frame::from_f32_vec(data, 32, 32, 3).unwrap();

        asinh_stretch_frame(&mut frame, 5.0, 1.0).unwrap();

        let r_out = frame.get_pixel(16, 16, 0);
        let g_out = frame.get_pixel(16, 16, 1);
        let b_out = frame.get_pixel(16, 16, 2);

        assert!(r_out > 0.3);
        assert!(g_out > 0.15);
        assert!(b_out > 0.2);

        let orig_rg = 0.3 / 0.15;
        let new_rg = r_out / g_out;
        assert!((orig_rg - new_rg).abs() < 1e-4);
    }

    #[test]
    fn test_asinh_stretch_frame_wrong_channels() {
        let mut frame = Frame::filled(10, 10, 2, 0.5).unwrap();
        let result = asinh_stretch_frame(&mut frame, 5.0, 1.0);
        assert!(matches!(result, Err(StackError::InvalidConfiguration(_))));
    }

    #[test]
    fn test_estimate_stretch_factor() {
        let input = 0.1;
        let target = 0.25;

        let stretch = estimate_stretch_factor(input, target);
        let actual = asinh_stretch(input, stretch);

        assert!(
            (actual - target).abs() < 0.01,
            "Stretch estimation failed: got {} expected {}",
            actual,
            target
        );
    }
}
