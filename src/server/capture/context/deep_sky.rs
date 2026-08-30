//! Star-registered deep-sky live stacking.

use tracing::{debug, field, info, info_span, instrument, warn, Span};

use crate::detection::{compute_median_fwhm, compute_median_snr, Star};
use crate::frame::Frame;
use crate::registration::AdaptiveRegistration;
use crate::server::state::CaptureSettings;
use crate::stacking::{
    FrameQuality, RejectionMethod, Stacker, StackingConfig, WeightingConfig, WeightingPreset,
    REJECTION_PLUGIN,
};

use crate::server::capture::frame_gate::{FrameAdmission, FrameGate, RejectionReason};

pub struct StackingContext {
    pub stacker: Stacker,
    pub adaptive_registration: AdaptiveRegistration,
    pub reference_stars: Vec<Star>,
    pub is_initialized: bool,
    /// Judges arriving frames against what this session has looked like so far.
    gate: FrameGate,
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
                warn!(error = %e, "Failed to create live stacker");
                return None;
            }
        };

        // Use adaptive registration which tries multiple strategies
        let adaptive_registration = AdaptiveRegistration::new();

        Some(Self {
            stacker,
            adaptive_registration,
            reference_stars: Vec::new(),
            is_initialized: false,
            gate: FrameGate::default(),
        })
    }

    #[instrument(skip(self, frame), fields(
        star_count = field::Empty,
    ))]
    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<usize, String> {
        // Try adaptive detection first for best results
        self.reference_stars = {
            let _span = info_span!("detect_stars").entered();
            crate::detection::detect_stars_adaptive(frame)
                .map_err(|e| format!("Star detection failed: {}", e))?
        };

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

        self.gate.set_reference(quality.fwhm);
        self.is_initialized = true;
        Span::current().record("star_count", self.reference_stars.len());
        Ok(self.reference_stars.len())
    }

    /// Offers one frame to the stack.
    ///
    /// A frame is admitted only if it registers *and* the fit is good enough to
    /// be worth averaging in. `AdaptiveRegistration` returns the first transform
    /// any of its presets can produce, with `robust`'s `max_residual` at 10 px —
    /// so "registration succeeded" on its own admits transforms fitted from a
    /// handful of coincidental correspondences, and averaging those is what
    /// smears the stack.
    #[instrument(skip(self, frame), fields(
        registered = field::Empty,
        matched_stars = field::Empty,
        residual = field::Empty,
    ))]
    pub fn add_frame(&mut self, frame: &Frame) -> Result<FrameAdmission, String> {
        if !self.is_initialized {
            return Err("Stacking context not initialized".to_string());
        }

        self.gate.frame_offered();

        // Use adaptive detection for target frame as well
        let target_stars = {
            let _span = info_span!("detect_stars").entered();
            match crate::detection::detect_stars_adaptive(frame) {
                Ok(stars) => stars,
                Err(_) => {
                    Span::current().record("registered", false);
                    return Ok(FrameAdmission::rejected(
                        RejectionReason::NoStars,
                        0,
                        f32::NAN,
                    ));
                }
            }
        };

        if target_stars.len() < 3 {
            Span::current().record("registered", false);
            return Ok(FrameAdmission::rejected(
                RejectionReason::TooFewStars,
                0,
                f32::NAN,
            ));
        }

        // Use adaptive registration which tries multiple strategies for robustness
        let register_result = {
            let _span = info_span!("register").entered();
            self.adaptive_registration
                .register(&self.reference_stars, &target_stars)
        };
        let result = match register_result {
            Ok(result) => {
                debug!(
                    config = %result.config_used,
                    matched_stars = result.matched_stars,
                    residual = result.mean_residual,
                    "Registration succeeded"
                );
                result
            }
            Err(_) => {
                Span::current().record("registered", false);
                return Ok(FrameAdmission::rejected(
                    RejectionReason::RegistrationFailed,
                    0,
                    f32::NAN,
                ));
            }
        };

        let span = Span::current();
        span.record("matched_stars", result.matched_stars);
        span.record("residual", result.mean_residual);

        // Compute quality metrics from detected stars for weighted stacking
        let quality = FrameQuality {
            fwhm: compute_median_fwhm(&target_stars),
            snr: compute_median_snr(&target_stars),
        };

        let verdict = self.gate.admit(
            &result,
            quality.fwhm,
            self.reference_stars.len(),
            target_stars.len(),
        );

        if let Some(reason) = verdict {
            span.record("registered", false);
            return Ok(FrameAdmission::rejected(
                reason,
                result.matched_stars,
                result.mean_residual,
            ));
        }

        // A sharper frame arriving early is worth more than the few frames of
        // integration it costs: the reference sets a hard sharpness floor on
        // everything that follows, and frame one is picked blind.
        if self.gate.should_rebase(quality.fwhm) {
            let previous = self.gate.reference_fwhm();
            self.rebase_on(frame, target_stars, quality)?;
            span.record("registered", true);
            info!(
                previous_fwhm = ?previous,
                new_fwhm = ?quality.fwhm,
                frames_discarded = self.gate.frames_seen() - 1,
                "Sharper frame arrived during warm-up, re-basing the stack on it"
            );
            return Ok(FrameAdmission::accepted(&result, true));
        }

        if let Err(e) = self
            .stacker
            .add_frame_with_quality(frame, &result.transform, quality)
        {
            warn!(error = %e, "Failed to add frame to stack");
            span.record("registered", false);
            return Ok(FrameAdmission::rejected(
                RejectionReason::StackerError,
                result.matched_stars,
                result.mean_residual,
            ));
        }

        span.record("registered", true);
        Ok(FrameAdmission::accepted(&result, false))
    }

    /// Discards the accumulated stack and starts again from this frame.
    fn rebase_on(
        &mut self,
        frame: &Frame,
        target_stars: Vec<Star>,
        quality: FrameQuality,
    ) -> Result<(), String> {
        self.stacker.clear();
        self.stacker
            .add_reference_with_quality(frame, quality)
            .map_err(|e| format!("Failed to re-base stack on new reference: {}", e))?;
        self.reference_stars = target_stars;
        self.gate.set_reference(quality.fwhm);
        Ok(())
    }

    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
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
