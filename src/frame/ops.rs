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

    /// Converts the frame back to 8-bit output
    pub fn to_rgb8(&self) -> Vec<u8> {
        self.data
            .iter()
            .map(|&v| (v.clamp(0.0, 1.0) * 255.0).round() as u8)
            .collect()
    }

    /// Converts the frame to 8-bit output using Rayon parallelism
    pub fn to_rgb8_fast(&self) -> Vec<u8> {
        self.data.par_iter().copied().map(sample_to_u8).collect()
    }
}
