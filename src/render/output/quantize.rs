//! Final f32 -> 8-bit conversion for display: black floor, dither, quantize.
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
//! **Dither before rounding, not after**: a sub-LSB offset biases the *rounding
//! decision*, turning quantization error into a high-frequency pattern the eye
//! integrates away. Adding a pattern to an already-rounded byte (the old ±8 LSB
//! version) recovers no sub-LSB information — just visible noise.
//!
//! **Blue noise, not an ordered matrix**: an 8x8 Bayer matrix quantises a smooth sky
//! into a lattice — one dot per tile near a level, a crosshatch between — and the
//! denoised sky is smooth enough to show it. See [`dither_offset`] for the numbers.

use crate::frame::sample_to_u8;

use super::blue_noise::BLUE_NOISE_64;

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
    /// Apply blue-noise dithering before rounding to 8 bits.
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
///
/// The 64x64 void-and-cluster mask replaced the 8x8 Bayer matrix on 2026-09-24. On a
/// flat sky with 0.3 output levels of noise the matrix left lattice lines at 31-41x
/// the spectrum beside them below half Nyquist; the mask leaves 1.0-1.3x, as no dither
/// does (`dither_tests`). With the Pro denoisers on, Orion's rendered sky read 36x
/// against the mask's 5.1x and no dither's 5.8x. The mask also holds 0.16 % of its
/// energy below half Nyquist against the matrix's 1.45 %. An earlier rejection of blue
/// noise measured an 8x8 blue tile, too small to be blue.
#[inline]
fn dither_offset(x: usize, y: usize) -> f32 {
    let rank = BLUE_NOISE_64[y & 63][x & 63] as f32;
    ((rank + 0.5) / 256.0 - 0.5) * LSB
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
/// pixel share one dither threshold, which is deliberate: a per-channel offset would
/// inject chroma noise into a grey sky rather than only breaking up the
/// luminance quantization.
#[inline]
pub fn write_row_rgb8(row_out: &mut [u8], row_in: &[f32], y: usize, output: DisplayOutput) {
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

    /// The offset must stay strictly inside ±half an LSB: larger and it is
    /// visible noise, smaller and it cannot flip a rounding decision.
    #[test]
    fn dither_offset_spans_just_under_half_an_lsb() {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for y in 0..64 {
            for x in 0..64 {
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
        let sum: f64 = (0..64)
            .flat_map(|y| (0..64).map(move |x| dither_offset(x, y) as f64))
            .sum();
        assert!(sum.abs() < 1e-6 * LSB as f64, "tile sum is {sum}, expected 0");
    }

    /// The property the dither exists for: for an input between two 8-bit levels, the
    /// local mean of the output must track the input, where an undithered conversion
    /// snaps every pixel to one level.
    ///
    /// Every 8x8 block, not only the whole tile: the eye averages locally. The mask's
    /// worst block misses by 0.062 LSB; thresholds drawn as white noise miss by 0.19,
    /// which is the clumping blue noise is built to avoid.
    #[test]
    fn dithering_preserves_sub_lsb_levels_in_every_block_mean() {
        let output = DisplayOutput {
            pedestal: 0.0,
            dither: true,
        };

        for step in 0..64 {
            let expected = 40.0 + step as f32 / 64.0;
            let value = expected * LSB;
            let mut tile = vec![0u8; 64 * 64];
            for y in 0..64 {
                let mut row_out = vec![0u8; 64 * 3];
                write_row_rgb8(&mut row_out, &vec![value; 64 * 3], y, output);
                for (x, px) in row_out.chunks_exact(3).enumerate() {
                    tile[y * 64 + x] = px[0];
                }
            }
            let mean = |x0: usize, y0: usize, side: usize| {
                let sum: u32 = (y0..y0 + side)
                    .flat_map(|y| (x0..x0 + side).map(move |x| (x, y)))
                    .map(|(x, y)| tile[y * 64 + x] as u32)
                    .sum();
                sum as f32 / (side * side) as f32
            };
            assert!(
                (mean(0, 0, 64) - expected).abs() < 0.01,
                "tile mean {} should track input {expected}",
                mean(0, 0, 64)
            );
            for (bx, by) in (0..8).flat_map(|by| (0..8).map(move |bx| (bx * 8, by * 8))) {
                let block = mean(bx, by, 8);
                assert!(
                    (block - expected).abs() < 0.1,
                    "8x8 block at ({bx}, {by}) averages {block} for input {expected}"
                );
            }

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

    /// Share of a pattern's energy below `cycles` cycles per `side`-pixel tile, by
    /// direct DFT.
    fn low_frequency_share(pattern: &dyn Fn(usize, usize) -> f32, side: usize, cycles: usize) -> f64 {
        let twiddle: Vec<(f64, f64)> = (0..side)
            .map(|k| {
                let phase = -2.0 * std::f64::consts::PI * k as f64 / side as f64;
                (phase.cos(), phase.sin())
            })
            .collect();
        let values: Vec<f64> = (0..side * side).map(|i| pattern(i % side, i / side) as f64).collect();
        let mut low = 0.0f64;
        let mut total = 0.0f64;
        for u in 0..side {
            for v in 0..side {
                if u == 0 && v == 0 {
                    continue; // the mean, which the dither holds at zero
                }
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for y in 0..side {
                    for x in 0..side {
                        let (c, s) = twiddle[(u * x + v * y) % side];
                        re += values[y * side + x] * c;
                        im += values[y * side + x] * s;
                    }
                }
                let power = re * re + im * im;
                total += power;
                // Frequencies are periodic in `side`, so fold the upper half down.
                let fu = u.min(side - u);
                let fv = v.min(side - v);
                if fu * fu + fv * fv <= cycles * cycles {
                    low += power;
                }
            }
        }
        low / total
    }

    /// The 8x8 Bayer matrix the mask replaced, from its bit-interleave definition, as
    /// the reference a dither has to beat.
    fn bayer_offset(x: usize, y: usize) -> f32 {
        let mut rank = 0;
        for bit in 0..3 {
            let (bx, by) = ((x >> bit) & 1, (y >> bit) & 1);
            rank |= (((bx ^ by) << 1) | by) << (4 - 2 * bit);
        }
        (rank as f32 + 0.5) / 64.0 - 0.5
    }

    /// The dither's energy stays out of the band the eye resolves best. The mask holds
    /// 0.16 % of it below half Nyquist, the Bayer matrix 1.45 %: the matrix parks most
    /// of its energy at the (½, ½) corner, but its period-4 and period-8 levels are lines
    /// below half Nyquist, and an 8x8 *blue* tile — measured 2026-09-17 — is too small
    /// to push energy up at all (2.4 %).
    #[test]
    fn the_dither_keeps_its_energy_near_nyquist() {
        const SIDE: usize = 64;
        let share = low_frequency_share(&|x, y| dither_offset(x, y) / LSB, SIDE, SIDE / 4);
        let bayer = low_frequency_share(&bayer_offset, SIDE, SIDE / 4);
        assert!(
            share < 0.004 && share < bayer / 4.0,
            "the dither must stay out of the band the eye is sharpest in: {share:.4} of \
             its energy sits below half Nyquist, against the Bayer matrix's {bayer:.4}"
        );
    }

    /// The dither must tile in output coordinates, every 64 pixels.
    #[test]
    fn dither_tiles_every_sixty_four_pixels() {
        assert_eq!(dither_offset(0, 0), dither_offset(64, 64));
        assert_eq!(dither_offset(3, 5), dither_offset(67, 133));
        assert_ne!(dither_offset(0, 0), dither_offset(1, 0));
    }
}
