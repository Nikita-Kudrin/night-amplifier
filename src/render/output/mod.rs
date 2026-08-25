//! Final output conversion and contrast adjustment.
//!
//! This module provides the final step of the rendering pipeline:
//! converting stretched f32 frames to display-ready 8-bit RGB.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

mod contrast;
mod dither;

pub use contrast::{apply_contrast_frame, apply_contrast_slice, apply_s_curve, ContrastConfig};
use dither::apply_ordered_dither;

/// Configuration for the final output conversion
#[derive(Debug, Clone, Copy)]
pub struct OutputConfig {
    /// Optional S-curve contrast adjustment
    pub contrast: ContrastConfig,
    /// Final gamma correction (applied after contrast)
    pub gamma: f32,
    /// Dithering to reduce banding in gradients
    pub dither: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            contrast: ContrastConfig::default(),
            gamma: 1.0,
            dither: false,
        }
    }
}

impl OutputConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_contrast(mut self, contrast: ContrastConfig) -> Self {
        self.contrast = contrast;
        self
    }

    pub fn with_gamma(mut self, gamma: f32) -> Self {
        self.gamma = gamma.clamp(0.1, 3.0);
        self
    }

    pub fn with_dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }
}

/// Convert a stretched f32 frame to 8-bit RGB buffer for display
pub fn frame_to_rgb8(frame: &Frame, config: OutputConfig) -> Result<Vec<u8>> {
    if frame.channels() != 3 {
        return Err(StackError::ChannelMismatch {
            expected: 3,
            actual: frame.channels(),
        });
    }

    // `Frame` is planar; this output is interleaved RGB8. Read the three planes in
    // lockstep rather than treating consecutive samples as one pixel.
    let (r_plane, g_plane, b_plane) = frame.planes();
    let num_pixels = frame.width() * frame.height();

    let gamma_lut: Option<[f32; 256]> = if (config.gamma - 1.0).abs() > 1e-6 {
        let inv_gamma = 1.0 / config.gamma;
        let mut lut = [0.0f32; 256];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = (i as f32 / 255.0).powf(inv_gamma);
        }
        Some(lut)
    } else {
        None
    };

    let apply_contrast = !config.contrast.is_disabled();
    let contrast = config.contrast;

    let mut output = vec![0u8; num_pixels * 3];
    output
        .par_chunks_mut(3)
        .zip(
            r_plane
                .par_iter()
                .zip(g_plane.par_iter())
                .zip(b_plane.par_iter()),
        )
        .for_each(|(out_px, ((&pr, &pg), &pb))| {
            let (mut r, mut g, mut b) = (pr, pg, pb);

            if apply_contrast {
                let luminance = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                if luminance > 1e-8 {
                    let luminance_adjusted = apply_s_curve(luminance, &contrast);
                    let scale = luminance_adjusted / luminance;
                    r = (r * scale).clamp(0.0, 1.0);
                    g = (g * scale).clamp(0.0, 1.0);
                    b = (b * scale).clamp(0.0, 1.0);
                }
            }

            if let Some(ref lut) = gamma_lut {
                let r_idx = (r * 255.0).round().clamp(0.0, 255.0) as usize;
                let g_idx = (g * 255.0).round().clamp(0.0, 255.0) as usize;
                let b_idx = (b * 255.0).round().clamp(0.0, 255.0) as usize;
                r = lut[r_idx];
                g = lut[g_idx];
                b = lut[b_idx];
            }

            out_px[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
            out_px[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
            out_px[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        });

    if config.dither {
        return Ok(apply_ordered_dither(output, frame.width(), frame.height()));
    }

    debug_assert_eq!(output.len(), num_pixels * 3);
    Ok(output)
}

#[inline]
pub fn frame_to_rgb8_simple(frame: &Frame) -> Result<Vec<u8>> {
    frame_to_rgb8(
        frame,
        OutputConfig {
            contrast: ContrastConfig::new(0.0, 0.5),
            gamma: 1.0,
            dither: false,
        },
    )
}

#[inline]
pub fn frame_to_rgb8_with_contrast(frame: &Frame) -> Result<Vec<u8>> {
    frame_to_rgb8(
        frame,
        OutputConfig::new().with_contrast(ContrastConfig::moderate()),
    )
}

pub fn finalize_for_display(
    frame: &Frame,
    contrast: Option<ContrastConfig>,
    gamma: f32,
) -> Result<Vec<u8>> {
    let config = OutputConfig {
        contrast: contrast.unwrap_or_else(|| ContrastConfig::new(0.0, 0.5)),
        gamma,
        dither: false,
    };
    frame_to_rgb8(frame, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s_curve_contrast_effect() {
        let config = ContrastConfig::moderate();
        let shadow = 0.2;
        let highlight = 0.8;

        let shadow_out = apply_s_curve(shadow, &config);
        let highlight_out = apply_s_curve(highlight, &config);

        assert!(shadow_out < shadow);
        assert!(highlight_out > highlight);
    }

    #[test]
    fn test_frame_to_rgb8_simple() {
        let mut data = vec![0.5f32; 64 * 64 * 3];
        data[0] = 0.0;
        data[64 * 64 * 3 - 1] = 1.0;
        let frame = Frame::from_f32_vec(data, 64, 64, 3).unwrap();
        let rgb8 = frame_to_rgb8_simple(&frame).unwrap();
        assert_eq!(rgb8.len(), 64 * 64 * 3);
        assert_eq!(rgb8[0], 0);
        assert_eq!(rgb8[64 * 64 * 3 - 1], 255);
    }

    #[test]
    fn test_frame_to_rgb8_gamma() {
        let data = vec![0.5f32; 16 * 16 * 3];
        let frame = Frame::from_f32_vec(data, 16, 16, 3).unwrap();

        let config_bright = OutputConfig::new().with_gamma(2.0);
        let rgb8_bright = frame_to_rgb8(&frame, config_bright).unwrap();
        let rgb8_linear = frame_to_rgb8_simple(&frame).unwrap();

        assert!(rgb8_bright[0] > rgb8_linear[0]);
    }
}
