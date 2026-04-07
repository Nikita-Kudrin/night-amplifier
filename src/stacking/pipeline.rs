//! High-level stacking pipeline for streaming frame processing.
//!
//! This pipeline handles the complete workflow for live stacking:
//! 1. Receives frames one at a time (as they arrive from camera)
//! 2. Detects stars in each frame
//! 3. Registers against the reference frame
//! 4. Adds to the stack if registration succeeds

use crate::detection::{DetectionConfig, Star, StarDetector};
use crate::error::{Result, StackError};
use crate::frame::Frame;
use crate::registration::{AdaptiveRegistration, AdaptiveRegistrationResult, RegistrationConfig};
use tracing::instrument;

use super::config::StackingConfig;
use super::rejection::RejectionMethod;
use super::stacker::Stacker;

/// Result of processing a single frame through the pipeline.
#[derive(Debug, Clone)]
pub struct FrameProcessingResult {
    /// Whether the frame was successfully stacked
    pub stacked: bool,
    /// Number of stars detected in the frame
    pub stars_detected: usize,
    /// Registration result if successful
    pub registration: Option<AdaptiveRegistrationResult>,
    /// Reason for rejection if not stacked
    pub rejection_reason: Option<String>,
}

/// Statistics about the stacking session.
#[derive(Debug, Clone, Default)]
pub struct StackingStats {
    /// Total frames received
    pub frames_received: usize,
    /// Frames successfully stacked
    pub frames_stacked: usize,
    /// Frames rejected due to insufficient stars
    pub rejected_no_stars: usize,
    /// Frames rejected due to registration failure
    pub rejected_registration: usize,
    /// Frames rejected due to dimension mismatch
    pub rejected_dimensions: usize,
    /// Average stars detected per frame
    pub avg_stars_detected: f32,
    /// Average registration residual
    pub avg_residual: f32,
}

impl StackingStats {
    /// Returns the stacking success rate as a percentage.
    pub fn success_rate(&self) -> f32 {
        if self.frames_received == 0 {
            0.0
        } else {
            100.0 * self.frames_stacked as f32 / self.frames_received as f32
        }
    }
}

/// Configuration for the stacking pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Stacking configuration
    pub stacking: StackingConfig,
    /// Star detection configuration
    pub detection: DetectionConfig,
    /// Registration configuration
    pub registration: RegistrationConfig,
    /// Minimum stars required for registration
    pub min_stars: usize,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            stacking: StackingConfig::default(),
            detection: DetectionConfig::fast(),
            registration: RegistrationConfig::default(),
            min_stars: 3,
        }
    }
}

impl PipelineConfig {
    /// Creates a fast configuration optimized for speed.
    pub fn fast() -> Self {
        Self {
            stacking: StackingConfig::default(),
            detection: DetectionConfig::fast(),
            registration: RegistrationConfig::fast(),
            min_stars: 3,
        }
    }

    /// Creates a quality configuration optimized for accuracy.
    pub fn quality() -> Self {
        Self {
            stacking: StackingConfig::default()
                .with_rejection(RejectionMethod::WinsorizedSigmaClip)
                .with_sigma(2.0),
            detection: DetectionConfig::default(),
            registration: RegistrationConfig::robust(),
            min_stars: 5,
        }
    }

    /// Sets the minimum stars required for registration.
    pub fn with_min_stars(mut self, min_stars: usize) -> Self {
        self.min_stars = min_stars;
        self
    }
}

/// High-level stacking pipeline for streaming frame processing.
///
/// This pipeline handles the complete workflow for live stacking:
/// 1. Receives frames one at a time (as they arrive from camera)
/// 2. Detects stars in each frame
/// 3. Registers against the reference frame
/// 4. Adds to the stack if registration succeeds
///
/// # Example
///
/// ```ignore
/// use night_amplifier::{StackingPipeline, PipelineConfig, Frame};
///
/// // Create pipeline with first frame as reference
/// let config = PipelineConfig::fast();
/// let mut pipeline = StackingPipeline::new(reference_frame, config)?;
///
/// // Process incoming frames
/// for frame in camera_frames {
///     let result = pipeline.process_frame(&frame);
///     if result.stacked {
///         println!("Frame stacked successfully");
///     }
/// }
///
/// // Get final result
/// let stacked = pipeline.compute()?;
/// ```
pub struct StackingPipeline {
    stacker: Stacker,
    ref_stars: Vec<Star>,
    detector: StarDetector,
    registration: AdaptiveRegistration,
    config: PipelineConfig,
    stats: StackingStats,
    ref_width: usize,
    ref_height: usize,
    ref_channels: usize,
    total_stars_detected: usize,
    total_residual: f32,
    residual_count: usize,
}

impl StackingPipeline {
    /// Creates a new stacking pipeline with the given reference frame.
    ///
    /// The reference frame is used for:
    /// - Determining output dimensions
    /// - Star detection for registration reference
    /// - First frame in the stack
    #[instrument(skip(reference, config), fields(
        resolution = %format!("{}x{}x{}", reference.width(), reference.height(), reference.channels()),
        min_stars = config.min_stars
    ))]
    pub fn new(reference: &Frame, config: PipelineConfig) -> Result<Self> {
        let ref_width = reference.width();
        let ref_height = reference.height();
        let ref_channels = reference.channels();

        let detector = StarDetector::new(config.detection.clone());
        let ref_stars = detector.detect(reference)?;

        if ref_stars.len() < config.min_stars {
            return Err(StackError::Detection(format!(
                "Reference frame has only {} stars, minimum {} required",
                ref_stars.len(),
                config.min_stars
            )));
        }

        let mut stacker =
            Stacker::new(ref_width, ref_height, ref_channels, config.stacking.clone())?;

        stacker.add_reference(reference)?;

        let registration = AdaptiveRegistration::new();

        let mut stats = StackingStats::default();
        stats.frames_received = 1;
        stats.frames_stacked = 1;

        let ref_stars_count = ref_stars.len();

        Ok(Self {
            stacker,
            ref_stars,
            detector,
            registration,
            config,
            stats,
            ref_width,
            ref_height,
            ref_channels,
            total_stars_detected: ref_stars_count,
            total_residual: 0.0,
            residual_count: 0,
        })
    }

    /// Creates a pipeline with default fast configuration.
    pub fn fast(reference: &Frame) -> Result<Self> {
        Self::new(reference, PipelineConfig::fast())
    }

    /// Creates a pipeline with quality configuration.
    pub fn quality(reference: &Frame) -> Result<Self> {
        Self::new(reference, PipelineConfig::quality())
    }

    /// Processes a single frame through the pipeline.
    ///
    /// This method:
    /// 1. Validates frame dimensions
    /// 2. Detects stars
    /// 3. Attempts registration against reference
    /// 4. Adds to stack if successful
    ///
    /// The frame can be dropped after this call - only pixel values
    /// needed for stacking are retained.
    pub fn process_frame(&mut self, frame: &Frame) -> FrameProcessingResult {
        self.stats.frames_received += 1;

        if let Some(rejection_reason) = self.validate_dimensions(frame) {
            self.stats.rejected_dimensions += 1;
            return FrameProcessingResult {
                stacked: false,
                stars_detected: 0,
                registration: None,
                rejection_reason: Some(rejection_reason),
            };
        }

        let stars = match self.detector.detect(frame) {
            Ok(s) => s,
            Err(e) => {
                self.stats.rejected_no_stars += 1;
                return FrameProcessingResult {
                    stacked: false,
                    stars_detected: 0,
                    registration: None,
                    rejection_reason: Some(format!("Star detection failed: {}", e)),
                };
            }
        };

        let stars_detected = stars.len();
        self.total_stars_detected += stars_detected;

        if stars.len() < self.config.min_stars {
            self.stats.rejected_no_stars += 1;
            return FrameProcessingResult {
                stacked: false,
                stars_detected,
                registration: None,
                rejection_reason: Some(format!(
                    "Insufficient stars: {} detected, {} required",
                    stars.len(),
                    self.config.min_stars
                )),
            };
        }

        let reg_result = match self.registration.register(&self.ref_stars, &stars) {
            Ok(r) => r,
            Err(e) => {
                self.stats.rejected_registration += 1;
                return FrameProcessingResult {
                    stacked: false,
                    stars_detected,
                    registration: None,
                    rejection_reason: Some(format!("Registration failed: {}", e)),
                };
            }
        };

        self.total_residual += reg_result.mean_residual;
        self.residual_count += 1;

        if let Err(e) = self.stacker.add_frame(frame, &reg_result.transform) {
            self.stats.rejected_registration += 1;
            return FrameProcessingResult {
                stacked: false,
                stars_detected,
                registration: Some(reg_result),
                rejection_reason: Some(format!("Stacking failed: {}", e)),
            };
        }

        self.stats.frames_stacked += 1;

        FrameProcessingResult {
            stacked: true,
            stars_detected,
            registration: Some(reg_result),
            rejection_reason: None,
        }
    }

    /// Validates frame dimensions against the reference.
    fn validate_dimensions(&self, frame: &Frame) -> Option<String> {
        if frame.width() != self.ref_width
            || frame.height() != self.ref_height
            || frame.channels() != self.ref_channels
        {
            Some(format!(
                "Dimension mismatch: expected {}x{}x{}, got {}x{}x{}",
                self.ref_width,
                self.ref_height,
                self.ref_channels,
                frame.width(),
                frame.height(),
                frame.channels()
            ))
        } else {
            None
        }
    }

    /// Computes the final stacked result.
    ///
    /// This applies the rejection algorithm and produces the final image.
    /// The pipeline can continue to accept frames after this call.
    #[instrument(skip(self), fields(
        frames_stacked = self.stats.frames_stacked,
        frames_rejected = self.stats.frames_received - self.stats.frames_stacked
    ))]
    pub fn compute(&self) -> Result<Frame> {
        self.stacker.compute()
    }

    /// Returns current stacking statistics.
    pub fn stats(&self) -> StackingStats {
        let mut stats = self.stats.clone();

        if self.stats.frames_received > 0 {
            stats.avg_stars_detected =
                self.total_stars_detected as f32 / self.stats.frames_received as f32;
        }
        if self.residual_count > 0 {
            stats.avg_residual = self.total_residual / self.residual_count as f32;
        }

        stats
    }

    /// Returns the number of frames successfully stacked.
    pub fn frame_count(&self) -> usize {
        self.stacker.frame_count()
    }

    /// Returns the reference stars used for registration.
    pub fn reference_stars(&self) -> &[Star] {
        &self.ref_stars
    }

    /// Clears the stack and statistics for reuse.
    ///
    /// The reference frame and stars are retained.
    pub fn clear(&mut self) {
        self.stacker.clear();
        self.stats = StackingStats::default();
        self.stats.frames_received = 1;
        self.stats.frames_stacked = 1;
        self.total_stars_detected = self.ref_stars.len();
        self.total_residual = 0.0;
        self.residual_count = 0;
    }

    /// Returns the coverage map showing how many frames contributed to each pixel.
    pub fn coverage_map(&self) -> Frame {
        self.stacker.coverage_map()
    }
}
