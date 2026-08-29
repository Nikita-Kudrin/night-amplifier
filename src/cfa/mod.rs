//! Raw-CFA stage: corrections that must run on the sensor mosaic
//!
//! Every colour camera in this codebase used to debayer inside its capture
//! shim, so the first thing any pipeline code saw was already RGB. That leaves
//! nowhere to put a correction that is only meaningful on the mosaic — and
//! several are:
//!
//! - **Hot pixels** ([`hot_pixels`]) — filtered after demosaic, one hot site has
//!   already been smeared into a coloured 3x3 cross by the interpolation.
//! - **Row / column fixed-pattern noise** ([`fpn`]) — a readout artefact of the
//!   sensor's own rows and columns. Debayering mixes neighbouring rows, so the
//!   pattern stops being a pattern.
//! - **Dark and flat calibration** ([`crate::calibration`]) — `(raw - dark) / flat`
//!   is defined on raw sensor samples, which is why that module has no call
//!   sites in the server yet.
//!
//! [`RawFrame::to_cfa_frame`](crate::camera::RawFrame::to_cfa_frame) now hands
//! back a [`CfaFrame`] — still mosaiced for a colour sensor — and the stacking
//! task debayers after running [`CfaPipeline`] over it. A pipeline with no
//! stages is bit-identical to debayering straight out of the shim.
//!
//! # Planes
//!
//! Both corrections work on one *colour site* at a time: the sub-lattice of
//! samples that share a filter. A Bayer mosaic has four ([`CfaFrame::step`] of
//! 2), a mono sensor has one (step of 1), and both are described by
//! [`CfaPlanes`] so the filters need no separate mono path. Mixing sites is the
//! bug this exists to prevent: an R sample and its neighbouring B sample sit at
//! entirely different levels, so a filter that treats them as neighbours reads
//! the mosaic itself as signal.

pub mod fpn;
pub mod hot_pixels;

use crate::debayer::{CfaPattern, DebayerAlgorithm, DebayerConfig, Debayerer};
use crate::error::Result;
use crate::frame::Frame;

pub use fpn::{remove_fpn, FpnFilter, FpnStats};
pub use hot_pixels::{reject_hot_pixels, HotPixelConfig, HotPixelFilter, HotPixelStats};

/// A frame as the sensor produced it: still carrying its CFA mosaic when the
/// sensor is a colour one.
///
/// `pattern` is `Some` exactly when `frame` is a single-channel mosaic awaiting
/// demosaic. A mono sensor, or a source that already arrived as RGB, carries
/// `None` and passes through [`Self::debayer`] untouched.
#[derive(Debug, Clone)]
pub struct CfaFrame {
    frame: Frame,
    pattern: Option<CfaPattern>,
}

impl CfaFrame {
    /// Wrap a single-channel mosaic that still needs demosaicing.
    pub fn mosaic(frame: Frame, pattern: CfaPattern) -> Result<Self> {
        if frame.channels() != 1 {
            return Err(crate::error::StackError::ChannelMismatch {
                expected: 1,
                actual: frame.channels(),
            });
        }
        Ok(Self {
            frame,
            pattern: Some(pattern),
        })
    }

    /// Wrap a frame that carries no mosaic — a mono sensor, or already-RGB data.
    pub fn direct(frame: Frame) -> Self {
        Self {
            frame,
            pattern: None,
        }
    }

    /// The CFA pattern, or `None` when this frame carries no mosaic.
    #[inline]
    pub fn pattern(&self) -> Option<CfaPattern> {
        self.pattern
    }

    /// Whether a demosaic is still owed on this frame.
    #[inline]
    pub fn is_mosaic(&self) -> bool {
        self.pattern.is_some()
    }

    /// The underlying frame.
    #[inline]
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// The underlying frame, mutably — how a [`CfaStage`] does its work.
    #[inline]
    pub fn frame_mut(&mut self) -> &mut Frame {
        &mut self.frame
    }

    /// Distance between two samples of the same colour site: 2 across a Bayer
    /// mosaic, 1 when there is none.
    #[inline]
    pub fn step(&self) -> usize {
        if self.is_mosaic() {
            2
        } else {
            1
        }
    }

    /// The colour sites this frame is made of, for a filter to walk one at a time.
    ///
    /// Only defined for single-channel data; an already-RGB frame has no
    /// sub-lattice structure and yields `None`.
    pub fn planes(&self) -> Option<CfaPlanes> {
        if self.frame.channels() != 1 {
            return None;
        }
        Some(CfaPlanes {
            step: self.step(),
            width: self.frame.width(),
            height: self.frame.height(),
        })
    }

    /// Demosaic into RGB, consuming the CFA frame.
    ///
    /// A frame with no pattern is returned as it is, so this is the single exit
    /// from the raw stage regardless of sensor type.
    pub fn debayer(self, algorithm: DebayerAlgorithm) -> Result<Frame> {
        let Some(pattern) = self.pattern else {
            return Ok(self.frame);
        };
        let debayerer = Debayerer::new(DebayerConfig::new(pattern).with_algorithm(algorithm));
        debayerer.debayer(&self.frame)
    }

    /// The frame without demosaicing — for callers that want the mosaic itself.
    pub fn into_frame(self) -> Frame {
        self.frame
    }
}

/// The colour sites of a mosaic, as a sub-lattice description.
///
/// A site is identified by its origin parity `(x0, y0)`, each in `0..step`, and
/// contains every sample at `x % step == x0 && y % step == y0`.
#[derive(Debug, Clone, Copy)]
pub struct CfaPlanes {
    /// 2 for a Bayer mosaic, 1 for mono.
    pub step: usize,
    /// Full frame width.
    pub width: usize,
    /// Full frame height.
    pub height: usize,
}

impl CfaPlanes {
    /// Number of colour sites: 4 for a Bayer mosaic, 1 for mono.
    #[inline]
    pub fn count(&self) -> usize {
        self.step * self.step
    }

    /// Number of samples of one site along a row.
    #[inline]
    pub fn plane_width(&self, x0: usize) -> usize {
        self.width.saturating_sub(x0).div_ceil(self.step)
    }

    /// Number of samples of one site down a column.
    #[inline]
    pub fn plane_height(&self, y0: usize) -> usize {
        self.height.saturating_sub(y0).div_ceil(self.step)
    }

    /// Origin parities of every colour site, in `(x0, y0)` order.
    pub fn origins(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..self.step).flat_map(move |y0| (0..self.step).map(move |x0| (x0, y0)))
    }
}

/// A correction that runs on raw sensor data, before demosaic.
///
/// The hook the plan calls for: a stage registers here rather than editing the
/// capture seam, so dark subtraction lands as one more `Box<dyn CfaStage>`.
///
/// Stages are only handed single-channel frames — a mosaic or a mono sensor.
/// [`CfaPipeline`] skips the whole list for a source that already arrived as
/// RGB, because none of these corrections is defined there.
pub trait CfaStage: Send + Sync {
    /// Name used in the tracing span for this stage.
    fn name(&self) -> &'static str;

    /// Apply the correction in place.
    fn apply(&self, frame: &mut CfaFrame) -> Result<()>;
}

/// The ordered set of pre-debayer corrections for one capture session.
///
/// Built once when settings change rather than per frame, because a stage may
/// own precomputed state (a master dark, eventually).
#[derive(Default)]
pub struct CfaPipeline {
    stages: Vec<Box<dyn CfaStage>>,
}

impl CfaPipeline {
    /// An empty pipeline — debayering straight through, exactly as before this
    /// stage existed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a stage. Order is the order corrections are applied.
    pub fn with_stage(mut self, stage: Box<dyn CfaStage>) -> Self {
        self.stages.push(stage);
        self
    }

    /// Whether this pipeline would do anything.
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Names of the registered stages, in order.
    pub fn stage_names(&self) -> Vec<&'static str> {
        self.stages.iter().map(|s| s.name()).collect()
    }

    /// Run every stage over the frame in place.
    ///
    /// A failing stage is logged and skipped rather than failing the frame: a
    /// correction that cannot be computed must not cost the exposure.
    pub fn apply(&self, frame: &mut CfaFrame) {
        if frame.planes().is_none() {
            tracing::debug!(
                channels = frame.frame().channels(),
                "Source is already RGB; raw-CFA stages skipped"
            );
            return;
        }
        for stage in &self.stages {
            let _span = tracing::debug_span!("cfa_stage", stage = stage.name()).entered();
            if let Err(e) = stage.apply(frame) {
                tracing::warn!(stage = stage.name(), error = %e, "CFA stage failed, skipping");
            }
        }
    }
}

impl std::fmt::Debug for CfaPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CfaPipeline")
            .field("stages", &self.stage_names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mosaic_frame(width: usize, height: usize) -> CfaFrame {
        let frame = Frame::filled(width, height, 1, 0.25).unwrap();
        CfaFrame::mosaic(frame, CfaPattern::Rggb).unwrap()
    }

    #[test]
    fn mosaic_rejects_multi_channel_input() {
        let rgb = Frame::filled(4, 4, 3, 0.5).unwrap();
        assert!(CfaFrame::mosaic(rgb, CfaPattern::Rggb).is_err());
    }

    #[test]
    fn a_bayer_mosaic_has_four_sites_a_mono_frame_one() {
        let mosaic = mosaic_frame(8, 6);
        let planes = mosaic.planes().unwrap();
        assert_eq!(planes.step, 2);
        assert_eq!(planes.count(), 4);
        assert_eq!(planes.origins().count(), 4);

        let mono = CfaFrame::direct(Frame::filled(8, 6, 1, 0.1).unwrap());
        let planes = mono.planes().unwrap();
        assert_eq!(planes.step, 1);
        assert_eq!(planes.count(), 1);
        assert_eq!(planes.origins().collect::<Vec<_>>(), vec![(0, 0)]);
    }

    #[test]
    fn odd_dimensions_split_unevenly_between_sites() {
        let planes = mosaic_frame(7, 5).planes().unwrap();
        assert_eq!(planes.plane_width(0), 4);
        assert_eq!(planes.plane_width(1), 3);
        assert_eq!(planes.plane_height(0), 3);
        assert_eq!(planes.plane_height(1), 2);
    }

    #[test]
    fn an_rgb_frame_has_no_sub_lattice() {
        let rgb = CfaFrame::direct(Frame::filled(4, 4, 3, 0.5).unwrap());
        assert!(rgb.planes().is_none());
        assert!(!rgb.is_mosaic());
    }

    #[test]
    fn debayer_passes_a_non_mosaic_frame_through_unchanged() {
        let mut rgb = Frame::zeros(4, 4, 3).unwrap();
        rgb.set_pixel(1, 2, 1, 0.75);
        let out = CfaFrame::direct(rgb)
            .debayer(DebayerAlgorithm::Bilinear)
            .unwrap();
        assert_eq!(out.channels(), 3);
        assert_eq!(out.get_pixel(1, 2, 1), 0.75);
    }

    struct Bump;
    impl CfaStage for Bump {
        fn name(&self) -> &'static str {
            "bump"
        }
        fn apply(&self, frame: &mut CfaFrame) -> Result<()> {
            for v in frame.frame_mut().data_mut() {
                *v += 0.1;
            }
            Ok(())
        }
    }

    struct Boom;
    impl CfaStage for Boom {
        fn name(&self) -> &'static str {
            "boom"
        }
        fn apply(&self, _frame: &mut CfaFrame) -> Result<()> {
            Err(crate::error::StackError::InvalidConfiguration(
                "nope".into(),
            ))
        }
    }

    #[test]
    fn an_empty_pipeline_leaves_the_frame_alone() {
        let mut cfa = mosaic_frame(4, 4);
        let pipeline = CfaPipeline::new();
        assert!(pipeline.is_empty());
        pipeline.apply(&mut cfa);
        assert!(cfa.frame().data().iter().all(|&v| v == 0.25));
    }

    #[test]
    fn stages_run_in_registration_order_and_a_failure_does_not_stop_the_rest() {
        let mut cfa = mosaic_frame(4, 4);
        let pipeline = CfaPipeline::new()
            .with_stage(Box::new(Bump))
            .with_stage(Box::new(Boom))
            .with_stage(Box::new(Bump));
        assert_eq!(pipeline.stage_names(), vec!["bump", "boom", "bump"]);

        pipeline.apply(&mut cfa);
        assert!(cfa.frame().data().iter().all(|&v| (v - 0.45).abs() < 1e-6));
    }
}
