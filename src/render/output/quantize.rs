//! Final f32 -> 8-bit conversion for display: black floor, ordered dither, quantize.
//! Every displayed byte crosses this boundary exactly once, in the tail of the two
//! fused streaming kernels (`server::encoding::fused`) and [`super::frame_to_rgb8`] —
//! kept as one helper because parallel 8-bit conversions have drifted by an LSB here
//! before.
//!
//! **Pedestal**: the autostretch black point (`mode - black_point_sigma * sigma`,
//! clamped at zero) puts a few percent of sky pixels at exactly 0, which an OLED
//! shows as black speckle at the eyepiece. Maps `[0,1]` to `[pedestal,1]` so nothing
//! reaches off while white stays white.
//!
//! **Dither before rounding, not after**: ordered dithering biases the *rounding
//! decision* with a sub-LSB offset, turning quantization error into a
//! high-frequency pattern the eye integrates away. Adding a pattern to an
//! already-rounded byte (the old ±8 LSB version) recovers no sub-LSB information —
//! just visible noise.

use crate::frame::sample_to_u8;

/// 8x8 ordered dither threshold matrix, values `0..=63`.
///
/// Eight rather than the conventional four because of the viewing geometry this
/// exists for: on a 70 mm / 1440 px eyepiece screen behind a 100 mm lens each
/// pixel subtends roughly 1.7 arcmin, which puts a 4x4 cell's ~7 arcmin period
/// inside what the eye resolves — the pattern reads as crosshatch instead of
/// disappearing. 8x8 halves the step between adjacent thresholds and pushes the
/// fundamental to ~14 arcmin.
#[rustfmt::skip]
const BAYER_8X8: [[u8; 8]; 8] = [
    [ 0, 32,  8, 40,  2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44,  4, 36, 14, 46,  6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [ 3, 35, 11, 43,  1, 33,  9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47,  7, 39, 13, 45,  5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// One 8-bit quantization step in normalized units.
const LSB: f32 = 1.0 / 255.0;

/// How the final f32 → u8 conversion treats the display's black floor and
/// quantization step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayOutput {
    /// Lowest normalized value an output sample may take, in `[0, 1)`.
    ///
    /// `0.0` reproduces a plain conversion. Non-zero compresses the output into
    /// `[pedestal, 1]` so no pixel reaches an OLED's off state.
    pub pedestal: f32,
    /// Apply ordered dithering before rounding to 8 bits.
    pub dither: bool,
}

impl Default for DisplayOutput {
    fn default() -> Self {
        Self {
            pedestal: 0.0,
            dither: false,
        }
    }
}

impl DisplayOutput {
    /// A conversion that neither lifts the floor nor dithers, i.e. exactly what
    /// [`sample_to_u8`] does on its own.
    pub const PLAIN: Self = Self {
        pedestal: 0.0,
        dither: false,
    };

    /// True when this transform is indistinguishable from a plain conversion,
    /// letting callers take the branch-free path.
    #[inline]
    pub fn is_plain(&self) -> bool {
        !self.dither && self.pedestal <= 0.0
    }

    /// Clamp the pedestal into the range the conversion is defined over.
    ///
    /// A pedestal at or above 1.0 would map every input to white; the ceiling
    /// keeps a usable range below it even if a caller passes nonsense.
    pub fn with_pedestal(mut self, pedestal: f32) -> Self {
        self.pedestal = pedestal.clamp(0.0, 0.5);
        self
    }

    pub fn with_dither(mut self, dither: bool) -> Self {
        self.dither = dither;
        self
    }
}

/// Sub-LSB dither offset for a pixel, in normalized units, spanning
/// `(-0.5, +0.5)` of one 8-bit step.
///
/// Indexed in **output** pixel coordinates. A pattern applied before resampling
/// would be averaged into mush by the downsample, so callers must pass the
/// coordinate of the pixel being written, not the source pixel it came from.
#[inline]
fn dither_offset(x: usize, y: usize) -> f32 {
    let cell = BAYER_8X8[y & 7][x & 7] as f32;
    ((cell + 0.5) / 64.0 - 0.5) * LSB
}

/// Convert one sample, applying the pedestal and a caller-supplied dither offset.
///
/// The input is clamped before the pedestal is applied so that a negative sample
/// — which the stretch can produce — still lands on the floor rather than below
/// it.
#[inline]
fn quantize(value: f32, pedestal: f32, dither: f32) -> u8 {
    let lifted = pedestal + value.clamp(0.0, 1.0) * (1.0 - pedestal);
    sample_to_u8(lifted + dither)
}

/// Convert one interleaved RGB f32 row to 8 bits.
///
/// `y` and the row's position are in output coordinates. All three channels of a
/// pixel share one dither cell, which is deliberate: a per-channel offset would
/// inject chroma noise into a grey sky rather than only breaking up the
/// luminance quantization.
#[inline]
pub(crate) fn write_row_rgb8(row_out: &mut [u8], row_in: &[f32], y: usize, output: DisplayOutput) {
    debug_assert_eq!(row_out.len(), row_in.len());

    if output.is_plain() {
        for (out, &v) in row_out.iter_mut().zip(row_in.iter()) {
            *out = sample_to_u8(v);
        }
        return;
    }

    let pedestal = output.pedestal;
    for (x, (out_px, in_px)) in row_out
        .chunks_exact_mut(3)
        .zip(row_in.chunks_exact(3))
        .enumerate()
    {
        let d = if output.dither {
            dither_offset(x, y)
        } else {
            0.0
        };
        out_px[0] = quantize(in_px[0], pedestal, d);
        out_px[1] = quantize(in_px[1], pedestal, d);
        out_px[2] = quantize(in_px[2], pedestal, d);
    }
}

/// Convert one interleaved RGB pixel at a known output coordinate.
///
/// For traversals that visit pixels in a flat run rather than by row — see
/// [`super::frame_to_rgb8`], whose rayon chunks are pixel counts and so cross
/// row boundaries.
#[inline]
pub(crate) fn write_pixel_rgb8(
    out_px: &mut [u8],
    r: f32,
    g: f32,
    b: f32,
    x: usize,
    y: usize,
    output: DisplayOutput,
) {
    if output.is_plain() {
        out_px[0] = sample_to_u8(r);
        out_px[1] = sample_to_u8(g);
        out_px[2] = sample_to_u8(b);
        return;
    }

    let d = if output.dither {
        dither_offset(x, y)
    } else {
        0.0
    };
    out_px[0] = quantize(r, output.pedestal, d);
    out_px[1] = quantize(g, output.pedestal, d);
    out_px[2] = quantize(b, output.pedestal, d);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicated or missing entry silently biases the dither toward one
    /// threshold, which shows up as a faint tint rather than as a crash.
    #[test]
    fn bayer_matrix_is_a_permutation_of_0_to_63() {
        let mut seen = [false; 64];
        for row in BAYER_8X8 {
            for cell in row {
                assert!(!seen[cell as usize], "value {cell} appears twice");
                seen[cell as usize] = true;
            }
        }
        assert!(seen.iter().all(|&s| s), "matrix does not cover 0..=63");
    }

    /// The offset must stay strictly inside ±half an LSB: larger and it is
    /// visible noise, smaller and it cannot flip a rounding decision.
    #[test]
    fn dither_offset_spans_just_under_half_an_lsb() {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for y in 0..8 {
            for x in 0..8 {
                let d = dither_offset(x, y);
                min = min.min(d);
                max = max.max(d);
            }
        }
        assert!(min > -0.5 * LSB, "min {min} exceeds half an LSB");
        assert!(max < 0.5 * LSB, "max {max} exceeds half an LSB");
        // The extremes should still reach most of the way there, or the dither
        // is too weak to break up a band.
        assert!(min < -0.48 * LSB && max > 0.48 * LSB);
    }

    /// The mean offset over one tile must be zero, or the dither shifts overall
    /// brightness instead of only redistributing rounding error.
    #[test]
    fn dither_offset_is_mean_zero_over_a_tile() {
        let sum: f32 = (0..8)
            .flat_map(|y| (0..8).map(move |x| dither_offset(x, y)))
            .sum();
        assert!(sum.abs() < 1e-6 * LSB, "tile mean is {sum}, expected 0");
    }

    /// The property the whole change exists for: for an input between two
    /// 8-bit levels, the local mean of the dithered output must track the input,
    /// where an undithered conversion would snap every pixel to one level.
    #[test]
    fn dithering_preserves_sub_lsb_levels_in_the_block_mean() {
        let output = DisplayOutput {
            pedestal: 0.0,
            dither: true,
        };

        for step in 0..8 {
            // Sit 'step/8' of the way between output levels 40 and 41.
            let value = (40.0 + step as f32 / 8.0) * LSB;
            let mut total = 0u32;
            for y in 0..8 {
                let row_in = vec![value; 8 * 3];
                let mut row_out = vec![0u8; 8 * 3];
                write_row_rgb8(&mut row_out, &row_in, y, output);
                total += row_out.iter().step_by(3).map(|&v| v as u32).sum::<u32>();
            }
            let mean = total as f32 / 64.0;
            let expected = 40.0 + step as f32 / 8.0;
            assert!(
                (mean - expected).abs() < 0.2,
                "block mean {mean} should track input {expected} within 0.2 LSB"
            );

            // The undithered conversion is what this improves on: it collapses
            // every one of these inputs onto a single byte.
            let mut plain = vec![0u8; 3];
            write_pixel_rgb8(&mut plain, value, value, value, 0, 0, DisplayOutput::PLAIN);
            assert_eq!(plain[0], sample_to_u8(value));
        }
    }

    /// A plain transform must be byte-identical to the canonical conversion, so
    /// enabling the feature is the only thing that can change existing output.
    #[test]
    fn plain_output_matches_the_canonical_conversion() {
        let values: Vec<f32> = (0..64).map(|i| i as f32 / 63.0).collect();
        let mut row_out = vec![0u8; values.len()];
        write_row_rgb8(&mut row_out, &values, 0, DisplayOutput::PLAIN);
        for (out, &v) in row_out.iter().zip(values.iter()) {
            assert_eq!(*out, sample_to_u8(v));
        }
    }

    /// The dark blocks this was built to remove: with a pedestal, no channel of
    /// any pixel may land on 0, whatever the input — including inputs the
    /// stretch drove negative.
    #[test]
    fn pedestal_keeps_every_sample_off_the_oled_floor() {
        let output = DisplayOutput::default()
            .with_pedestal(0.04)
            .with_dither(true);

        let row_in: Vec<f32> = (0..24)
            .map(|i| if i % 2 == 0 { 0.0 } else { -0.05 })
            .collect();
        for y in 0..8 {
            let mut row_out = vec![0u8; row_in.len()];
            write_row_rgb8(&mut row_out, &row_in, y, output);
            assert!(
                row_out.iter().all(|&v| v > 0),
                "row {y} put a sample on 0: {row_out:?}"
            );
        }
    }

    #[test]
    fn pedestal_leaves_white_at_full_scale() {
        let output = DisplayOutput::default().with_pedestal(0.04);
        let mut px = vec![0u8; 3];
        write_pixel_rgb8(&mut px, 1.0, 1.0, 1.0, 0, 0, output);
        assert_eq!(px, vec![255, 255, 255]);
    }

    #[test]
    fn pedestal_is_clamped_to_a_usable_range() {
        assert_eq!(DisplayOutput::default().with_pedestal(-1.0).pedestal, 0.0);
        assert_eq!(DisplayOutput::default().with_pedestal(9.0).pedestal, 0.5);
    }

    /// Two kernels doing one job: `AGENTS.md` asks for an equivalence test
    /// wherever a row form and a per-pixel form of the same operation coexist.
    #[test]
    fn row_and_pixel_kernels_agree() {
        for output in [
            DisplayOutput::PLAIN,
            DisplayOutput::default().with_dither(true),
            DisplayOutput::default().with_pedestal(0.04),
            DisplayOutput::default().with_pedestal(0.04).with_dither(true),
        ] {
            for y in 0..9 {
                let row_in: Vec<f32> = (0..30).map(|i| i as f32 / 29.0).collect();
                let mut row_out = vec![0u8; row_in.len()];
                write_row_rgb8(&mut row_out, &row_in, y, output);

                for x in 0..10 {
                    let mut px = vec![0u8; 3];
                    write_pixel_rgb8(
                        &mut px,
                        row_in[x * 3],
                        row_in[x * 3 + 1],
                        row_in[x * 3 + 2],
                        x,
                        y,
                        output,
                    );
                    assert_eq!(
                        &row_out[x * 3..x * 3 + 3],
                        px.as_slice(),
                        "kernels disagree at ({x}, {y}) for {output:?}"
                    );
                }
            }
        }
    }

    /// The dither must tile in output coordinates, so the same x within a row
    /// eight rows apart gets the same offset.
    #[test]
    fn dither_tiles_every_eight_pixels() {
        assert_eq!(dither_offset(0, 0), dither_offset(8, 8));
        assert_eq!(dither_offset(3, 5), dither_offset(11, 13));
        assert_ne!(dither_offset(0, 0), dither_offset(1, 0));
    }
}
