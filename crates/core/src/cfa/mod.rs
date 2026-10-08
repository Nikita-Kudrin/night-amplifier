//! Raw-CFA stage: corrections that must run on the sensor mosaic, before demosaic turns it into RGB.
//! **Hot pixels** ([`hot_pixels`]) smear into a coloured 3x3 cross once debayered; **row/column FPN**
//! ([`fpn`]) stops being a pattern once rows mix; **dark/flat calibration** is defined on raw samples.
//!
//! [`RawFrame::to_cfa_frame`](crate::camera::RawFrame::to_cfa_frame) hands back a still-mosaiced
//! [`CfaFrame`]; stacking debayers after running [`CfaPipeline`] over it (empty pipeline = bit-
//! identical to debayering direct). Both corrections work one *colour site* at a time (4 for Bayer,
//! 1 for mono, see [`CfaPlanes`]) — mixing sites reads the mosaic pattern itself as signal.

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
            // `info_span!`, not `debug_span!`: these run per captured frame and
            // cost real milliseconds on a Pi, and `--span-timings` — which
            // AGENTS.md names as *the* on-device breakdown — filters at INFO.
            // A stage nobody can measure is a stage nobody can decide about.
            let _span = tracing::info_span!("cfa_stage", stage = stage.name()).entered();
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

/// Corrections that run on the raw CFA mosaic, before demosaic.
///
/// These target defects stacking cannot remove: a hot pixel and a readout offset sit
/// in the same place in every sub, so averaging leaves them untouched. Both are only
/// well defined on the mosaic — after demosaic a hot site is smeared into a coloured 3x3 cross, and neighbouring sensor rows are mixed together.
///
/// Hot-pixel rejection has no switch: it always runs, `hot_pixel_sigma` is its only
/// tuning. See `stage_config::build_cfa_pipeline` for why.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "api-schema", derive(schemars::JsonSchema))]
pub struct SensorCorrectionSettings {
    /// How far above its brightest same-colour neighbour a sample must sit to
    /// count as hot, in sigmas of that colour site's own noise.
    #[serde(default = "default_hot_pixel_sigma")]
    pub hot_pixel_sigma: f32,
    /// Flatten per-row and per-column readout offsets.
    #[serde(default = "default_fpn_removal")]
    pub fpn_removal: bool,
    /// Bin each 2x2 CFA quad to one RGB pixel instead of interpolating.
    ///
    /// Halves both dimensions. Free on a sensor that oversamples the eyepiece
    /// screen (IMX533's 3008² becomes 1504², still above 1440²) and a real loss
    /// on one that does not (IMX464 lands at 1356x769), which is why it is off
    /// by default.
    #[serde(default)]
    pub superpixel_debayer: bool,
}

/// The range `hot_pixel_sigma` is held to at every boundary it can arrive through. Keep in
/// sync with `HOT_PIXEL_SIGMA_LIMITS` in `web/src/constants/index.js`.
///
/// Enforced rather than trusted because the stage has no off switch any more, so this is
/// the only way to break it. Measured on a guide sub: sigma <= 0 or a huge value disables it
/// outright (0 corrections), and 0.5-1 replaced 48-65k noise samples a frame.
const HOT_PIXEL_SIGMA_RANGE: std::ops::RangeInclusive<f32> = 3.0..=12.0;

fn default_hot_pixel_sigma() -> f32 {
    5.0
}

fn default_fpn_removal() -> bool {
    true
}

impl SensorCorrectionSettings {
    /// This block with `hot_pixel_sigma` inside [`HOT_PIXEL_SIGMA_RANGE`]; a non-finite
    /// value falls back to the default rather than to either end of the range.
    pub fn sanitized(mut self) -> Self {
        self.hot_pixel_sigma = if self.hot_pixel_sigma.is_finite() {
            self.hot_pixel_sigma.clamp(
                *HOT_PIXEL_SIGMA_RANGE.start(),
                *HOT_PIXEL_SIGMA_RANGE.end(),
            )
        } else {
            default_hot_pixel_sigma()
        };
        self
    }
}

impl Default for SensorCorrectionSettings {
    fn default() -> Self {
        Self {
            hot_pixel_sigma: default_hot_pixel_sigma(),
            fpn_removal: default_fpn_removal(),
            superpixel_debayer: false,
        }
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
