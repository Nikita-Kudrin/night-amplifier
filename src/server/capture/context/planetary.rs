//! Correlation-aligned planetary live stacking.

use tracing::{debug, field, info_span, instrument, warn, Span};

use crate::frame::Frame;
use crate::planetary::AlignmentRoi;
use crate::server::state::CaptureSettings;
use crate::stacking::{
    FrameQuality, RejectionMethod, Stacker, StackingConfig, WeightingConfig, WeightingPreset,
    REJECTION_PLUGIN,
};

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

        let rejection = if crate::license::pro_plugin(&REJECTION_PLUGIN).is_some() {
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

    #[instrument(skip(self, frame))]
    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<(), String> {
        self.reference_frame = Some(frame.clone());

        if let Some(plugin) = crate::license::pro_plugin(&crate::planetary::PLANETARY_PLUGIN) {
            plugin.clear_cache();
        }

        // Add reference frame with default quality
        let quality = FrameQuality::default();

        self.stacker
            .add_reference_with_quality(frame, quality)
            .map_err(|e| format!("Failed to add reference frame: {}", e))?;

        self.is_initialized = true;
        Ok(())
    }

    #[instrument(skip(self, frame, settings), fields(
        dx = field::Empty,
        dy = field::Empty,
        ncc = field::Empty,
        registered = field::Empty,
    ))]
    pub fn add_frame(&mut self, frame: &Frame, settings: &CaptureSettings) -> Result<bool, String> {
        if !self.is_initialized {
            return Err("Planetary stacking context not initialized".to_string());
        }

        let reference = self.reference_frame.as_ref().unwrap();

        // Use planetary alignment logic
        let mut roi = settings.planetary_roi.unwrap_or_else(|| {
            let width = frame.width();
            let height = frame.height();
            let size = (width.min(height) / 2).max(64);
            AlignmentRoi::centered(width, height, size)
        });

        if settings.planetary_auto_tracking {
            let lum = crate::planetary::frame_to_luminance(frame);
            let (cx, cy) = crate::planetary::compute_centroid(&lum, frame.width(), frame.height());
            let (base_w, base_h) = match settings.planetary_roi {
                Some(ref r) => (r.width, r.height),
                None => {
                    let size = (frame.width().min(frame.height()) / 2).max(64);
                    (size, size)
                }
            };
            roi = AlignmentRoi::centered_at(cx, cy, base_w, base_h, frame.width(), frame.height());
        }

        // Search radius and subpixel factor from planetary defaults
        let search_radius = 50;
        let subpixel_factor = 2;

        // Try multi-point alignment via Pro plugin, falling back to single-point correlation.
        let warped_frame: Option<Frame> = if settings.planetary_multi_point_alignment {
            crate::license::pro_plugin(&crate::planetary::PLANETARY_PLUGIN).and_then(|plugin| {
                match plugin.warp_frame(frame, reference, &roi, search_radius) {
                    Ok(warped) => Some(warped),
                    Err(e) => {
                        debug!(error = %e, "Multi-point planetary alignment failed, falling back");
                        None
                    }
                }
            })
        } else {
            None
        };

        let transform = if warped_frame.is_some() {
            Span::current().record("dx", 0.0);
            Span::current().record("dy", 0.0);
            Span::current().record("ncc", 1.0);
            crate::registration::AffineTransform::from_translation(0.0, 0.0)
        } else {
            let (dx, dy, ncc) = crate::planetary::compute_alignment(
                reference,
                frame,
                &roi,
                search_radius,
                subpixel_factor,
            );

            debug!(dx, dy, ncc, "Planetary alignment results");
            Span::current().record("dx", dx);
            Span::current().record("dy", dy);
            Span::current().record("ncc", ncc);

            crate::registration::AffineTransform::from_translation(dx, dy)
        };

        let final_frame = warped_frame.as_ref().unwrap_or(frame);

        // Compute quality (standard FWHM/SNR or planetary-specific)
        let quality = FrameQuality::default();

        match self
            .stacker
            .add_frame_with_quality(final_frame, &transform, quality)
        {
            Ok(()) => {
                Span::current().record("registered", true);
                Ok(true)
            }
            Err(e) => {
                debug!(error = %e, "Failed to add frame to planetary stack");
                Span::current().record("registered", false);
                Ok(false)
            }
        }
    }

    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
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
