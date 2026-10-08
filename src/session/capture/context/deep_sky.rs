//! Star-registered deep-sky live stacking.

use tracing::{debug, field, info, info_span, instrument, warn, Span};

use super::{LiveStacker, StackSettings};
use crate::detection::{compute_median_fwhm, compute_median_snr, Star};
use crate::frame::{Frame, NoiseField};
use crate::registration::AdaptiveRegistration;
use crate::stacking::{FrameQuality, Stacker, StackingType};

use crate::session::capture::frame_gate::{FrameAdmission, FrameGate, RejectionReason};

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
        settings: &StackSettings,
    ) -> Option<Self> {
        // Built the same way `update_from_settings` builds it, so a session does not
        // start on a different method than a no-op settings edit would give it. This
        // used to hardcode `SigmaClip`, which meant the observer's choice only took
        // effect if they happened to touch settings mid-session.
        let config = settings.config.clone();
        let stacker = match Stacker::with_plugins(width, height, channels, config, settings.plugins.clone()) {
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
                    // Debug, not the bare f32: mean_residual is f32::INFINITY when
                    // registration matched but no correspondences survived the
                    // diagnostic pass (see AdaptiveRegistration::compute_diagnostics).
                    // A non-finite double attribute makes Jaeger's query API 500 on
                    // any trace search that includes it; Debug records it as a string.
                    residual = ?result.mean_residual,
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
        // field::debug, not the bare f32: see the "Registration succeeded" debug!
        // above — mean_residual can be non-finite and Jaeger's query API 500s on a
        // non-finite double attribute.
        span.record("residual", field::debug(result.mean_residual));

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

    /// The stacked result and its coverage map, from one read of the accumulator.
    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
    pub fn compute_with_coverage(&self) -> Result<(Frame, crate::frame::NoiseField), String> {
        self.stacker
            .compute_with_coverage()
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
    pub fn update_from_settings(&mut self, settings: &StackSettings) {
        self.stacker.update_config(settings.config.clone());
    }
}

impl LiveStacker for StackingContext {
    fn kind(&self) -> StackingType {
        StackingType::DeepSky
    }

    fn geometry(&self) -> (usize, usize, usize) {
        (self.width(), self.height(), self.channels())
    }

    fn depth(&self) -> usize {
        self.frame_count()
    }

    fn has_reference(&self) -> bool {
        self.is_initialized
    }

    fn apply_settings(&mut self, settings: &StackSettings) {
        self.update_from_settings(settings);
    }

    fn set_reference(&mut self, frame: &Frame) -> Result<(), String> {
        let star_count = self.initialize_with_reference(frame)?;
        info!(star_count, "Stacking initialized with reference frame");
        Ok(())
    }

    fn offer(&mut self, frame: &Frame, _settings: &StackSettings) -> Result<FrameAdmission, String> {
        let admission = self.add_frame(frame)?;
        // `residual` as Debug, not the bare f32: NaN/inf are legitimate sentinels here
        // (see `FrameAdmission::mean_residual`), and OTel exports a bare f32 as a double
        // attribute that Jaeger's query API 500s trying to JSON-encode — taking down every
        // trace search that touches one. Debug makes it a string attribute instead.
        match admission.rejected_because {
            None => info!(
                frame_count = self.frame_count(),
                matched_stars = admission.matched_stars,
                residual = ?admission.mean_residual,
                "Frame added to stack"
            ),
            Some(reason) => info!(
                frame_count = self.frame_count(),
                matched_stars = admission.matched_stars,
                residual = ?admission.mean_residual,
                reason = reason.describe(),
                "Frame not added to stack"
            ),
        }
        Ok(admission)
    }

    /// One read of the 434 MB accumulator for both the display copy and the coverage
    /// map; see `MasterStack::compute_with_coverage`. A stack every sub covered
    /// completely — the common case — carries no map, so it costs the encoders nothing.
    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), String> {
        let (frame, coverage) = self.compute_with_coverage()?;
        Ok((frame, coverage.is_usable().then_some(coverage)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::state::CaptureSettings;
    use crate::stacking::{RejectionMethod, WeightingPreset};

    /// Starting a session and editing settings mid-session must agree.
    ///
    /// They did not: `new` hardcoded `SigmaClip` from licence state while
    /// `update_from_settings` read `settings.rejection_method`, so the observer's choice
    /// only took effect if they happened to touch settings after capture began. Compared
    /// through the config the accumulator actually ends up running, not through the
    /// resolver both now call — going through the resolver twice would pass however the
    /// two paths were wired.
    #[test]
    fn session_start_and_a_settings_edit_configure_the_stack_alike() {
        for method in [RejectionMethod::None, RejectionMethod::SigmaClip] {
            for (preset, sigma) in [
                (WeightingPreset::Balanced, 2.5),
                (WeightingPreset::Nebulae, 1.8),
                (WeightingPreset::Disabled, 3.2),
            ] {
                let mut settings = CaptureSettings::default();
                settings.rejection_method = method;
                settings.weighting_preset = preset;
                settings.rejection_sigma = sigma;
                let settings = StackSettings::of(&settings, &crate::plugins::Plugins::none());

                let mut at_start =
                    StackingContext::new(64, 64, 3, &settings).expect("context builds");
                let from_start = at_start.stacker.config().clone();

                at_start.update_from_settings(&settings);
                let after_edit = at_start.stacker.config();

                assert_eq!(from_start.rejection, after_edit.rejection, "{method:?}");
                assert_eq!(from_start.sigma_low, after_edit.sigma_low);
                assert_eq!(from_start.sigma_high, after_edit.sigma_high);
                assert_eq!(from_start.weighting, after_edit.weighting, "{preset:?}");
            }
        }
    }
}
