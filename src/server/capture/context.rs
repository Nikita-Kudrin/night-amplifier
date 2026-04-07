//! Stacking contexts for capture loop
//!
//! This module provides `StackingContext` and `PlanetaryStackingContext` which
//! encapsulate the logic for frame registration and accumulation.

use tracing::{debug, info, warn};

use crate::detection::{
    compute_median_fwhm, compute_median_snr, DetectionConfig, Star, StarDetector,
};
use crate::frame::Frame;
use crate::planetary::AlignmentRoi;
use crate::registration::AdaptiveRegistration;
use crate::server::state::CaptureSettings;
use crate::stacking::{
    FrameQuality, RejectionMethod, Stacker, StackingConfig, WeightingConfig, WeightingPreset,
    REJECTION_PLUGIN,
};

/// Holds state for the live stacking pipeline
pub struct StackingContext {
    pub stacker: Stacker,
    pub detector: StarDetector,
    pub adaptive_registration: AdaptiveRegistration,
    pub reference_stars: Vec<Star>,
    pub is_initialized: bool,
}

impl StackingContext {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        settings: &CaptureSettings,
    ) -> Option<Self> {
        // Convert weighting preset to WeightingConfig
        let weighting = match settings.weighting_preset {
            WeightingPreset::Disabled => WeightingConfig::disabled(),
            WeightingPreset::Balanced => WeightingConfig::balanced(),
            WeightingPreset::Galaxies => WeightingConfig::for_galaxies(),
            WeightingPreset::Nebulae => WeightingConfig::for_nebulae(),
            WeightingPreset::FwhmOnly => WeightingConfig::fwhm_only(),
            WeightingPreset::SnrOnly => WeightingConfig::snr_only(),
        };

        let rejection = if REJECTION_PLUGIN.get().is_some() {
            RejectionMethod::SigmaClip
        } else {
            RejectionMethod::None
        };

        let stacking_config = StackingConfig::default()
            .with_rejection(rejection)
            .with_sigma(settings.rejection_sigma)
            .with_weighting(weighting);

        let stacker = match Stacker::new(width, height, channels, stacking_config) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "Failed to create live stacker");
                return None;
            }
        };

        // Use sensitive detection for better star finding on faint images
        let detection_config = DetectionConfig::sensitive().with_max_stars(200);
        let detector = StarDetector::new(detection_config);

        // Use adaptive registration which tries multiple strategies
        let adaptive_registration = AdaptiveRegistration::new();

        Some(Self {
            stacker,
            detector,
            adaptive_registration,
            reference_stars: Vec::new(),
            is_initialized: false,
        })
    }

    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<usize, String> {
        // Try adaptive detection first for best results
        self.reference_stars = crate::detection::detect_stars_adaptive(frame)
            .map_err(|e| format!("Star detection failed: {}", e))?;

        if self.reference_stars.len() < 3 {
            return Err(format!(
                "Too few stars detected ({}) for registration, need at least 3",
                self.reference_stars.len()
            ));
        }

        // Compute quality metrics from detected stars
        let quality = FrameQuality {
            fwhm: compute_median_fwhm(&self.reference_stars),
            snr: compute_median_snr(&self.reference_stars),
        };

        self.stacker
            .add_reference_with_quality(frame, quality)
            .map_err(|e| format!("Failed to add reference frame: {}", e))?;

        self.is_initialized = true;
        Ok(self.reference_stars.len())
    }

    pub fn add_frame(&mut self, frame: &Frame) -> Result<bool, String> {
        if !self.is_initialized {
            return Err("Stacking context not initialized".to_string());
        }

        // Use adaptive detection for target frame as well
        let target_stars = match crate::detection::detect_stars_adaptive(frame) {
            Ok(stars) => stars,
            Err(_) => return Ok(false),
        };

        if target_stars.len() < 3 {
            return Ok(false);
        }

        // Use adaptive registration which tries multiple strategies for robustness
        let transform = match self
            .adaptive_registration
            .register(&self.reference_stars, &target_stars)
        {
            Ok(result) => {
                debug!(
                    config = %result.config_used,
                    matched_stars = result.matched_stars,
                    residual = result.mean_residual,
                    "Registration succeeded"
                );
                result.transform
            }
            Err(_) => return Ok(false),
        };

        // Compute quality metrics from detected stars for weighted stacking
        let quality = FrameQuality {
            fwhm: compute_median_fwhm(&target_stars),
            snr: compute_median_snr(&target_stars),
        };

        match self
            .stacker
            .add_frame_with_quality(frame, &transform, quality)
        {
            Ok(()) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub fn compute(&self) -> Result<Frame, String> {
        self.stacker
            .compute()
            .map_err(|e| format!("Failed to compute stack: {}", e))
    }

    pub fn frame_count(&self) -> usize {
        self.stacker.frame_count()
    }

    pub fn width(&self) -> usize {
        self.stacker.width()
    }

    pub fn height(&self) -> usize {
        self.stacker.height()
    }

    pub fn channels(&self) -> usize {
        self.stacker.channels()
    }

    /// Update stacking parameters from current settings dynamically
    pub fn update_from_settings(&mut self, settings: &CaptureSettings) {
        let weighting = match settings.weighting_preset {
            WeightingPreset::Disabled => WeightingConfig::disabled(),
            WeightingPreset::Balanced => WeightingConfig::balanced(),
            WeightingPreset::Galaxies => WeightingConfig::for_galaxies(),
            WeightingPreset::Nebulae => WeightingConfig::for_nebulae(),
            WeightingPreset::FwhmOnly => WeightingConfig::fwhm_only(),
            WeightingPreset::SnrOnly => WeightingConfig::snr_only(),
        };

        let rejection = settings.rejection_method;

        let config = StackingConfig::default()
            .with_rejection(rejection)
            .with_sigma(settings.rejection_sigma)
            .with_weighting(weighting);

        self.stacker.update_config(config);
    }
}

// CometStackingContext functionality is now provided by the CometPlugin trait
// and implemented in the Pro version.

/// Holds state for planetary-based live stacking pipeline
pub struct PlanetaryStackingContext {
    pub stacker: Stacker,
    pub is_initialized: bool,
    pub reference_frame: Option<Frame>,
}

impl PlanetaryStackingContext {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        settings: &CaptureSettings,
    ) -> Option<Self> {
        // Planetary stacking often uses mean or median without aggressive rejection
        // but for live stacking, SigmaClip is usually safe and effective.
        let weighting = match settings.weighting_preset {
            WeightingPreset::Disabled => WeightingConfig::disabled(),
            WeightingPreset::Balanced => WeightingConfig::balanced(),
            WeightingPreset::Galaxies => WeightingConfig::for_galaxies(),
            WeightingPreset::Nebulae => WeightingConfig::for_nebulae(),
            WeightingPreset::FwhmOnly => WeightingConfig::fwhm_only(),
            WeightingPreset::SnrOnly => WeightingConfig::snr_only(),
        };

        let rejection = if REJECTION_PLUGIN.get().is_some() {
            RejectionMethod::SigmaClip
        } else {
            RejectionMethod::None
        };

        let stacking_config = StackingConfig::default()
            .with_rejection(rejection)
            .with_sigma(settings.rejection_sigma)
            .with_weighting(weighting);

        let stacker = match Stacker::new(width, height, channels, stacking_config) {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "Failed to create live stacker for planetary mode");
                return None;
            }
        };

        Some(Self {
            stacker,
            is_initialized: false,
            reference_frame: None,
        })
    }

    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<(), String> {
        self.reference_frame = Some(frame.clone());

        // Add reference frame with default quality
        let quality = FrameQuality::default();

        self.stacker
            .add_reference_with_quality(frame, quality)
            .map_err(|e| format!("Failed to add reference frame: {}", e))?;

        self.is_initialized = true;
        Ok(())
    }

    pub fn add_frame(&mut self, frame: &Frame, settings: &CaptureSettings) -> Result<bool, String> {
        if !self.is_initialized {
            return Err("Planetary stacking context not initialized".to_string());
        }

        let reference = self.reference_frame.as_ref().unwrap();

        // Use planetary alignment logic
        let roi = settings.planetary_roi.unwrap_or_else(|| {
            let width = frame.width();
            let height = frame.height();
            let size = (width.min(height) / 2).max(64);
            AlignmentRoi::centered(width, height, size)
        });

        // Search radius and subpixel factor from planetary defaults
        let search_radius = 50;
        let subpixel_factor = 2;

        let (dx, dy) = crate::planetary::compute_alignment(
            reference,
            frame,
            &roi,
            search_radius,
            subpixel_factor,
        );

        // Convert (dx, dy) translation to AffineTransform
        // Note: Planetary alignment is currently translation-only
        let transform = crate::registration::AffineTransform::from_translation(dx, dy);

        // Compute quality (standard FWHM/SNR or planetary-specific)
        let quality = FrameQuality::default();

        match self
            .stacker
            .add_frame_with_quality(frame, &transform, quality)
        {
            Ok(()) => Ok(true),
            Err(e) => {
                debug!(error = %e, "Failed to add frame to planetary stack");
                Ok(false)
            }
        }
    }

    pub fn compute(&self) -> Result<Frame, String> {
        self.stacker
            .compute()
            .map_err(|e| format!("Failed to compute planetary stack: {}", e))
    }

    pub fn frame_count(&self) -> usize {
        self.stacker.frame_count()
    }

    pub fn width(&self) -> usize {
        self.stacker.width()
    }

    pub fn height(&self) -> usize {
        self.stacker.height()
    }

    pub fn channels(&self) -> usize {
        self.stacker.channels()
    }

    /// Update stacking parameters from current settings dynamically
    pub fn update_from_settings(&mut self, settings: &CaptureSettings) {
        let weighting = match settings.weighting_preset {
            WeightingPreset::Disabled => WeightingConfig::disabled(),
            WeightingPreset::Balanced => WeightingConfig::balanced(),
            WeightingPreset::Galaxies => WeightingConfig::for_galaxies(),
            WeightingPreset::Nebulae => WeightingConfig::for_nebulae(),
            WeightingPreset::FwhmOnly => WeightingConfig::fwhm_only(),
            WeightingPreset::SnrOnly => WeightingConfig::snr_only(),
        };

        let rejection = settings.rejection_method;

        let config = StackingConfig::default()
            .with_rejection(rejection)
            .with_sigma(settings.rejection_sigma)
            .with_weighting(weighting);

        self.stacker.update_config(config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Frame;
    use crate::planetary::AlignmentRoi;
    use crate::server::state::CaptureSettings;

    #[test]
    fn test_planetary_stacking_context_initialization() {
        let settings = CaptureSettings::default();
        let mut ctx = PlanetaryStackingContext::new(100, 100, 3, &settings).unwrap();

        let frame = Frame::zeros(100, 100, 3).unwrap();
        ctx.initialize_with_reference(&frame).unwrap();

        assert!(ctx.is_initialized);
        assert_eq!(ctx.frame_count(), 1);
    }

    #[test]
    fn test_planetary_stacking_context_add_frame() {
        let settings = CaptureSettings::default();
        let mut ctx = PlanetaryStackingContext::new(100, 100, 1, &settings).unwrap();

        // Create a reference frame with a "planet" (a square)
        let mut ref_frame = Frame::zeros(100, 100, 1).unwrap();
        for y in 40..60 {
            for x in 40..60 {
                ref_frame.set_pixel(x, y, 0, 1.0);
            }
        }
        ctx.initialize_with_reference(&ref_frame).unwrap();

        // Create a second frame shifted by (5, 3)
        let mut next_frame = Frame::zeros(100, 100, 1).unwrap();
        for y in 43..63 {
            for x in 45..65 {
                next_frame.set_pixel(x, y, 0, 1.0);
            }
        }

        let added = ctx.add_frame(&next_frame, &settings).unwrap();
        assert!(added);
        assert_eq!(ctx.frame_count(), 2);

        let stacked = ctx.compute().unwrap();
        // The stacked frame should have the square back at (40, 40)
        assert!(stacked.get_pixel(40, 40, 0) > 0.0);
    }
}
