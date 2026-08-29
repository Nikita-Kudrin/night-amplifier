//! Final output conversion and contrast adjustment.
//!
//! This module provides the final step of the rendering pipeline:
//! converting stretched f32 frames to display-ready 8-bit RGB.

use crate::error::{Result, StackError};
use crate::frame::Frame;
use rayon::prelude::*;

mod contrast;
mod quantize;

pub use contrast::{apply_contrast_frame, apply_contrast_slice, apply_s_curve, ContrastConfig};
pub use quantize::DisplayOutput;
pub(crate) use quantize::{write_pixel_rgb8, write_row_rgb8};

/// Configuration for the final output conversion
#[derive(Debug, Clone, Copy)]
pub struct OutputConfig {
    /// Optional S-curve contrast adjustment
    pub contrast: ContrastConfig,
    /// Final gamma correction (applied after contrast)
    pub gamma: f32,
    /// Black floor and dithering applied at the 8-bit conversion.
    pub display: DisplayOutput,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            contrast: ContrastConfig::default(),
            gamma: 1.0,
            display: DisplayOutput::PLAIN,
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

    pub fn with_display(mut self, display: DisplayOutput) -> Self {
        self.display = display;
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

    // Chunked by pixel run, not by pixel. `par_chunks_mut(3)` zipped three deep made one
    // rayon item per pixel through a four-level `Zip`, so the split and index bookkeeping
    // cost about as much as the conversion: measured against `Frame::to_rgb8_fast` doing
    // identical work (contrast and gamma both off) on a 2712x1538x3 frame, 3.07 ms
    // against 1.55 ms. `Frame::gather_interleaved_into` was moved off this shape for the
    // same reason; this is the same fix applied to the same traversal.
    let pixels_per_chunk = crate::parallel::balanced_chunk_len(num_pixels);
    let width = frame.width();
    let display = config.display;
    output
        .par_chunks_mut(pixels_per_chunk * 3)
        .zip(r_plane.par_chunks(pixels_per_chunk))
        .zip(g_plane.par_chunks(pixels_per_chunk))
        .zip(b_plane.par_chunks(pixels_per_chunk))
        .enumerate()
        .for_each(|(chunk_idx, (((px_block, r_block), g_block), b_block))| {
            // Chunks are pixel runs, not rows, so the dither's output coordinate
            // has to be recovered from the absolute pixel index.
            let base = chunk_idx * pixels_per_chunk;
            for (i, out_px) in px_block.chunks_exact_mut(3).enumerate() {
                let (mut r, mut g, mut b) = (r_block[i], g_block[i], b_block[i]);

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

                // The shared conversion rather than a fourth open-coded copy of it.
                // Over the clamped, non-negative range both `(v * 255.0).round()`
                // and `v * 255.0 + 0.5` truncate to the same byte, so the plain
                // path is unchanged from before this carried a `DisplayOutput`.
                let pixel = base + i;
                write_pixel_rgb8(out_px, r, g, b, pixel % width, pixel / width, display);
            }
        });

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
            display: DisplayOutput::PLAIN,
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
        display: DisplayOutput::PLAIN,
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

    /// With contrast and gamma both off this does exactly what `Frame::to_rgb8_fast`
    /// does, so it must produce exactly the same bytes. Pins both the shared
    /// `sample_to_u8` conversion and the block traversal: an off-by-one in the chunk
    /// arithmetic shows up here as a shifted buffer.
    #[test]
    fn passthrough_config_matches_to_rgb8_fast() {
        // 271x153 is deliberately awkward: 41463 pixels is prime-ish enough that no
        // chunk length divides it, so the last block is a short one.
        let mut frame = Frame::zeros(271, 153, 3).unwrap();
        let mut seed = 0x9E37_79B9u32;
        for y in 0..153 {
            for x in 0..271 {
                for c in 0..3 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    frame.set_pixel(x, y, c, (seed >> 8) as f32 / 16_777_216.0);
                }
            }
        }

        let passthrough = OutputConfig {
            contrast: ContrastConfig::new(0.0, 0.5),
            gamma: 1.0,
            display: DisplayOutput::PLAIN,
        };
        assert_eq!(
            frame_to_rgb8(&frame, passthrough).unwrap(),
            frame.to_rgb8_fast()
        );
    }

    /// The block split must not change the answer, whatever rayon does with it.
    #[test]
    fn output_is_invariant_to_thread_count() {
        let mut frame = Frame::zeros(97, 61, 3).unwrap();
        for y in 0..61 {
            for x in 0..97 {
                for c in 0..3 {
                    frame.set_pixel(x, y, c, ((x * 7 + y * 13 + c * 29) % 251) as f32 / 250.0);
                }
            }
        }
        let config = OutputConfig::new()
            .with_contrast(ContrastConfig::moderate())
            .with_gamma(1.8);

        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| frame_to_rgb8(&frame, config).unwrap())
        };
        assert_eq!(run(1), run(8));
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
