//! Channel-averaged luminance extraction for star detection.
//!
//! Split out because `detection::adaptive` and `detection::detector` carried
//! byte-identical copies of this, and both were edited identically during the planar
//! migration — which is the point at which duplication stops being harmless.
//!
//! Distinct from `planetary::quality::frame_to_luminance`, which uses Rec. 709
//! weights. Star detection wants a flat channel average so a red-heavy field is not
//! down-weighted relative to a green-heavy one.

use crate::error::Result;
use crate::frame::Frame;
use rayon::prelude::*;

/// Mean of all channels per pixel.
///
/// Planar layout makes this one pass: `channels` streaming reads and one write. The
/// interleaved version had to gather with a stride, and the first planar version
/// accumulated `+=` once per channel over the whole output — `channels` read-modify-
/// write passes for a job that needs none.
///
/// # Why it is parallel
///
/// Every `StarDetector::detect` starts here, so it runs once per captured frame on the
/// registration path. Written as a plain `(0..pixel_count).map().collect()` it measured
/// **24.9 ms** of a 137 ms `stacking_iteration` on a 3008x3008 colour frame in
/// production traces — a single core streaming 108 MB while nineteen others idled, and
/// the largest serial section anywhere in the stacking thread.
///
/// The destination is allocated once and written through `par_chunks_mut` rather than
/// `collect`ed, so there is one allocation instead of rayon's per-task intermediates.
/// `parallel_matches_the_sequential_projection` pins the result against the form this
/// replaced, over channel counts that hit all three arms.
///
/// `pub` because the Pro plate solver projects a colour frame down to this before
/// handing it to ASTAP, and it must use the same projection star detection used to
/// decide the frame was solvable in the first place — a Rec. 709 combine would
/// down-weight exactly the red-heavy fields the flat average exists to protect.
pub fn mean_luminance(frame: &Frame) -> Vec<f32> {
    let pixel_count = frame.pixel_count();
    let channels = frame.channels();
    let data = frame.data();

    // Already contiguous: `to_vec` is a memcpy, and rayon cannot beat one.
    if channels == 1 {
        return data[..pixel_count].to_vec();
    }

    let inv_channels = 1.0 / channels as f32;
    let chunk = crate::parallel::balanced_chunk_len(pixel_count);
    let mut out = vec![0.0f32; pixel_count];

    if channels == 3 {
        let (r, g, b) = frame.planes();
        out.par_chunks_mut(chunk)
            .enumerate()
            .for_each(|(block, dest)| {
                let base = block * chunk;
                let (r, g, b) = (
                    &r[base..base + dest.len()],
                    &g[base..base + dest.len()],
                    &b[base..base + dest.len()],
                );
                for (i, slot) in dest.iter_mut().enumerate() {
                    *slot = (r[i] + g[i] + b[i]) * inv_channels;
                }
            });
        return out;
    }

    out.par_chunks_mut(chunk)
        .enumerate()
        .for_each(|(block, dest)| {
            let base = block * chunk;
            for (i, slot) in dest.iter_mut().enumerate() {
                let mut sum = 0.0;
                for c in 0..channels {
                    sum += data[c * pixel_count + base + i];
                }
                *slot = sum * inv_channels;
            }
        });
    out
}

/// A single-channel `Frame` holding [`mean_luminance`].
///
/// Returns the frame unchanged when it is already mono, so a mono camera pays nothing.
/// The Pro plate solver uses this to avoid writing a three-plane FITS cube for ASTAP:
/// astrometry needs one plane, so binning and writing three triples the work for no
/// information.
pub fn luminance_frame(frame: &Frame) -> Result<Frame> {
    if frame.channels() == 1 {
        return Ok(frame.clone());
    }
    Frame::from_f32_vec(
        mean_luminance(frame),
        frame.width(),
        frame.height(),
        1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The projection must read one pixel's own channels, not three neighbouring
    /// samples of one plane. A colour fixture with constant, distinct planes is what
    /// makes a planar/interleaved slip visible, and the whole interior is swept because
    /// an interleaved read lands correctly wherever `p % 3 == 0`.
    #[test]
    fn luminance_frame_averages_each_pixels_own_channels() {
        let (w, h) = (17usize, 11);
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        for y in 0..h {
            for x in 0..w {
                frame.set_pixel(x, y, 0, 0.10);
                frame.set_pixel(x, y, 1, 0.40);
                frame.set_pixel(x, y, 2, 0.70);
            }
        }

        let lum = luminance_frame(&frame).unwrap();
        assert_eq!(lum.channels(), 1);
        assert_eq!((lum.width(), lum.height()), (w, h));

        let want = (0.10 + 0.40 + 0.70) / 3.0;
        for y in 0..h {
            for x in 0..w {
                let got = lum.get_pixel(x, y, 0);
                assert!((got - want).abs() < 1e-6, "({x}, {y}) is {got}, expected {want}");
            }
        }
    }

    /// The sequential form [`mean_luminance`] replaced, kept as an independent oracle.
    ///
    /// Written straight against `data()` and a hand-computed plane offset — the shape
    /// the parallel version deliberately does *not* use — so a chunk-boundary or base-
    /// index slip in the rewrite shows up as a disagreement rather than as two copies of
    /// the same mistake.
    fn sequential_mean_luminance(frame: &Frame) -> Vec<f32> {
        let pixel_count = frame.pixel_count();
        let channels = frame.channels();
        let data = frame.data();
        (0..pixel_count)
            .map(|i| {
                let mut sum = 0.0;
                for c in 0..channels {
                    sum += data[c * pixel_count + i];
                }
                sum / channels as f32
            })
            .collect()
    }

    /// Sizes chosen so the chunking cannot divide evenly: `balanced_chunk_len` has a
    /// 8192 floor, so anything under that is one chunk, and the larger sizes straddle it
    /// with a short trailing chunk. Channel counts hit all three arms — the mono memcpy,
    /// the fused RGB arm, and the general ladder.
    #[test]
    fn parallel_matches_the_sequential_projection() {
        let mut seed = 0x9E37_79B9u32;
        let mut rand = move || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / 16_777_216.0
        };

        for (w, h) in [(37usize, 23usize), (211, 97), (409, 271)] {
            for channels in [1usize, 3, 4] {
                let mut frame = Frame::zeros(w, h, channels).unwrap();
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..channels {
                            frame.set_pixel(x, y, c, rand());
                        }
                    }
                }

                let got = mean_luminance(&frame);
                let want = sequential_mean_luminance(&frame);
                assert_eq!(got.len(), want.len(), "{w}x{h}x{channels}");
                for (i, (g, w_)) in got.iter().zip(want.iter()).enumerate() {
                    assert!(
                        (g - w_).abs() < 1e-6,
                        "{w}x{h}x{channels} sample {i}: {g} != {w_}"
                    );
                }
            }
        }
    }

    /// How rayon splits the work must not change the answer.
    #[test]
    fn mean_luminance_is_invariant_to_thread_count() {
        let (w, h) = (211usize, 97);
        let mut frame = Frame::zeros(w, h, 3).unwrap();
        for y in 0..h {
            for x in 0..w {
                frame.set_pixel(x, y, 0, (x * h + y) as f32 / 40_000.0);
                frame.set_pixel(x, y, 1, (y * w + x) as f32 / 40_000.0);
                frame.set_pixel(x, y, 2, (x + y) as f32 / 400.0);
            }
        }
        let run = |threads: usize| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| mean_luminance(&frame))
        };
        assert_eq!(run(1), run(8));
    }

    /// A mono frame passes straight through: same dimensions, same samples.
    #[test]
    fn luminance_frame_passes_mono_through() {
        let mut frame = Frame::zeros(9, 5, 1).unwrap();
        for y in 0..5 {
            for x in 0..9 {
                frame.set_pixel(x, y, 0, (x + y) as f32 / 20.0);
            }
        }
        let lum = luminance_frame(&frame).unwrap();
        assert_eq!(lum.channels(), 1);
        assert_eq!(lum.data(), frame.data());
    }
}
