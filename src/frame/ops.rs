use super::Frame;
use rayon::prelude::*;

/// Canonical normalised-f32 → 8-bit sample conversion.
///
/// Shared with the streaming encoder's fused downsample so the two cannot drift:
/// an equivalence test between a downsampled encode and [`Frame::to_rgb8_fast`]
/// only means anything while both round identically.
#[inline(always)]
pub(crate) fn sample_to_u8(value: f32) -> u8 {
    (value.max(0.0).min(1.0) * 255.0 + 0.5) as u8
}

impl Frame {
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
    /// Same output as [`Frame::to_rgb8_fast`] without the allocation, for callers that
    /// already hold a pooled buffer of the right size. Exists so those callers do not
    /// re-derive the planar → interleaved gather themselves: duplicating it is exactly
    /// how the PNG writer and the SER writer drifted apart from it before.
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
