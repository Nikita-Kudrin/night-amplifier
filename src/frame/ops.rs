use super::Frame;
use rayon::prelude::*;

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
        self.data
            .par_iter()
            .map(|&v| (v.max(0.0).min(1.0) * 255.0 + 0.5) as u8)
            .collect()
    }
}
