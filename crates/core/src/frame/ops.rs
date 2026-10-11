use super::Frame;
use rayon::prelude::*;

/// Canonical normalised-f32 → 8-bit sample conversion.
///
/// Shared with the streaming encoder's fused downsample so the two cannot drift:
/// an equivalence test between a downsampled encode and [`Frame::to_rgb8_fast`]
/// only means anything while both round identically.
#[inline(always)]
pub fn sample_to_u8(value: f32) -> u8 {
    (value.max(0.0).min(1.0) * 255.0 + 0.5) as u8
}

/// Canonical normalised-f32 → 16-bit sample conversion. Truncates, unlike
/// [`sample_to_u8`]: the FITS and SER 16-bit writers have always truncated, and they
/// must agree with each other.
#[inline(always)]
pub fn sample_to_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65535.0) as u16
}

/// Rec.709 weights: every luminance-preserving kernel and luminance measurement uses them.
pub const LUMA_REC709: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Rec.601 weights, only for consumers built on them (the simulator's mono sensor, and
/// Pro's AI denoiser, whose training mask was OpenCV's grey).
pub const LUMA_REC601: [f32; 3] = [0.299, 0.587, 0.114];

/// Rec.709 luminance. Summed left to right, as every kernel did before sharing this.
#[inline(always)]
pub fn luminance(r: f32, g: f32, b: f32) -> f32 {
    LUMA_REC709[0] * r + LUMA_REC709[1] * g + LUMA_REC709[2] * b
}

impl Frame {
    /// Keeps only `rect`, in place: each plane's rows move forward and the buffer is truncated,
    /// so nothing is allocated. A destination never lies past its source, which is what makes
    /// the forward walk safe.
    pub fn crop_in_place(&mut self, rect: &crate::stacking::CropRect) -> crate::error::Result<()> {
        rect.check(self.width, self.height)?;
        if rect.is_full(self.width, self.height) {
            return Ok(());
        }
        let (w, area) = (self.width, self.width * self.height);
        let (cw, ch) = (rect.width, rect.height);
        for plane in 0..self.channels {
            for row in 0..ch {
                let src = plane * area + (rect.y + row) * w + rect.x;
                self.data.copy_within(src..src + cw, plane * cw * ch + row * cw);
            }
        }
        self.data.truncate(cw * ch * self.channels);
        self.width = cw;
        self.height = ch;
        Ok(())
    }

    /// A copy of `rect` alone, allocating only its size: the cut for a frame shared with
    /// another holder, which [`Self::crop_in_place`] would have to copy whole first.
    pub fn cropped(&self, rect: &crate::stacking::CropRect) -> crate::error::Result<Self> {
        rect.check(self.width, self.height)?;
        let area = self.width * self.height;
        let mut data = Vec::with_capacity(rect.area() * self.channels);
        for plane in self.data.chunks_exact(area) {
            for row in rect.y..rect.y + rect.height {
                let start = row * self.width + rect.x;
                data.extend_from_slice(&plane[start..start + rect.width]);
            }
        }
        Self::from_f32_vec(data, rect.width, rect.height, self.channels)
    }

    /// Clamps all pixel values to the range [0.0, 1.0]
    pub fn clamp(&mut self) {
        for v in &mut self.data {
            *v = v.clamp(0.0, 1.0);
        }
    }

    /// Converts the frame back to 8-bit output.
    ///
    /// Deprecated in favour of [`Frame::to_rgb8_fast`], which is parallel and shares
    /// the canonical [`sample_to_u8`] rounding. Kept because it rounds with
    /// `round()` rather than `+0.5`, which some callers may depend on.
    pub fn to_rgb8(&self) -> Vec<u8> {
        self.gather_interleaved(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    /// Converts the frame to 8-bit output using Rayon parallelism.
    ///
    /// `Frame` is planar; this output is **interleaved** for 3-channel frames, which
    /// is what every 8-bit consumer (JPEG, LZ4, PNG, TIFF) expects. Mono frames stay
    /// one byte per pixel — callers tag those as greyscale rather than replicating.
    pub fn to_rgb8_fast(&self) -> Vec<u8> {
        self.gather_interleaved(sample_to_u8)
    }

    /// Writes the interleaved 8-bit conversion into a caller-owned buffer.
    ///
    /// Same output as [`Frame::to_rgb8_fast`] without the allocation, for callers that already
    /// hold a pooled buffer of the right size — exists so they don't re-derive the planar →
    /// interleaved gather themselves, which is exactly how the PNG and SER writers drifted apart from it before.
    ///
    /// # Panics
    /// Panics unless `out.len() == self.sample_count()`.
    pub fn write_rgb8_into(&self, out: &mut [u8]) {
        self.gather_interleaved_into(out, sample_to_u8);
    }

    /// Shared planar → interleaved gather for the 8-bit conversions.
    ///
    /// Single channel is already contiguous, so it streams; 3 channels read three
    /// planes in lockstep and write one interleaved run.
    fn gather_interleaved(&self, convert: impl Fn(f32) -> u8 + Send + Sync) -> Vec<u8> {
        let mut out = vec![0u8; self.data.len()];
        self.gather_interleaved_into(&mut out, convert);
        out
    }

    fn gather_interleaved_into(&self, out: &mut [u8], convert: impl Fn(f32) -> u8 + Send + Sync) {
        assert_eq!(
            out.len(),
            self.data.len(),
            "gather_interleaved_into: destination must hold one byte per sample"
        );

        if self.channels == 1 {
            out.par_iter_mut()
                .zip(self.data.par_iter())
                .for_each(|(slot, &v)| *slot = convert(v));
            return;
        }

        if self.channels == 3 {
            // Chunked by pixel run, not by pixel: `par_chunks_mut(3)` zipped three deep
            // makes one rayon item per pixel through a four-level `Zip`, so the split and
            // index bookkeeping cost as much as the conversion. `balanced_chunk_len` gives
            // a few runs per worker, and the inner loop is a plain scalar walk.
            let (r, g, b) = self.planes();
            let pixels_per_chunk = crate::parallel::balanced_chunk_len(r.len());

            out.par_chunks_mut(pixels_per_chunk * 3)
                .zip(r.par_chunks(pixels_per_chunk))
                .zip(g.par_chunks(pixels_per_chunk))
                .zip(b.par_chunks(pixels_per_chunk))
                .for_each(|(((px_block, r_block), g_block), b_block)| {
                    for (i, px) in px_block.chunks_exact_mut(3).enumerate() {
                        px[0] = convert(r_block[i]);
                        px[1] = convert(g_block[i]);
                        px[2] = convert(b_block[i]);
                    }
                });
            return;
        }

        // Uncommon channel counts: interleave generically rather than silently
        // emitting plane-major bytes.
        let area = self.width * self.height;
        let channels = self.channels;
        out.par_chunks_mut(channels)
            .enumerate()
            .for_each(|(i, px)| {
                for (c, slot) in px.iter_mut().enumerate() {
                    *slot = convert(self.data[c * area + i]);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::{luminance, sample_to_u16, LUMA_REC601, LUMA_REC709};
    use crate::frame::Frame;
    use crate::stacking::CropRect;

    /// The one home of each weight set, so a typo or a swapped R/B (BGR order) no longer
    /// meets ~45 independent copies that disagree with it. ITU-R BT.709 and BT.601 values.
    #[test]
    fn luma_weights_are_the_published_ones_in_rgb_order() {
        assert_eq!(LUMA_REC709, [0.2126, 0.7152, 0.0722]);
        assert_eq!(LUMA_REC601, [0.299, 0.587, 0.114]);
    }

    #[test]
    fn both_luma_weight_sets_keep_grey_grey() {
        for weights in [LUMA_REC709, LUMA_REC601] {
            assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-6, "{weights:?}");
        }
        assert!((luminance(0.4, 0.4, 0.4) - 0.4).abs() < 1e-6);
        assert_eq!(luminance(1.0, 0.0, 0.0), LUMA_REC709[0]);
        assert_eq!(luminance(0.0, 0.0, 1.0), LUMA_REC709[2]);
    }

    #[test]
    fn sample_to_u16_clamps_and_truncates() {
        assert_eq!(sample_to_u16(-0.5), 0);
        assert_eq!(sample_to_u16(f32::NAN), 0);
        assert_eq!(sample_to_u16(1.0), u16::MAX);
        assert_eq!(sample_to_u16(7.0), u16::MAX);
        // Just under the 2nd level truncates to the 1st, where rounding would reach the 2nd.
        assert_eq!(sample_to_u16(1.9 / 65535.0), 1);
    }

    /// Every sample names its own position, so a crop that crossed planes or rows shows it.
    fn labelled(width: usize, height: usize, channels: usize) -> Frame {
        let mut frame = Frame::zeros(width, height, channels).unwrap();
        for c in 0..channels {
            for y in 0..height {
                for x in 0..width {
                    frame.set_pixel(x, y, c, (c * 1_000_000 + y * 1000 + x) as f32);
                }
            }
        }
        frame
    }

    #[test]
    fn a_crop_keeps_each_planes_own_samples() {
        for channels in [1, 3] {
            let mut frame = labelled(37, 29, channels);
            let capacity = frame.data.capacity();
            let rect = CropRect { x: 8, y: 5, width: 21, height: 17 };
            frame.crop_in_place(&rect).unwrap();

            assert_eq!((frame.width(), frame.height(), frame.channels()), (21, 17, channels));
            assert_eq!(frame.sample_count(), 21 * 17 * channels);
            assert_eq!(frame.data.capacity(), capacity, "the crop reallocated");
            for c in 0..channels {
                for y in 0..17 {
                    for x in 0..21 {
                        let want = (c * 1_000_000 + (y + 5) * 1000 + x + 8) as f32;
                        assert_eq!(frame.get_pixel(x, y, c), want, "channel {c} at ({x}, {y})");
                    }
                }
            }
        }
    }

    /// The copying cut and the binning cut both equal the in-place crop they stand in for,
    /// bin and all, at every edge of the frame and for factors the cell grid does not divide.
    #[test]
    fn a_cut_from_a_shared_frame_equals_the_in_place_crop() {
        for channels in [1, 3] {
            let whole = labelled(37, 29, channels);
            for rect in [
                CropRect { x: 8, y: 5, width: 21, height: 17 },
                CropRect { x: 0, y: 0, width: 37, height: 29 },
                CropRect { x: 16, y: 8, width: 21, height: 21 },
            ] {
                let mut in_place = whole.clone();
                in_place.crop_in_place(&rect).unwrap();
                let cut = whole.cropped(&rect).unwrap();
                assert_eq!((cut.width(), cut.height()), (rect.width, rect.height));
                assert!(cut.data() == in_place.data(), "{channels} channels, {rect:?}: cropped");
                for factor in [1, 2, 3] {
                    let fused = whole.downsample_region(&rect, factor).unwrap();
                    let reference = in_place.downsample(factor).unwrap();
                    assert_eq!((fused.width(), fused.height()), (reference.width(), reference.height()));
                    assert!(fused.data() == reference.data(), "{channels} channels, {rect:?}, factor {factor}");
                }
            }
        }
    }

    #[test]
    fn a_cut_outside_the_frame_or_by_zero_is_refused() {
        let frame = labelled(16, 12, 3);
        let outside = CropRect { x: 8, y: 0, width: 9, height: 4 };
        assert!(frame.cropped(&outside).is_err());
        assert!(frame.downsample_region(&outside, 2).is_err());
        assert!(frame.downsample_region(&CropRect { x: 0, y: 0, width: 16, height: 12 }, 0).is_err());
        assert!(frame.downsample_region(&CropRect { x: 0, y: 0, width: 4, height: 4 }, 5).is_err());
    }

    #[test]
    fn a_full_frame_crop_changes_nothing() {
        let mut frame = labelled(16, 12, 3);
        let before = frame.data().to_vec();
        frame.crop_in_place(&CropRect { x: 0, y: 0, width: 16, height: 12 }).unwrap();
        assert_eq!(frame.data(), &before[..]);
    }

    #[test]
    fn a_crop_outside_the_frame_is_refused_and_leaves_it_whole() {
        let mut frame = labelled(16, 12, 1);
        for rect in [
            CropRect { x: 8, y: 0, width: 9, height: 4 },
            CropRect { x: 0, y: 0, width: 0, height: 4 },
        ] {
            assert!(frame.crop_in_place(&rect).is_err(), "{rect:?} accepted");
        }
        assert_eq!((frame.width(), frame.height()), (16, 12));
    }
}
