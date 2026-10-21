//! Star-registered deep-sky live stacking.

use tracing::{debug, field, info, info_span, instrument, warn, Span};

use super::probation::{Commit, Probation};
use super::{LiveStackError, LiveStacker, StackSettings, MIN_REFERENCE_STARS};
use night_amplifier_core::detection::{
    collapse_doubles, compute_median_fwhm, compute_median_snr, doubled_star_share,
    dominant_companion_offset, typical_fwhm, Star,
};
use night_amplifier_core::frame::{Frame, NoiseField};
use night_amplifier_core::registration::{AdaptiveRegistration, AffineTransform};
use night_amplifier_core::stacking::{FrameQuality, Stacker, StackingType};

use crate::capture::frame_gate::{median, push_bounded, FrameAdmission, FrameGate, RejectionReason, Restacked, StarField};

pub struct StackingContext {
    pub stacker: Stacker,
    pub adaptive_registration: AdaptiveRegistration,
    pub reference_stars: Vec<Star>,
    pub is_initialized: bool,
    /// Judges arriving frames against what this session has looked like so far.
    gate: FrameGate,
    /// The last stacked sub's transform against `reference_stars`: an alt-az field keeps
    /// turning against the fixed reference, and registration searches near it.
    last_transform: Option<AffineTransform>,
    /// Each sub's [`typical_fwhm`], registered or not: the size of a star on screen, which
    /// the renderer holds Detail off by. Not the gate's FWHM, which drops the widest stars.
    star_widths: std::collections::VecDeque<f32>,
    /// The stack's first subs, held until they can be judged; `None` once committed.
    probation: Option<Probation>,
    /// The reference is elected from a probation, so no later sub re-bases it: the
    /// re-base exists because sub 1 was picked blind, and once discarded 7 subs at sub 8.
    elected: bool,
}

impl StackingContext {
    pub fn new(
        width: usize,
        height: usize,
        channels: usize,
        settings: &StackSettings,
    ) -> Result<Self, LiveStackError> {
        // Built the same way `update_from_settings` builds it, so a session does not
        // start on a different method than a no-op settings edit would give it. This
        // used to hardcode `SigmaClip`, which meant the observer's choice only took
        // effect if they happened to touch settings mid-session.
        let config = settings.config.clone();
        let stacker =
            Stacker::with_plugins(width, height, channels, config, settings.plugins.clone())?;

        // Use adaptive registration which tries multiple strategies
        let adaptive_registration = AdaptiveRegistration::new();
        let probation = Probation::for_stack(settings.probation_subs, width * height * channels);

        Ok(Self {
            stacker,
            adaptive_registration,
            reference_stars: Vec::new(),
            is_initialized: false,
            gate: FrameGate::default(),
            last_transform: None,
            star_widths: Default::default(),
            elected: probation.is_some(),
            probation,
        })
    }

    #[instrument(skip(self, frame), fields(
        star_count = field::Empty,
    ))]
    pub fn initialize_with_reference(&mut self, frame: &Frame) -> Result<usize, LiveStackError> {
        // Try adaptive detection first for best results
        self.reference_stars = {
            let _span = info_span!("detect_stars").entered();
            night_amplifier_core::detection::detect_stars_adaptive(frame)?
        };

        if self.reference_stars.len() < MIN_REFERENCE_STARS {
            return Err(LiveStackError::TooFewStars {
                found: self.reference_stars.len(),
            });
        }
        self.record_star_width(typical_fwhm(&self.reference_stars));

        // Compute quality metrics from detected stars
        let quality = FrameQuality {
            fwhm: compute_median_fwhm(&self.reference_stars),
            snr: compute_median_snr(&self.reference_stars),
        };

        self.stacker.add_reference_with_quality(frame, quality)?;

        self.gate
            .set_reference(quality.fwhm, doubled_star_share(&self.reference_stars));
        self.last_transform = None;
        self.is_initialized = true;
        if let Some(probation) = &mut self.probation {
            probation.offered();
            probation.hold(frame, self.reference_stars.clone(), quality, true);
        }
        Span::current().record("star_count", self.reference_stars.len());
        Ok(self.reference_stars.len())
    }

    fn record_star_width(&mut self, width: Option<f32>) {
        if let Some(width) = width {
            push_bounded(&mut self.star_widths, width);
        }
    }

    /// Offers one frame to the stack.
    ///
    /// A frame is admitted only if it registers *and* the fit is good enough to
    /// be worth averaging in. `AdaptiveRegistration` hands back a fit chance can
    /// explain when it finds no better, so "registration succeeded" on its own
    /// admits coincidental correspondences, and averaging those is what smears the
    /// stack.
    #[instrument(skip(self, frame), fields(
        registered = field::Empty,
        matched_stars = field::Empty,
        residual = field::Empty,
    ))]
    pub fn add_frame(&mut self, frame: &Frame) -> Result<FrameAdmission, LiveStackError> {
        if !self.is_initialized {
            return Err(LiveStackError::NoReference);
        }

        self.gate.frame_offered();

        // Use adaptive detection for target frame as well
        let target_stars = {
            let _span = info_span!("detect_stars").entered();
            match night_amplifier_core::detection::detect_stars_adaptive(frame) {
                Ok(stars) => stars,
                Err(_) => {
                    Span::current().record("registered", false);
                    let starless = FrameAdmission::rejected(RejectionReason::NoStars, 0, f32::NAN);
                    return self.settle(starless, false);
                }
            }
        };

        if target_stars.len() < MIN_REFERENCE_STARS {
            Span::current().record("registered", false);
            let starless = FrameAdmission::rejected(RejectionReason::TooFewStars, 0, f32::NAN);
            return self.settle(starless, false);
        }
        self.record_star_width(typical_fwhm(&target_stars));

        let admission = self.place(frame, &target_stars)?;
        let Some(probation) = &mut self.probation else {
            return Ok(admission);
        };
        let quality = FrameQuality {
            fwhm: compute_median_fwhm(&target_stars),
            snr: compute_median_snr(&target_stars),
        };
        probation.hold(frame, target_stars, quality, admission.added);
        self.settle(admission, true)
    }

    /// Registers and judges a sub with stars, and stacks it if it passes.
    fn place(&mut self, frame: &Frame, target_stars: &[Star]) -> Result<FrameAdmission, LiveStackError> {
        let field = StarField::of(target_stars);

        let register_result = {
            let _span = info_span!("register").entered();
            self.adaptive_registration.register_near(
                &self.reference_stars,
                target_stars,
                self.last_transform.as_ref(),
            )
        };
        let result = match register_result {
            Ok(result) => {
                debug!(
                    config = %result.config_used,
                    matched_stars = result.support.pairs,
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
                // A bumped sub of this field registers once each star is one image
                // again; a different field does not.
                let (registration, reference) = (&self.adaptive_registration, &self.reference_stars);
                let prior = self.last_transform.as_ref();
                let is_this_field = || {
                    dominant_companion_offset(target_stars)
                        .map(|offset| collapse_doubles(target_stars, offset))
                        .is_some_and(|single| {
                            registration
                                .register_near(reference, &single, prior)
                                .is_ok_and(|result| result.is_credible())
                        })
                };
                return Ok(FrameAdmission::rejected(
                    self.gate.explain_unregistered(field, is_this_field),
                    0,
                    f32::NAN,
                ));
            }
        };

        let span = Span::current();
        span.record("matched_stars", result.support.pairs);
        // field::debug, not the bare f32: see the "Registration succeeded" debug!
        // above — mean_residual can be non-finite and Jaeger's query API 500s on a
        // non-finite double attribute.
        span.record("residual", field::debug(result.mean_residual));

        // Compute quality metrics from detected stars for weighted stacking
        let quality = FrameQuality {
            fwhm: field.fwhm,
            snr: compute_median_snr(target_stars),
        };

        let verdict = self.gate.admit(&result, field);

        if let Some(reason) = verdict {
            span.record("registered", false);
            return Ok(FrameAdmission::rejected(
                reason,
                result.support.pairs,
                result.mean_residual,
            ));
        }

        // A sharper frame arriving early is worth more than the few frames of
        // integration it costs: the reference sets a hard sharpness floor on
        // everything that follows, and frame one is picked blind. A probation elects
        // its reference from the whole batch instead, and keeps what this discards.
        if !self.elected && self.gate.should_rebase(quality.fwhm) {
            let previous = self.gate.reference_fwhm();
            self.rebase_on(frame, target_stars.to_vec(), quality, field.doubling)?;
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
                result.support.pairs,
                result.mean_residual,
            ));
        }

        self.last_transform = Some(result.transform);
        span.record("registered", true);
        Ok(FrameAdmission::accepted(&result, false))
    }

    /// Counts a sub against the probation and commits it once it is due, folding the
    /// rebuilt stack's verdicts into this sub's admission. `held`: this sub is the
    /// probation's newest.
    fn settle(&mut self, admission: FrameAdmission, held: bool) -> Result<FrameAdmission, LiveStackError> {
        let Some(probation) = &mut self.probation else {
            return Ok(admission);
        };
        probation.offered();
        if !probation.is_due() {
            return Ok(admission);
        }
        let probation = self.probation.take().expect("checked above");
        let Some(commit) = probation.decide(&self.adaptive_registration, self.gate.frames_seen()) else {
            // Nothing was elected, so sub 1 is still the blind pick the re-base exists for.
            self.elected = false;
            info!(held = probation.held().len(), "No held sub registers most of the others: the stack stays as it streamed");
            return Ok(admission);
        };
        Ok(self.commit(probation, commit, admission, held))
    }

    fn commit(&mut self, probation: Probation, commit: Commit, admission: FrameAdmission, held: bool) -> FrameAdmission {
        let subs = probation.held();
        let elected = &subs[commit.reference];
        let (reference_stars, elected_fwhm) = (elected.stars.clone(), elected.field.fwhm);
        let streamed: Vec<bool> = subs.iter().map(|sub| sub.stacked).collect();
        let (mut frames, transforms, qualities) = probation.into_kept(&commit);
        // Refused before the accumulator is touched (`MasterStack::restack`), so the stack
        // stays as it streamed, on a reference nobody elected.
        if let Err(error) = self.stacker.restack(&mut frames, &transforms, &qualities) {
            warn!(%error, "Probation not committed: the stack stays as it streamed");
            self.elected = false;
            return admission;
        }

        let admission = fold_verdicts(admission, &commit, &streamed, held);
        let latest = commit.placed.iter().rev().flatten().next().copied();
        self.last_transform = latest.filter(|t| *t != AffineTransform::identity());
        self.reference_stars = reference_stars;
        self.gate = commit.gate;
        info!(
            held = streamed.len(),
            kept = frames.len(),
            reference = commit.reference + 1,
            reference_fwhm = ?elected_fwhm,
            restacked = ?admission.restacked,
            "Probation committed: stack rebuilt on the elected reference"
        );
        admission
    }

    /// Discards the accumulated stack and starts again from this frame.
    fn rebase_on(
        &mut self,
        frame: &Frame,
        target_stars: Vec<Star>,
        quality: FrameQuality,
        doubling: f32,
    ) -> Result<(), LiveStackError> {
        self.stacker.clear();
        self.stacker.add_reference_with_quality(frame, quality)?;
        self.reference_stars = target_stars;
        self.last_transform = None;
        self.gate.set_reference(quality.fwhm, doubling);
        Ok(())
    }

    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
    pub fn compute(&self) -> Result<Frame, LiveStackError> {
        Ok(self.stacker.compute()?)
    }

    /// The stacked result and its coverage map, from one read of the accumulator.
    #[instrument(skip(self), fields(frame_count = self.frame_count()))]
    pub fn compute_with_coverage(&self) -> Result<(Frame, NoiseField), LiveStackError> {
        Ok(self.stacker.compute_with_coverage()?)
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

/// Folds a commit's verdicts into the admission of the sub that closed it: that sub's own
/// fate when it is the probation's newest (`held`), and the earlier subs the rebuilt stack
/// took back out or let in after all (`streamed`: each held sub's place as it streamed).
fn fold_verdicts(mut admission: FrameAdmission, commit: &Commit, streamed: &[bool], held: bool) -> FrameAdmission {
    let mut restacked = Restacked { rereferenced: commit.reference != 0, ..Restacked::default() };
    let newest = streamed.len() - 1;
    for (i, &stacked) in streamed.iter().enumerate() {
        let kept = commit.placed[i].is_some();
        if held && i == newest {
            admission.added = kept;
            admission.rejected_because = commit.reasons[i];
            admission.transform = commit.placed[i];
            continue;
        }
        match (stacked, kept) {
            (true, false) => restacked.retracted += 1,
            (false, true) => restacked.reinstated += 1,
            _ => {}
        }
    }
    admission.restacked = Some(restacked);
    admission
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

    fn star_fwhm(&self) -> Option<f32> {
        median(&self.star_widths)
    }

    fn apply_settings(&mut self, settings: &StackSettings) {
        self.update_from_settings(settings);
    }

    fn set_reference(&mut self, frame: &Frame) -> Result<(), LiveStackError> {
        let star_count = self.initialize_with_reference(frame)?;
        info!(star_count, "Stacking initialized with reference frame");
        Ok(())
    }

    fn offer(
        &mut self,
        frame: &Frame,
        _settings: &StackSettings,
    ) -> Result<FrameAdmission, LiveStackError> {
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
    fn snapshot(&self) -> Result<(Frame, Option<NoiseField>), LiveStackError> {
        let (frame, coverage) = self.compute_with_coverage()?;
        Ok((frame, coverage.is_usable().then_some(coverage)))
    }
}

#[cfg(test)]
#[path = "deep_sky_tests.rs"]
mod tests;
