//! A sub held back at half precision while the stack decides what to keep.
//!
//! Half precision, not f32: a probation holds several 3008²x3 subs beside the 434 MB
//! accumulator, 54 MB each instead of 108. Its 11-bit significand puts the rounding at
//! 1/2048 of a sample's value — 1.4e-6 on a 0.0028 sky whose noise is 5e-4 — so a sub
//! restacked from here differs from the one first stacked well below its own noise.

use half::f16;
use half::slice::HalfFloatSliceExt;
use rayon::prelude::*;

use crate::error::{Result, StackError};
use crate::frame::Frame;

/// Samples converted per rayon task: large enough that the split costs nothing.
const CONVERT_CHUNK: usize = 1 << 16;

pub struct HeldFrame {
    width: usize,
    height: usize,
    channels: usize,
    data: Vec<f16>,
}

impl HeldFrame {
    /// What one held sample costs.
    pub const BYTES_PER_SAMPLE: usize = std::mem::size_of::<f16>();

    pub fn of(frame: &Frame) -> Self {
        let mut data = vec![f16::ZERO; frame.sample_count()];
        write_half(&mut data, frame.data());
        Self { width: frame.width(), height: frame.height(), channels: frame.channels(), data }
    }

    /// Replaces the held samples with `frame`'s, in place: a sub is held once, unwarped
    /// and then warped, never twice.
    pub fn overwrite(&mut self, frame: &Frame) -> Result<()> {
        self.check(frame)?;
        write_half(&mut self.data, frame.data());
        Ok(())
    }

    /// The held sub as a frame again, written into `out`.
    pub fn restore_into(&self, out: &mut Frame) -> Result<()> {
        self.check(out)?;
        read_half(&self.data, out.data_mut());
        Ok(())
    }

    /// The samples from `offset` on, enough to fill `out` (planar order, as `Frame`).
    pub fn read(&self, offset: usize, out: &mut [f32]) {
        read_half(&self.data[offset..offset + out.len()], out);
    }

    pub fn sample_count(&self) -> usize {
        self.data.len()
    }

    pub fn memory_size(&self) -> usize {
        self.data.len() * Self::BYTES_PER_SAMPLE
    }

    fn check(&self, frame: &Frame) -> Result<()> {
        if (frame.width(), frame.height(), frame.channels()) == (self.width, self.height, self.channels) {
            return Ok(());
        }
        Err(StackError::CalibrationDimensionMismatch {
            frame_width: frame.width(),
            frame_height: frame.height(),
            cal_width: self.width,
            cal_height: self.height,
        })
    }
}

fn write_half(to: &mut [f16], from: &[f32]) {
    to.par_chunks_mut(CONVERT_CHUNK)
        .zip(from.par_chunks(CONVERT_CHUNK))
        .for_each(|(to, from)| to.convert_from_f32_slice(from));
}

fn read_half(from: &[f16], to: &mut [f32]) {
    to.par_chunks_mut(CONVERT_CHUNK)
        .zip(from.par_chunks(CONVERT_CHUNK))
        .for_each(|(to, from)| from.convert_to_f32_slice(to));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sky sample keeps its value to well under its own noise, and the warp's border
    /// stays exactly zero, which is what the accumulator skips.
    #[test]
    fn a_held_frame_restores_within_half_precision() {
        let values = [0.0f32, 1.3e-4, 0.0028, 0.0031, 0.47, 1.0];
        let frame = Frame::from_f32_vec(values.to_vec(), values.len(), 1, 1).unwrap();
        let mut restored = Frame::zeros(values.len(), 1, 1).unwrap();
        HeldFrame::of(&frame).restore_into(&mut restored).unwrap();
        for (&was, &is) in values.iter().zip(restored.data()) {
            assert!((was - is).abs() <= was / 2048.0, "{was} came back as {is}");
        }
        assert_eq!(restored.data()[0], 0.0);
    }

    #[test]
    fn a_frame_of_another_size_is_refused() {
        let mut held = HeldFrame::of(&Frame::zeros(4, 4, 1).unwrap());
        assert!(held.overwrite(&Frame::zeros(4, 4, 3).unwrap()).is_err());
    }
}
