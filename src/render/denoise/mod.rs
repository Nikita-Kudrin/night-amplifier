//! Spatial denoising, run at **stream resolution** inside the encoders.
//!
//! # Why here and not in the render pipeline
//!
//! The pipeline's frame is at sensor resolution — 9 MP on an IMX533. The
//! eyepiece screen is 1440². Denoising nine megapixels and then discarding
//! three quarters of the result is 4.5x the DRAM traffic for no benefit: a
//! four-level à trous transform on a 9 MP luma plane moves roughly 576 MB per
//! frame against ~128 MB at display size. The encoders' box downsample is
//! itself an area average — a 2x noise reduction — so running after it also
//! hands the filters an easier problem than they would get at full resolution.
//!
//! That puts this stage between the downsample and the tone curve, which is
//! where `server::encoding::fused` stages a full-resolution interleaved RGB
//! buffer for it. Neither filter can fuse into the encoders' per-row closure:
//! both need neighbourhood access across rows.
//!
//! # Why linear light
//!
//! The stretch has not run yet, so sky noise is still roughly stationary across
//! the frame and a single MAD-derived threshold describes it. After the tone
//! curve the same physical noise spans wildly different amplitudes at different
//! brightnesses, and one threshold would either miss the shadows or eat the
//! highlights.
//!
//! # Why YCbCr and not RGB
//!
//! The two defects are different and want different filters. Colour mottle is
//! chroma-only and survives aggressive smoothing because the eye barely
//! resolves chroma detail; luminance grain sits on top of the structure the
//! observer came to see, so it gets a scale-selective filter that leaves stars
//! and coarse nebulosity alone. Filtering RGB directly would apply one
//! compromise to both.

mod guided;
mod wavelet;

pub use guided::ChromaDenoiseConfig;
pub use wavelet::{LumaDenoiseConfig, MAX_LEVEL1_K, MAX_STRENGTH as MAX_LUMA_STRENGTH};

/// Rec. 709 luma weights. The chroma planes are the plain differences
/// `b - y` and `r - y`, which makes the inverse exact in f32 rather than
/// merely close — this transform runs on every streamed frame and a lossy
/// round trip would show up as a slow colour drift.
const KR: f32 = 0.2126;
const KG: f32 = 0.7152;
const KB: f32 = 0.0722;

/// Both spatial denoisers, as the encoders see them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DenoiseConfig {
    /// À trous wavelet denoising of the luminance plane.
    pub luma: LumaDenoiseConfig,
    /// Guided-filter smoothing of the two chroma planes.
    pub chroma: ChromaDenoiseConfig,
}

impl Default for DenoiseConfig {
    fn default() -> Self {
        Self::OFF
    }
}

impl DenoiseConfig {
    /// Neither filter runs. The encoders take their original fused path for
    /// this, so it is byte-identical to the pre-Tier-2 output rather than
    /// merely equivalent.
    pub const OFF: Self = Self {
        luma: LumaDenoiseConfig::OFF,
        chroma: ChromaDenoiseConfig::OFF,
    };

    /// Whether anything at all would happen, which is what decides between the
    /// encoders' fused and staged traversals.
    pub fn is_enabled(&self) -> bool {
        self.luma.is_enabled() || self.chroma.is_enabled()
    }
}

/// Reusable working buffers for one denoise pass.
///
/// At 1440² a pass allocates about 75 MB — a staged interleaved RGB buffer,
/// three planar channels and the wavelet's three — all freshly zero-initialised
/// and dropped again, once per payload, once per frame. Measured on a 20-core
/// x86 box that is 13 ms of the 20 ms denoising adds to an encode: page faults,
/// not arithmetic.
///
/// Owned by the render task for the life of its thread and passed down, rather
/// than kept in a thread-local: the inline encode that primes a newly-connected
/// client runs on a pooled tokio blocking thread, and a thread-local there would
/// strand 75 MB on every thread that pool ever grows to. A caller with no
/// scratch to lend — a test, a benchmark, that same inline encode — uses
/// [`denoise_rgb_interleaved`] and pays the allocation once.
#[derive(Default)]
pub struct DenoiseScratch {
    /// Interleaved RGB at output resolution, between the resample and the tone
    /// curve. Filled by the encoder, not by this module, so it is taken out
    /// rather than borrowed.
    pub(crate) staged: Vec<f32>,
    /// Luma, Cb, Cr.
    planes: [Vec<f32>; 3],
    /// The wavelet's ping-pong pair and its separable-convolution intermediate.
    wavelet: [Vec<f32>; 3],
}

/// Grow `buf` to hold at least `len` samples and hand back exactly `len`.
///
/// Grow-only, and deliberately not re-zeroed between passes: every consumer here
/// fills what it takes before reading it, so clearing a 25 MB buffer each frame
/// would be a memset with no reader.
pub(crate) fn take(buf: &mut Vec<f32>, len: usize) -> &mut [f32] {
    if buf.len() < len {
        buf.resize(len, 0.0);
    }
    &mut buf[..len]
}

/// Denoise an interleaved RGB f32 image in place, at the resolution it will be
/// displayed at.
///
/// `buf` is `width * height * 3` samples in linear light, the staged buffer the
/// fused encoders build between their resample and their tone curve.
pub fn denoise_rgb_interleaved(
    buf: &mut [f32],
    width: usize,
    height: usize,
    config: &DenoiseConfig,
) {
    denoise_rgb_interleaved_with(buf, width, height, config, &mut DenoiseScratch::default());
}

/// [`denoise_rgb_interleaved`], reusing a caller-owned set of buffers.
pub fn denoise_rgb_interleaved_with(
    buf: &mut [f32],
    width: usize,
    height: usize,
    config: &DenoiseConfig,
    scratch: &mut DenoiseScratch,
) {
    let pixels = width * height;
    if pixels == 0 || buf.len() < pixels * 3 || !config.is_enabled() {
        return;
    }

    let _span = tracing::info_span!("denoise", width, height).entered();

    // Narrowed rather than taken whole: `merge_ycbcr` walks the interleaved
    // buffer and indexes the planes from it, so a caller-supplied slice longer
    // than the image would run off the end of them.
    let buf = &mut buf[..pixels * 3];

    // Destructured in one binding so the planes and the wavelet's buffers are
    // borrowed disjointly — `denoise_luma_with` needs the latter while `luma` is
    // still out on loan from the former.
    let DenoiseScratch {
        planes, wavelet, ..
    } = scratch;
    let [luma_buf, cb_buf, cr_buf] = planes;
    let luma = take(luma_buf, pixels);
    let cb = take(cb_buf, pixels);
    let cr = take(cr_buf, pixels);
    split_ycbcr(buf, luma, cb, cr);

    if config.chroma.is_enabled() {
        guided::denoise_chroma(luma, cb, cr, width, height, &config.chroma);
    }
    if config.luma.is_enabled() {
        wavelet::denoise_luma_with(luma, width, height, &config.luma, wavelet);
    }

    merge_ycbcr(buf, luma, cb, cr);
}

/// Interleaved RGB -> three planar channels.
fn split_ycbcr(buf: &[f32], luma: &mut [f32], cb: &mut [f32], cr: &mut [f32]) {
    use rayon::prelude::*;

    let chunk = crate::parallel::balanced_chunk_len(luma.len());
    luma.par_chunks_mut(chunk)
        .zip(cb.par_chunks_mut(chunk))
        .zip(cr.par_chunks_mut(chunk))
        .enumerate()
        .for_each(|(i, ((y_out, cb_out), cr_out))| {
            let px = &buf[i * chunk * 3..][..y_out.len() * 3];
            for (j, rgb) in px.chunks_exact(3).enumerate() {
                let y = KR * rgb[0] + KG * rgb[1] + KB * rgb[2];
                y_out[j] = y;
                cb_out[j] = rgb[2] - y;
                cr_out[j] = rgb[0] - y;
            }
        });
}

/// Three planar channels -> interleaved RGB, exactly inverting [`split_ycbcr`].
fn merge_ycbcr(buf: &mut [f32], luma: &[f32], cb: &[f32], cr: &[f32]) {
    use rayon::prelude::*;

    let chunk = crate::parallel::balanced_chunk_len(luma.len());
    buf.par_chunks_mut(chunk * 3)
        .enumerate()
        .for_each(|(i, px)| {
            let base = i * chunk;
            for (j, rgb) in px.chunks_exact_mut(3).enumerate() {
                let y = luma[base + j];
                let r = y + cr[base + j];
                let b = y + cb[base + j];
                rgb[0] = r;
                rgb[1] = (y - KR * r - KB * b) / KG;
                rgb[2] = b;
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_rgb(width: usize, height: usize) -> Vec<f32> {
        (0..width * height * 3)
            .map(|i| ((i % 97) as f32) / 97.0)
            .collect()
    }

    /// The colour transform runs on every streamed frame, so a round trip that
    /// is merely close would show up as a slow drift rather than as a failure.
    #[test]
    fn ycbcr_round_trip_is_exact_to_f32_rounding() {
        let (w, h) = (37, 29);
        let original = ramp_rgb(w, h);
        let mut buf = original.clone();

        let mut luma = vec![0.0; w * h];
        let mut cb = vec![0.0; w * h];
        let mut cr = vec![0.0; w * h];
        split_ycbcr(&buf, &mut luma, &mut cb, &mut cr);
        merge_ycbcr(&mut buf, &luma, &cb, &cr);

        for (i, (&got, &want)) in buf.iter().zip(original.iter()).enumerate() {
            assert!(
                (got - want).abs() < 1e-5,
                "sample {i}: {got} != {want} after a YCbCr round trip"
            );
        }
    }

    /// A grey frame has zero chroma, so neither filter may invent any — this is
    /// what keeps a mono sensor (replicated across RGB by the encoders) from
    /// picking up a colour cast.
    #[test]
    fn grey_input_stays_grey() {
        let (w, h) = (64, 48);
        let mut buf: Vec<f32> = (0..w * h)
            .flat_map(|i| {
                let v = 0.1 + ((i % 13) as f32) * 0.001;
                [v, v, v]
            })
            .collect();

        let config = DenoiseConfig {
            luma: LumaDenoiseConfig::default(),
            chroma: ChromaDenoiseConfig::default(),
        };
        denoise_rgb_interleaved(&mut buf, w, h, &config);

        for px in buf.chunks_exact(3) {
            assert!(
                (px[0] - px[1]).abs() < 1e-5 && (px[1] - px[2]).abs() < 1e-5,
                "denoising a grey frame produced colour: {px:?}"
            );
        }
    }

    /// The encoders pass a slice sized exactly to the image, but the guard
    /// accepts a longer one — and the merge walks the interleaved buffer rather
    /// than the planes, so a longer slice would index past them.
    #[test]
    fn a_buffer_longer_than_the_image_is_left_alone_past_the_end() {
        let (w, h) = (16, 12);
        let mut buf = ramp_rgb(w, h);
        let tail = vec![-1.0f32; 40];
        buf.extend_from_slice(&tail);

        let config = DenoiseConfig {
            luma: LumaDenoiseConfig::default(),
            chroma: ChromaDenoiseConfig::default(),
        };
        denoise_rgb_interleaved(&mut buf, w, h, &config);

        assert_eq!(&buf[w * h * 3..], &tail[..]);
    }

    #[test]
    fn disabled_config_leaves_the_buffer_untouched() {
        let (w, h) = (16, 12);
        let original = ramp_rgb(w, h);
        let mut buf = original.clone();
        denoise_rgb_interleaved(&mut buf, w, h, &DenoiseConfig::OFF);
        assert_eq!(buf, original);
    }

    /// Both filters must survive a frame smaller than their own windows: the
    /// wavelet's level-4 hole is 8 px and the guided filter subsamples by 4.
    #[test]
    fn tiny_frames_do_not_panic() {
        let config = DenoiseConfig {
            luma: LumaDenoiseConfig::default(),
            chroma: ChromaDenoiseConfig::default(),
        };
        for (w, h) in [(1, 1), (3, 1), (1, 5), (5, 5), (2, 9)] {
            let mut buf = ramp_rgb(w, h);
            denoise_rgb_interleaved(&mut buf, w, h, &config);
            assert!(buf.iter().all(|v| v.is_finite()), "{w}x{h} produced NaN");
        }
    }
}
