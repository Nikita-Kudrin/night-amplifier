//! Comet stacking, driven through the Pro `CometPlugin`.

use tracing::{info, warn};

use super::{LiveStacker, StackSettings};
use crate::frame::{Frame, NoiseField};
use crate::session::capture::frame_gate::FrameAdmission;
use crate::stacking::{CometContext, StackingType};

/// A [`LiveStacker`] over the plugin's context, which owns the nucleus alignment.
pub struct CometStacker(Box<dyn CometContext>);

impl CometStacker {
    /// `Err` without the Pro plugin: Community has no comet alignment to run.
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        settings: &StackSettings,
    ) -> Result<Self, String> {
        let plugin = settings
            .plugins
            .comet()
            .ok_or_else(|| "Comet stacking plugin not found (Pro feature)".to_string())?;
        Ok(Self(plugin.create_context(width, height, channels, &settings.comet())))
    }

    #[cfg(test)]
    pub(crate) fn from_context(context: Box<dyn CometContext>) -> Self {
        Self(context)
    }
}

impl LiveStacker for CometStacker {
    fn kind(&self) -> StackingType {
        StackingType::Comet
    }

    fn geometry(&self) -> (usize, usize, usize) {
        (self.0.width(), self.0.height(), self.0.channels())
    }

    fn depth(&self) -> usize {
        self.0.frame_count()
    }

    fn has_reference(&self) -> bool {
        self.0.frame_count() > 0
    }

    fn apply_settings(&mut self, settings: &StackSettings) {
        self.0.update_from_settings(&settings.comet());
        let Some(roi) = settings.comet_roi else {
            return;
        };
        let current = self.0.get_roi();
        let moved = roi.x != current.x
            || roi.y != current.y
            || roi.width != current.width
            || roi.height != current.height;
        if moved {
            info!(x = roi.x, y = roi.y, width = roi.width, height = roi.height, "Comet ROI updated");
            self.0.update_roi(roi);
        }
    }

    fn set_reference(&mut self, frame: &Frame) -> Result<(), String> {
        self.0.initialize_with_reference(frame).map_err(|e| e.to_string())?;
        info!("Comet stacking initialized with reference frame");
        Ok(())
    }

    fn offer(&mut self, frame: &Frame, _settings: &StackSettings) -> Result<FrameAdmission, String> {
        // An error is a frame the nucleus could not be found in, not a broken stack:
        // the accumulated comet stays on screen.
        let added = match self.0.add_frame(frame) {
            Ok(true) => {
                info!(frame_count = self.0.frame_count(), "Frame added to comet stack");
                true
            }
            Ok(false) => {
                info!(
                    frame_count = self.0.frame_count(),
                    "Comet alignment failed, frame not added to stack"
                );
                false
            }
            Err(e) => {
                warn!(error = %e, "Error adding frame to comet stack");
                false
            }
        };
        Ok(FrameAdmission::unreasoned(added))
    }

    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), String> {
        self.0.compute().map(|frame| (frame, None)).map_err(|e| e.to_string())
    }
}

/// A scripted [`CometContext`]: each `add_frame` pops the next scripted answer, and the
/// stack is a flat frame whose level is its depth, so a test can tell stacks apart.
#[cfg(test)]
pub(crate) struct StubComet {
    geometry: (usize, usize, usize),
    frames: usize,
    roi: crate::planetary::AlignmentRoi,
    /// Shared so a test can keep counting once the stub is boxed into a stacker.
    pub(crate) roi_updates: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    answers: std::collections::VecDeque<crate::error::Result<bool>>,
}

#[cfg(test)]
impl StubComet {
    pub(crate) fn new(width: usize, height: usize, channels: usize) -> Self {
        Self {
            geometry: (width, height, channels),
            frames: 0,
            roi: crate::planetary::AlignmentRoi::centered(width, height, width.min(height) / 2),
            roi_updates: Default::default(),
            answers: Default::default(),
        }
    }

    /// Answers for the next `add_frame` calls; past them, every frame aligns.
    pub(crate) fn answering(mut self, answers: Vec<crate::error::Result<bool>>) -> Self {
        self.answers = answers.into();
        self
    }
}

#[cfg(test)]
impl CometContext for StubComet {
    fn update_roi(&mut self, roi: crate::planetary::AlignmentRoi) {
        self.roi = roi;
        self.roi_updates.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn initialize_with_reference(&mut self, _frame: &Frame) -> crate::error::Result<()> {
        self.frames = 1;
        Ok(())
    }

    fn add_frame(&mut self, _frame: &Frame) -> crate::error::Result<bool> {
        let added = self.answers.pop_front().unwrap_or(Ok(true))?;
        self.frames += usize::from(added);
        Ok(added)
    }

    fn compute(&self) -> crate::error::Result<Frame> {
        if self.frames == 0 {
            return Err(crate::error::StackError::EmptyStack);
        }
        let (width, height, channels) = self.geometry;
        Frame::filled(width, height, channels, self.frames as f32 / 100.0)
    }

    fn frame_count(&self) -> usize {
        self.frames
    }

    fn width(&self) -> usize {
        self.geometry.0
    }

    fn height(&self) -> usize {
        self.geometry.1
    }

    fn channels(&self) -> usize {
        self.geometry.2
    }

    fn update_from_settings(&mut self, _settings: &crate::stacking::CometSettings) {}

    fn get_roi(&self) -> crate::planetary::AlignmentRoi {
        self.roi
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planetary::AlignmentRoi;
    use crate::session::state::CaptureSettings;

    fn stacker(answers: Vec<crate::error::Result<bool>>) -> CometStacker {
        CometStacker::from_context(Box::new(StubComet::new(32, 32, 1).answering(answers)))
    }

    fn frame() -> Frame {
        Frame::filled(32, 32, 1, 0.1).unwrap()
    }

    #[test]
    fn without_the_plugin_there_is_no_comet_stack() {
        let settings = StackSettings::of(&CaptureSettings::default(), &crate::plugins::Plugins::none());
        let refused = CometStacker::new(32, 32, 1, &settings);
        assert!(refused.is_err(), "Community has no comet alignment to run");
    }

    #[test]
    fn the_reference_is_what_makes_a_stack() {
        let mut comet = stacker(vec![]);
        assert!(!comet.has_reference());
        comet.set_reference(&frame()).unwrap();
        assert!(comet.has_reference());
        assert_eq!(comet.depth(), 1);
        assert_eq!(comet.geometry(), (32, 32, 1));
    }

    /// The nucleus not being found is a verdict on the frame, not on the stack: the
    /// accumulated comet has to stay on screen, so neither outcome is an `Err`.
    #[test]
    fn a_frame_the_nucleus_cannot_be_found_in_leaves_the_stack_alone() {
        let settings = StackSettings::of(&CaptureSettings::default(), &crate::plugins::Plugins::none());
        let mut comet = stacker(vec![
            Ok(false),
            Err(crate::error::StackError::Registration("lost the nucleus".into())),
            Ok(true),
        ]);
        comet.set_reference(&frame()).unwrap();

        for _ in 0..2 {
            let verdict = comet.offer(&frame(), &settings).expect("never an Err");
            assert!(!verdict.added);
            assert_eq!(verdict.rejected_because, None, "comet reports no reasons");
        }
        assert!(comet.offer(&frame(), &settings).unwrap().added);
        assert_eq!(comet.depth(), 2);

        let (stack, noise) = comet.snapshot().unwrap();
        assert!((stack.get_pixel(0, 0, 0) - 0.02).abs() < 1e-6, "the two-frame stack");
        assert!(noise.is_none(), "the plugin keeps no coverage map");
    }

    /// The ROI the observer drew reaches the plugin once, and only when it moved.
    #[test]
    fn an_edited_roi_reaches_the_plugin_once() {
        use std::sync::atomic::Ordering;

        let stub = StubComet::new(32, 32, 1);
        let updates = std::sync::Arc::clone(&stub.roi_updates);
        let mut comet = CometStacker::from_context(Box::new(stub));
        let mut settings = StackSettings::of(&CaptureSettings::default(), &crate::plugins::Plugins::none());

        comet.apply_settings(&settings);
        assert_eq!(updates.load(Ordering::Relaxed), 0, "no ROI drawn, nothing to send");

        settings.comet_roi = Some(AlignmentRoi { x: 3, y: 4, width: 10, height: 12 });
        comet.apply_settings(&settings);
        comet.apply_settings(&settings);
        assert_eq!(updates.load(Ordering::Relaxed), 1, "re-sent an unchanged ROI");

        let roi = comet.0.get_roi();
        assert_eq!((roi.x, roi.y, roi.width, roi.height), (3, 4, 10, 12));
    }
}
