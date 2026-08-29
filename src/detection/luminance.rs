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

/// Mean of all channels per pixel.
///
/// Planar layout makes this one pass: `channels` streaming reads and one write. The
/// interleaved version had to gather with a stride, and the first planar version
/// accumulated `+=` once per channel over the whole output — `channels` read-modify-
/// write passes for a job that needs none.
///
/// `pub` because the Pro plate solver projects a colour frame down to this before
/// handing it to ASTAP, and it must use the same projection star detection used to
/// decide the frame was solvable in the first place — a Rec. 709 combine would
/// down-weight exactly the red-heavy fields the flat average exists to protect.
pub fn mean_luminance(frame: &Frame) -> Vec<f32> {
    let pixel_count = frame.pixel_count();
    let channels = frame.channels();
    let data = frame.data();

    if channels == 1 {
        return data[..pixel_count].to_vec();
    }

    let inv_channels = 1.0 / channels as f32;

    if channels == 3 {
        let (r, g, b) = frame.planes();
        return (0..pixel_count)
            .map(|i| (r[i] + g[i] + b[i]) * inv_channels)
            .collect();
    }

    (0..pixel_count)
        .map(|i| {
            let mut sum = 0.0;
            for c in 0..channels {
                sum += data[c * pixel_count + i];
            }
            sum * inv_channels
        })
        .collect()
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
