use super::format::PixelFormat;
use super::Frame;
use crate::debayer::{CfaPattern, DebayerConfig, Debayerer};
use crate::error::{Result, StackError};
use rayon::prelude::*;
use tracing::instrument;

/// Writes `sample(i, c)` into plane-major `data`, one contiguous run per channel. The
/// three `PixelFormat` arms differ only in how a sample decodes, so they share this
/// traversal instead of repeating the channel ladder three times.
///
/// One pass over the source, not one per plane: walking each output plane in turn
/// (`raw[i * channels + c]`) reads the whole source once per channel — three passes
/// over 12.5MB for a 2712x1538 RGB frame, on the per-frame camera ingest path, where
/// interleaved did one. Instead the source is walked once and each sample scattered to
/// its plane, with planes split apart up front so the write is a sequential store
/// stream per channel, not a bounds-checked `data[c * area + i]` per sample.
#[inline]
fn scatter_to_planes(
    data: &mut [f32],
    pixels: usize,
    channels: usize,
    sample: impl Fn(usize, usize) -> f32 + Sync,
) {
    // Why only the multi-channel arm is parallel: `frame_ingest_benchmark` shows these
    // are different kinds of work. Mono is one streaming read + write with a multiply,
    // and a single core already saturates it (4.2M samples in ~1.1ms at 2712x1538,
    // ~22GB/s) — rayon measured **18-21% slower** there, dispatch overhead against no
    // spare bandwidth. Interleaved scatters each pixel to three planes, genuinely
    // compute-bound: parallelising took `from_raw_rgb8` from 14.2ms to 4.8ms (-66%).
    //
    // Left unresolved on purpose: a Pi 5/RK3588 core doesn't saturate memory
    // bandwidth, so mono may split profitably there — an ARM measurement
    // (`from_raw_bayer16_mono`) would settle it, not a guess shipped untested.
    if channels == 1 {
        for (i, slot) in data[..pixels].iter_mut().enumerate() {
            *slot = sample(i, 0);
        }
        return;
    }

    if channels == 3 {
        // The chunk index recovers the absolute sample index, which is what keeps
        // `sample` — the per-format decode closure — unchanged and pure.
        // `balanced_chunk_len` imposes no divisibility constraint, so a short trailing
        // chunk is normal and correct here.
        let chunk = crate::parallel::balanced_chunk_len(pixels);
        let (r, rest) = data.split_at_mut(pixels);
        let (g, b) = rest.split_at_mut(pixels);
        // Zipped rather than dispatched per plane: the three planes read the *same*
        // interleaved source pixel, so keeping them in one task means each cache line of
        // `raw` is fetched once instead of three times.
        r.par_chunks_mut(chunk)
            .zip(g.par_chunks_mut(chunk))
            .zip(b.par_chunks_mut(chunk))
            .enumerate()
            .for_each(|(block, ((r_block, g_block), b_block))| {
                let base = block * chunk;
                for i in 0..r_block.len() {
                    r_block[i] = sample(base + i, 0);
                    g_block[i] = sample(base + i, 1);
                    b_block[i] = sample(base + i, 2);
                }
            });
        return;
    }

    // Uncommon channel counts: still one pass over the source, collecting the plane
    // slices first so the inner write does not re-derive an offset per sample.
    let mut planes: Vec<&mut [f32]> = Vec::with_capacity(channels);
    let mut rest = &mut data[..pixels * channels];
    for _ in 0..channels {
        let (plane, tail) = rest.split_at_mut(pixels);
        planes.push(plane);
        rest = tail;
    }
    for i in 0..pixels {
        for (c, plane) in planes.iter_mut().enumerate() {
            plane[i] = sample(i, c);
        }
    }
}

impl Frame {
    /// Creates a new Frame from raw 8-bit or 16-bit image data
    #[instrument(skip(raw), fields(format = ?format, resolution = %format!("{}x{}x{}", width, height, channels), buffer_size = raw.len()))]
    pub fn from_raw(
        raw: &[u8],
        width: usize,
        height: usize,
        channels: usize,
        format: PixelFormat,
    ) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        let pixel_count = width * height * channels;
        let expected_bytes = pixel_count * format.bytes_per_channel();

        if raw.len() != expected_bytes {
            return Err(StackError::BufferSizeMismatch {
                expected: expected_bytes,
                actual: raw.len(),
            });
        }

        let mut data = vec![0.0f32; pixel_count];
        let max_value = format.max_value();
        let inv_max = 1.0 / max_value;

        // Split into plane slices once, so each channel is a sequential store stream.
        // Indexing `data[c * area + i]` per sample made this three bounds-checked
        // scatter writes per pixel on every incoming camera frame.
        match format {
            PixelFormat::Rgb8 | PixelFormat::Bayer8 => {
                scatter_to_planes(&mut data, raw.len() / channels, channels, |i, c| {
                    raw[i * channels + c] as f32 * inv_max
                });
            }
            PixelFormat::Rgb16 | PixelFormat::Bayer16 => {
                let u16_raw = raw.as_chunks::<2>().0;
                scatter_to_planes(&mut data, u16_raw.len() / channels, channels, |i, c| {
                    let s = u16_raw[i * channels + c];
                    u16::from_le_bytes([s[0], s[1]]) as f32 * inv_max
                });
            }
            PixelFormat::Rgb16Be | PixelFormat::Bayer16Be => {
                let u16_raw = raw.as_chunks::<2>().0;
                scatter_to_planes(&mut data, u16_raw.len() / channels, channels, |i, c| {
                    let s = u16_raw[i * channels + c];
                    u16::from_be_bytes([s[0], s[1]]) as f32 * inv_max
                });
            }
        }

        Ok(Self {
            data,
            width,
            height,
            channels,
        })
    }

    /// Creates a new RGB Frame from raw Bayer pattern data with debayering
    #[instrument(skip(raw), fields(format = ?format, pattern = ?pattern, resolution = %format!("{}x{}", width, height)))]
    pub fn from_bayer(
        raw: &[u8],
        width: usize,
        height: usize,
        format: PixelFormat,
        pattern: CfaPattern,
    ) -> Result<Self> {
        if !format.is_bayer() {
            return Err(StackError::InvalidConfiguration(
                "from_bayer requires a Bayer pixel format".to_string(),
            ));
        }

        let mono_frame = Self::from_raw(raw, width, height, 1, format)?;
        let debayerer = Debayerer::new(DebayerConfig::new(pattern));
        debayerer.debayer(&mono_frame)
    }

    /// Creates a new Frame filled with zeros (black frame)
    pub fn zeros(width: usize, height: usize, channels: usize) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data: vec![0.0; width * height * channels],
            width,
            height,
            channels,
        })
    }

    /// Creates a new Frame filled with a constant value
    pub fn filled(width: usize, height: usize, channels: usize, value: f32) -> Result<Self> {
        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data: vec![value; width * height * channels],
            width,
            height,
            channels,
        })
    }

    /// Creates a Frame from existing f32 data
    pub fn from_f32_vec(
        data: Vec<f32>,
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<Self> {
        let expected = width * height * channels;
        if data.len() != expected {
            return Err(StackError::BufferSizeMismatch {
                expected,
                actual: data.len(),
            });
        }

        if width == 0 || height == 0 || channels == 0 {
            return Err(StackError::InvalidDimensions {
                width,
                height,
                channels,
            });
        }

        Ok(Self {
            data,
            width,
            height,
            channels,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain interleaved-to-planar projection: `data[c * pixels + i]` from
    /// `raw[i * channels + c]`, with no chunking and no parallelism.
    ///
    /// This is the shape [`scatter_to_planes`] replaced. It exists so the rewrite is
    /// pinned against something other than itself — the RGB arm recovers the absolute
    /// sample index from the chunk index (`block * chunk`), and that arithmetic is
    /// correct for every full chunk whether or not it is right for the trailing one.
    /// The decode itself is mirrored rather than rewritten — `* inv_max` and `/ max`
    /// differ by an ULP, and the traversal is what is under test here, not the scaling.
    fn sequential_planes(raw: &[u8], pixels: usize, channels: usize, max: f32) -> Vec<f32> {
        let inv_max = 1.0 / max;
        let mut out = vec![0.0f32; pixels * channels];
        for i in 0..pixels {
            for c in 0..channels {
                out[c * pixels + i] = raw[i * channels + c] as f32 * inv_max;
            }
        }
        out
    }

    fn interleaved_bytes(pixels: usize, channels: usize) -> Vec<u8> {
        let mut seed = 0x9E37_79B9u32;
        (0..pixels * channels)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 24) as u8
            })
            .collect()
    }

    /// Every sample, not a spot check.
    ///
    /// A constant or single-pixel assertion passes against a fully scrambled buffer —
    /// an interleaved write lands sample `c * pixels + i` exactly where the planar read
    /// expects it whenever `i % channels == 0` — so the whole projection is swept.
    #[test]
    fn scatter_matches_the_sequential_projection_across_chunk_boundaries() {
        // `balanced_chunk_len` has an 8192 floor, so these straddle it deliberately:
        // 181x137 = 24 797 samples is three full chunks plus a short trailing 221, and
        // 37x23 stays under the floor as the single-chunk case. Channel counts hit all
        // three arms — the sequential mono arm, the fused RGB arm, and the general
        // ladder.
        for (w, h) in [(37usize, 23usize), (181, 137), (409, 271)] {
            for channels in [1usize, 3, 4] {
                let pixels = w * h;
                let raw = interleaved_bytes(pixels, channels);

                let frame =
                    Frame::from_raw(&raw, w, h, channels, PixelFormat::Rgb8).unwrap();
                let want = sequential_planes(&raw, pixels, channels, 255.0);

                assert_eq!(frame.data().len(), want.len(), "{w}x{h}x{channels}");
                for (i, (got, expected)) in frame.data().iter().zip(want.iter()).enumerate() {
                    assert_eq!(
                        got, expected,
                        "{w}x{h}x{channels} sample {i} (plane {}, pixel {})",
                        i / pixels,
                        i % pixels
                    );
                }
            }
        }
    }

    /// The 16-bit arms decode through the same traversal, so the chunk arithmetic has to
    /// hold for a source whose stride is two bytes per sample as well.
    #[test]
    fn the_16_bit_arms_scatter_the_same_way() {
        let (w, h, channels) = (181usize, 137usize, 3usize);
        let pixels = w * h;
        let mut seed = 0x5A5A_1234u32;
        let samples: Vec<u16> = (0..pixels * channels)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 16) as u16
            })
            .collect();

        for (format, to_bytes) in [
            (PixelFormat::Rgb16, u16::to_le_bytes as fn(u16) -> [u8; 2]),
            (PixelFormat::Rgb16Be, u16::to_be_bytes as fn(u16) -> [u8; 2]),
        ] {
            let raw: Vec<u8> = samples.iter().flat_map(|&s| to_bytes(s)).collect();
            let frame = Frame::from_raw(&raw, w, h, channels, format).unwrap();

            let inv_max = 1.0 / format.max_value();
            for i in 0..pixels {
                for c in 0..channels {
                    let expected = samples[i * channels + c] as f32 * inv_max;
                    assert_eq!(
                        frame.data()[c * pixels + i],
                        expected,
                        "{format:?} plane {c} pixel {i}"
                    );
                }
            }
        }
    }

    /// How rayon splits the scatter must not change where a sample lands.
    #[test]
    fn scatter_is_invariant_to_thread_count() {
        let (w, h, channels) = (181usize, 137usize, 3usize);
        let raw = interleaved_bytes(w * h, channels);
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    Frame::from_raw(&raw, w, h, channels, PixelFormat::Rgb8)
                        .unwrap()
                        .data()
                        .to_vec()
                })
        };
        assert_eq!(run(1), run(8));
        assert_eq!(run(1), run(3));
    }
}
