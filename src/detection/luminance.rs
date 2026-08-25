//! Channel-averaged luminance extraction for star detection.
//!
//! Split out because `detection::adaptive` and `detection::detector` carried
//! byte-identical copies of this, and both were edited identically during the planar
//! migration — which is the point at which duplication stops being harmless.
//!
//! Distinct from `planetary::quality::frame_to_luminance`, which uses Rec. 709
//! weights. Star detection wants a flat channel average so a red-heavy field is not
//! down-weighted relative to a green-heavy one.

use crate::frame::Frame;

/// Mean of all channels per pixel.
///
/// Planar layout makes this one pass: `channels` streaming reads and one write. The
/// interleaved version had to gather with a stride, and the first planar version
/// accumulated `+=` once per channel over the whole output — `channels` read-modify-
/// write passes for a job that needs none.
pub(crate) fn mean_luminance(frame: &Frame) -> Vec<f32> {
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
