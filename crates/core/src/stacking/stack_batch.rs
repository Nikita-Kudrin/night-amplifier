//! Starting a stack over from a batch of subs held side by side.
//!
//! Streaming judges each sample against the samples before it, so the first few of every
//! pixel are judged against nothing: a trail in sub 1 is averaged in, and inflates the
//! scale later subs are clipped against. A batch shows every sample of a pixel at once,
//! so the rejection plugin can call one out against the others before any is blended.

use rayon::prelude::*;
use tracing::warn;

use super::{MasterStack, BATCH_BAND};
use crate::error::{Result, StackError};
use crate::stacking::config::FrameQuality;
use crate::stacking::held::HeldFrame;
use crate::stacking::rejection::RejectionPlugin;

/// Distance from the border value within which a sample is outside its sub, as streamed.
const BORDER_TOLERANCE: f32 = 1e-6;

impl MasterStack {
    /// Clears the stack and rebuilds it from `subs`, in order (the reference first), each
    /// already in the stack's coordinates with its warp border at `border_value`.
    ///
    /// Every sub is weighed against the whole set, not against the subs before it: a
    /// streamed start gives its first five weight 1.0 however soft they are. Samples the
    /// rejection plugin calls outliers within the batch never reach the accumulator.
    ///
    /// Refuses before touching the stack and cannot fail after it: the plugin is resolved
    /// once, up front, and one that errors has the batch rebuilt as the plain mean — a
    /// licence lapsing mid-restack once left the session's first subs an empty stack.
    pub fn restack(&mut self, subs: &[HeldFrame], qualities: &[FrameQuality], border_value: f32) -> Result<()> {
        let samples = self.pixels.len();
        if subs.len() != qualities.len() || subs.iter().any(|sub| sub.sample_count() != samples) {
            return Err(StackError::InvalidConfiguration(
                "a restack needs one quality per sub, each the stack's size".into(),
            ));
        }
        let plugins = self.plugins.clone();
        let rejection = self.rejection(&plugins).unwrap_or_else(|error| {
            warn!(%error, "Restacking as the plain mean");
            None
        });
        if let Err(error) = self.rebuild(subs, qualities, rejection, border_value) {
            warn!(%error, "Batch rejection failed, restacking as the plain mean");
            self.rebuild(subs, qualities, None, border_value)?;
        }
        Ok(())
    }

    fn rebuild(
        &mut self,
        subs: &[HeldFrame],
        qualities: &[FrameQuality],
        rejection: Option<&dyn RejectionPlugin>,
        border_value: f32,
    ) -> Result<()> {
        self.clear();
        for quality in qualities {
            self.quality_baseline.record(quality);
        }
        let weights: Vec<f32> = qualities
            .iter()
            .map(|q| self.quality_baseline.calculate_weight(q, &self.config.weighting))
            .collect();

        let samples = self.pixels.len();
        let _span = tracing::info_span!("restack", subs = subs.len(), rejection = rejection.is_some()).entered();
        let mut bands = vec![vec![0.0f32; BATCH_BAND.min(samples)]; subs.len()];
        let mut offset = 0;
        while offset < samples {
            let len = BATCH_BAND.min(samples - offset);
            for (band, sub) in bands.iter_mut().zip(subs) {
                sub.read(offset, &mut band[..len]);
            }
            if let Some(plugin) = rejection {
                let mut views: Vec<&mut [f32]> = bands.iter_mut().map(|b| &mut b[..len]).collect();
                plugin.reject_batch(&mut views, border_value, BORDER_TOLERANCE, &self.config)?;
                self.count_dropped(offset, len, &bands);
            }
            for (band, &weight) in bands.iter().zip(&weights) {
                self.blend(rejection, offset, &band[..len], border_value, BORDER_TOLERANCE, weight)?;
            }
            offset += len;
        }

        self.frame_count = subs.len();
        self.frame_qualities = qualities.to_vec();
        Ok(())
    }

    /// Counts the samples the batch dropped as offered: held subs are finite, so a NaN is a
    /// drop, and it reached its pixel as a streamed clip does. Counted out, a stack every
    /// sub covered read one sub short there for the rest of the session (`coverage_row`).
    fn count_dropped(&mut self, offset: usize, len: usize, bands: &[Vec<f32>]) {
        self.pixels[offset..offset + len].par_iter_mut().enumerate().for_each(|(i, pixel)| {
            let dropped = bands.iter().filter(|band| band[i].is_nan()).count();
            if dropped > 0 {
                pixel.offered = pixel.offered.saturating_add(dropped as u16);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;
    use crate::stacking::StackingConfig;

    fn flat(value: f32) -> Frame {
        Frame::filled(8, 8, 1, value).unwrap()
    }

    /// Without rejection a restack is the stream it replaces: the same mean, depth and
    /// coverage as adding the subs one by one.
    #[test]
    fn a_restack_without_rejection_is_the_streamed_mean() {
        let values = [0.2f32, 0.3, 0.25, 0.4];
        let mut streamed = MasterStack::with_plugins(8, 8, 1, StackingConfig::default(), crate::plugins::Plugins::none()).unwrap();
        for &v in &values {
            streamed.add_frame(&flat(v)).unwrap();
        }

        let mut batch = MasterStack::with_plugins(8, 8, 1, StackingConfig::default(), crate::plugins::Plugins::none()).unwrap();
        batch.add_frame(&flat(0.9)).unwrap();
        let held: Vec<HeldFrame> = values.iter().map(|&v| HeldFrame::of(&flat(v))).collect();
        batch.restack(&held, &vec![FrameQuality::default(); values.len()], 0.0).unwrap();

        assert_eq!(batch.frame_count(), values.len());
        let (a, b) = (streamed.compute().unwrap(), batch.compute().unwrap());
        for (x, y) in a.data().iter().zip(b.data()) {
            assert!((x - y).abs() < 1e-3, "{x} vs {y}");
        }
        assert_eq!(batch.pixels[0].offered, values.len() as u16);
    }

    /// A rejection plugin that fails a restack: its batch pass when `batch`, else its blend.
    /// What a licence lapsing mid-restack looked like before the plugin was resolved once.
    struct Failing {
        batch: bool,
    }

    impl crate::stacking::RejectionPlugin for Failing {
        fn compute_rejection(&self, _: &[f32], _: crate::stacking::RejectionMethod, _: &StackingConfig) -> Result<(f32, u32)> {
            unreachable!("the live path never asks")
        }

        fn compute_weighted_rejection(
            &self,
            _: &[f32],
            _: &[f32],
            _: crate::stacking::RejectionMethod,
            _: &StackingConfig,
        ) -> Result<(f32, f32)> {
            unreachable!("the live path never asks")
        }

        fn blend_incremental(
            &self,
            _: &mut [crate::stacking::IncrementalPixel],
            _: &[f32],
            _: f32,
            _: f32,
            _: f32,
            _: &StackingConfig,
        ) -> Result<()> {
            Err(StackError::InvalidConfiguration("blend failed".into()))
        }

        fn reject_batch(&self, _: &mut [&mut [f32]], _: f32, _: f32, _: &StackingConfig) -> Result<()> {
            match self.batch {
                true => Err(StackError::InvalidConfiguration("batch failed".into())),
                false => Ok(()),
            }
        }
    }

    /// A rejection plugin failing mid-restack leaves the batch's plain mean, not the empty,
    /// half-built stack the failure used to leave behind.
    #[test]
    fn a_failing_rejection_restacks_as_the_plain_mean() {
        for batch in [true, false] {
            let plugins = crate::plugins::Plugins::none().with_rejection(std::sync::Arc::new(Failing { batch })).always_licensed();
            let config = StackingConfig::default().with_rejection(crate::stacking::RejectionMethod::SigmaClip);
            let mut stack = MasterStack::with_plugins(8, 8, 1, config, plugins).unwrap();

            let held: Vec<HeldFrame> = [0.2f32, 0.3, 0.25].iter().map(|&v| HeldFrame::of(&flat(v))).collect();
            stack.restack(&held, &[FrameQuality::default(); 3], 0.0).expect("the plain mean cannot fail");
            assert_eq!(stack.frame_count(), 3, "batch fails: {batch}");
            let mean = stack.compute().unwrap().data()[0];
            assert!((mean - 0.25).abs() < 1e-3, "batch fails: {batch}, mean {mean}");
            assert_eq!(stack.pixels[0].offered, 3);
        }
    }

    #[test]
    fn a_restack_refuses_a_sub_of_another_size() {
        let mut stack = MasterStack::with_plugins(8, 8, 1, StackingConfig::default(), crate::plugins::Plugins::none()).unwrap();
        let held = [HeldFrame::of(&Frame::zeros(4, 4, 1).unwrap())];
        assert!(stack.restack(&held, &[FrameQuality::default()], 0.0).is_err());
    }
}
