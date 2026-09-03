//! Deciding whether a frame is worth stacking. `AdaptiveRegistration` returns the
//! first transform any preset can fit (`robust`'s `max_residual` is 10px), so
//! "registration succeeded" alone admits transforms built from a handful of
//! coincidental correspondences — averaging those in smears the stack, so the
//! diagnostics registration already computes are judged here first.
//!
//! Every limit derives from the session's own frames, not a fixed constant: mount,
//! seeing and focal length move the numbers too much (the 250mm dob fixture
//! registers at ~0.5px median residual, the 250mm Orion set at ~5.5px, both normal).
//! A frame is an outlier relative to its neighbours, or not an outlier at all.

use std::collections::VecDeque;

use crate::registration::AdaptiveRegistrationResult;

/// Absolute floor for the residual gate, in pixels.
///
/// Without it a run of near-perfect early frames would set a threshold so tight
/// that everything after it is rejected.
const RESIDUAL_FLOOR_PX: f32 = 1.5;

/// How far above the session's median residual a frame may sit before its
/// transform is treated as a bad fit rather than ordinary scatter.
const RESIDUAL_K: f32 = 3.0;

/// Second floor for the residual gate, as a fraction of the session's median star
/// size. `RESIDUAL_K * median_residual` alone is scale-multiplicative and gets it
/// backwards: the better a rig tracks, the tighter its own gate becomes — on the
/// 250mm dumbbell fixture (0.6px median residual, 5.4px stars) that rule set a
/// 1.8px limit and threw away 9 of 34 frames whose residuals (1.9-3.3px) were well
/// inside a single star's width, while the 4x-worse-tracking Orion fixture rejected
/// nothing. Misalignment only matters relative to the PSF it smears, so the floor
/// follows the stars: measured on the dumbbell fixture, adding half a star width
/// recovers 31 of 35 frames at 6.077px stacked FWHM (vs 26 frames/5.863px with no
/// floor, 34/6.180px ungated) — keeping most of the sharpening and most of the
/// integration.
const RESIDUAL_FWHM_K: f32 = 0.5;

/// Fraction of the smaller star list that must correspond for a fit to be
/// trusted. A transform derived from a handful of stars out of two hundred is a
/// coincidence, not an alignment.
const MATCH_FRACTION: f32 = 0.25;

/// How far above the session's median star size a frame may sit before it's treated
/// as defocused, clouded, or shaken rather than merely soft. Bounded below by how
/// well star size can be measured: `compute_fwhm` derives width from an integer
/// pixel count above half maximum, quantised in ~10% steps at the sharp end, and the
/// median over a changing star field moves further still (the 250mm dumbbell fixture
/// spans 1.60-7.57px around a 5.4px median while residuals hold at 0.6px). At 1.35
/// this gate rejected frame 17 (7.57px) while admitting neighbours at 6.82/6.48px —
/// a verdict on the estimator, not the sky.
const FWHM_K: f32 = 1.8;

/// Measured frames needed before the running medians mean anything. Until then
/// every registered frame is admitted.
const WARMUP_FRAMES: usize = 5;

/// Frames the running medians are computed over. Bounded so the gate tracks
/// seeing as it drifts through the night instead of averaging over a session
/// that no longer resembles the current sky.
const HISTORY_LEN: usize = 50;

/// Frames during which a sharper arrival can still take over as the reference.
const REBASE_WINDOW: usize = 10;

/// How much sharper a candidate must be to justify discarding the integration
/// built so far, as a fraction of the incumbent's FWHM.
///
/// Held clear of the same quantisation that bounds [`FWHM_K`]. At 0.85 the Orion
/// fixture re-based on its first frame — 2.52 px against a 2.99 px reference,
/// one step of the area-based estimator, in a set whose every frame measures
/// between 2.26 and 2.99 px. A re-base costs the integration built so far *and*
/// drops the preview back to a single sub, so it has to be paid for by more than
/// the estimator's own resolution.
const REBASE_MARGIN: f32 = 0.75;

/// Below this fraction of the running median FWHM, an implausibly "sharp" frame
/// is star detection latching onto noise, not a sharp frame.
const REBASE_MIN_RATIO: f32 = 0.6;

/// Why a frame was kept out of the stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionReason {
    /// Star detection found nothing usable.
    NoStars,
    /// Too few stars to attempt an alignment.
    TooFewStars,
    /// No preset could fit a transform at all.
    RegistrationFailed,
    /// A transform was fitted, but from too small a share of the star field to
    /// be more than a coincidence.
    TooFewCorrespondences,
    /// The fit is far looser than the rest of the session's.
    ResidualTooHigh,
    /// The stars are far larger than the rest of the session's — defocus,
    /// cloud, or shake.
    StarsTooLarge,
    /// The accumulator refused the frame.
    StackerError,
}

impl RejectionReason {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::NoStars => "star detection failed",
            Self::TooFewStars => "too few stars",
            Self::RegistrationFailed => "registration failed",
            Self::TooFewCorrespondences => "too few correspondences for the fitted transform",
            Self::ResidualTooHigh => "registration residual far above the session median",
            Self::StarsTooLarge => "stars far larger than the session median",
            Self::StackerError => "stacker rejected the frame",
        }
    }

    /// Whether this verdict was reached by judging how well the frame aligned,
    /// as opposed to how it looked or whether it aligned at all.
    pub fn is_about_alignment_quality(&self) -> bool {
        matches!(self, Self::ResidualTooHigh | Self::TooFewCorrespondences)
    }

    /// Whether a frame carrying this verdict still measured the sky well enough to
    /// belong in the running medians. [`RejectionReason::ResidualTooHigh`] and
    /// [`RejectionReason::StarsTooLarge`] do — their numbers come from a fit over
    /// most of the star field, and dropping them would make the baseline
    /// self-referential (see [`QualityHistory`]). [`TooFewCorrespondences`] doesn't:
    /// its residual is a mean over the handful of pairs the fit selected for itself,
    /// on an unrelated scale (observed: 6 of 200 stars at 8.46px against neighbours'
    /// 1.3-2.0px) — with a 50-frame window, 26 such frames would drag the median low
    /// enough to latch the gate shut against every good frame after.
    fn measures_the_sky(&self) -> bool {
        matches!(self, Self::ResidualTooHigh | Self::StarsTooLarge)
    }

    /// Whether this verdict means the frame couldn't be placed against the
    /// reference at all, vs. being placed badly. What Wanderer mode watches: a user
    /// swinging a dobsonian to a new object makes the field stop matching, and the
    /// stack must restart — but a frame that *did* align, merely soft or loose from
    /// a passing cloud or gust, shouldn't throw away the integration.
    /// [`TooFewCorrespondences`] counts as "could not align": agreement on a
    /// handful of two hundred stars means the fields don't overlap, whatever the
    /// fitter produced.
    pub fn means_the_sky_moved(&self) -> bool {
        match self {
            Self::NoStars
            | Self::TooFewStars
            | Self::RegistrationFailed
            | Self::TooFewCorrespondences => true,
            Self::ResidualTooHigh | Self::StarsTooLarge | Self::StackerError => false,
        }
    }
}

/// What became of one frame offered to the stack.
pub struct FrameAdmission {
    /// Whether the frame joined the stack.
    pub added: bool,
    /// Why it did not. `None` when it did.
    pub rejected_because: Option<RejectionReason>,
    /// Whether this frame replaced the reference, discarding prior integration.
    pub rebased: bool,
    /// Correspondences found for the fitted transform; 0 if registration failed.
    pub matched_stars: usize,
    /// Mean residual of those correspondences, in pixels; NaN if there were none.
    pub mean_residual: f32,
}

impl FrameAdmission {
    pub(super) fn rejected(
        reason: RejectionReason,
        matched_stars: usize,
        mean_residual: f32,
    ) -> Self {
        Self {
            added: false,
            rejected_because: Some(reason),
            rebased: false,
            matched_stars,
            mean_residual,
        }
    }

    pub(super) fn accepted(result: &AdaptiveRegistrationResult, rebased: bool) -> Self {
        Self {
            added: true,
            rejected_because: None,
            rebased,
            matched_stars: result.matched_stars,
            mean_residual: result.mean_residual,
        }
    }
}

/// Rolling medians of the registration residual and star size seen this session.
///
/// Every frame that yields a measurement is recorded, including ones the gate
/// then rejects. Recording only accepted frames would make the baseline
/// self-referential: if focus drifted or tracking degraded past the threshold,
/// nothing would be accepted, so nothing would update the median, and the gate
/// would reject every remaining frame of the night. Medians tolerate up to half
/// the window being outliers, so a burst of bad frames barely moves the limit
/// while a sustained change in conditions correctly becomes the new normal.
#[derive(Default)]
struct QualityHistory {
    residuals: VecDeque<f32>,
    fwhms: VecDeque<f32>,
}

impl QualityHistory {
    fn record(&mut self, residual: f32, fwhm: Option<f32>) {
        push_bounded(&mut self.residuals, residual);
        if let Some(fwhm) = fwhm {
            push_bounded(&mut self.fwhms, fwhm);
        }
    }

    fn measured(&self) -> usize {
        self.residuals.len()
    }

    fn median_residual(&self) -> Option<f32> {
        median(&self.residuals)
    }

    fn median_fwhm(&self) -> Option<f32> {
        median(&self.fwhms)
    }
}

fn push_bounded(values: &mut VecDeque<f32>, value: f32) {
    if values.len() == HISTORY_LEN {
        values.pop_front();
    }
    values.push_back(value);
}

fn median(values: &VecDeque<f32>) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f32> = values.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(sorted[sorted.len() / 2])
}

/// Judges arriving frames against what this session has looked like so far.
#[derive(Default)]
pub struct FrameGate {
    history: QualityHistory,
    /// Sharpness of the frame the stack is currently registered against.
    reference_fwhm: Option<f32>,
    /// Frames offered since the stack began, counted whether or not they were
    /// accepted — this is what closes the re-basing window.
    frames_seen: usize,
}

impl FrameGate {
    /// Notes the sharpness of the frame the stack is now registered against.
    pub fn set_reference(&mut self, fwhm: Option<f32>) {
        self.reference_fwhm = fwhm;
    }

    /// Counts a frame arriving, accepted or not.
    pub fn frame_offered(&mut self) {
        self.frames_seen += 1;
    }

    /// Sharpness of the frame the stack is currently registered against.
    pub fn reference_fwhm(&self) -> Option<f32> {
        self.reference_fwhm
    }

    /// Frames offered since the stack began.
    pub fn frames_seen(&self) -> usize {
        self.frames_seen
    }

    /// Judges a frame and folds its measurements into the baseline, in that
    /// order.
    ///
    /// Both halves live here because both orderings are wrong in a different
    /// way. Recording first lets a frame help define the yardstick it is
    /// measured against; recording nothing lets a sustained change in conditions
    /// latch the gate shut for the rest of the night. What is recorded is
    /// everything the frame actually measured — see
    /// [`RejectionReason::measures_the_sky`].
    pub fn admit(
        &mut self,
        result: &AdaptiveRegistrationResult,
        fwhm: Option<f32>,
        reference_stars: usize,
        target_stars: usize,
    ) -> Option<RejectionReason> {
        let verdict = self.judge(result, fwhm, reference_stars, target_stars);

        if verdict.is_none_or(|reason| reason.measures_the_sky()) {
            self.history.record(result.mean_residual, fwhm);
        }

        verdict
    }

    /// Returns why this frame should not be averaged into the stack, or `None`
    /// if it passes.
    fn judge(
        &self,
        result: &AdaptiveRegistrationResult,
        fwhm: Option<f32>,
        reference_stars: usize,
        target_stars: usize,
    ) -> Option<RejectionReason> {
        let pool = reference_stars.min(target_stars) as f32;
        if (result.matched_stars as f32) < MATCH_FRACTION * pool {
            return Some(RejectionReason::TooFewCorrespondences);
        }

        // Until there is history to compare against, a registered frame is the
        // best evidence available.
        if self.history.measured() < WARMUP_FRAMES {
            return None;
        }

        if let Some(median) = self.history.median_residual() {
            if result.mean_residual > self.residual_limit(median) {
                return Some(RejectionReason::ResidualTooHigh);
            }
        }

        if let (Some(fwhm), Some(median)) = (fwhm, self.history.median_fwhm()) {
            if fwhm > FWHM_K * median {
                return Some(RejectionReason::StarsTooLarge);
            }
        }

        None
    }

    /// How loose a fit this session will still average in.
    ///
    /// Three floors, whichever is highest: an absolute one so a run of
    /// near-perfect early frames cannot pin the gate shut, a multiple of the
    /// session's own scatter, and a fraction of the session's star size — the
    /// last because a misalignment only matters against the width of what it is
    /// smearing.
    fn residual_limit(&self, median_residual: f32) -> f32 {
        let limit = RESIDUAL_FLOOR_PX.max(RESIDUAL_K * median_residual);
        match self.history.median_fwhm() {
            Some(median_fwhm) => limit.max(RESIDUAL_FWHM_K * median_fwhm),
            None => limit,
        }
    }

    /// Whether this frame is sharp enough, and early enough, to become the new
    /// reference.
    ///
    /// The reference sets a hard sharpness floor on everything stacked onto it
    /// and frame one is picked blind, so a sharper frame arriving early is worth
    /// more than the few frames of integration restarting costs.
    ///
    /// Only ever called for frames that already passed [`Self::judge`], which is
    /// what keeps a bogus FWHM from a noise-latched detection out.
    pub fn should_rebase(&self, fwhm: Option<f32>) -> bool {
        if self.frames_seen > REBASE_WINDOW {
            return false;
        }

        let (Some(fwhm), Some(reference_fwhm)) = (fwhm, self.reference_fwhm) else {
            return false;
        };

        if fwhm > REBASE_MARGIN * reference_fwhm {
            return false;
        }

        // Without history to compare against, take the measurement at face value
        // — the gate has already vouched for the registration.
        let Some(median) = self.history.median_fwhm() else {
            return true;
        };

        // An implausibly small measurement is star detection finding noise, not
        // a sharper frame.
        if fwhm < REBASE_MIN_RATIO * median {
            return false;
        }

        // The incumbent's FWHM is one noisy sample, so beating it is not on its
        // own evidence of a sharper frame — the candidate has to beat what this
        // session typically looks like by the same margin. That is what
        // separates the two re-bases in the bundled fixtures: the dumbbell set's
        // frame 2 at 4.37 px against a 6.28 px session is a real change of
        // sharpness, while the Orion set's frame 1 at 2.52 px against a 2.99 px
        // reference is a session that measures 2.26–2.99 px throughout.
        fwhm <= REBASE_MARGIN * median
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::AffineTransform;

    /// Fills the gate's history so it is past warm-up, with residuals centred on
    /// `residual` and star sizes on `fwhm`.
    fn seeded(residual: f32, fwhm: f32) -> FrameGate {
        let mut gate = FrameGate::default();
        for _ in 0..WARMUP_FRAMES + 3 {
            gate.history.record(residual, Some(fwhm));
        }
        gate
    }

    fn fit(matched_stars: usize, mean_residual: f32) -> AdaptiveRegistrationResult {
        AdaptiveRegistrationResult {
            transform: AffineTransform::identity(),
            matched_stars,
            mean_residual,
            config_used: "test".to_string(),
            attempts: 1,
        }
    }

    #[test]
    fn admits_a_clean_fit() {
        let gate = seeded(0.5, 5.0);
        assert_eq!(gate.judge(&fit(180, 0.6), Some(5.0), 200, 200), None);
    }

    #[test]
    fn rejects_a_fit_built_from_a_handful_of_stars() {
        let gate = seeded(0.5, 5.0);
        // A transform agreeing with 7 of 200 stars is a coincidence, however
        // small its residual over those seven.
        assert_eq!(
            gate.judge(&fit(7, 0.4), Some(5.0), 200, 200),
            Some(RejectionReason::TooFewCorrespondences)
        );
    }

    #[test]
    fn rejects_a_residual_far_above_the_session_median() {
        let gate = seeded(0.5, 5.0);
        assert_eq!(
            gate.judge(&fit(180, 20.0), Some(5.0), 200, 200),
            Some(RejectionReason::ResidualTooHigh)
        );
    }

    #[test]
    fn follows_a_loose_session_rather_than_a_fixed_idea_of_good() {
        // The 250mm Orion fixture registers with a ~5.5 px median residual
        // throughout. A fixed threshold would reject every frame in it; the
        // point of scoring against the session's own median is that a frame is
        // only an outlier relative to its neighbours.
        let gate = seeded(5.5, 5.0);
        assert_eq!(gate.judge(&fit(180, 6.5), Some(5.0), 200, 200), None);
        assert_eq!(
            gate.judge(&fit(180, 20.0), Some(5.0), 200, 200),
            Some(RejectionReason::ResidualTooHigh)
        );
    }

    /// The other half of the same idea, and the one a median-only rule gets
    /// backwards: a rig that tracks *well* must not end up with the strictest
    /// gate. The 250 mm dumbbell fixture holds a 0.6 px median residual on 5.4 px
    /// stars, where `RESIDUAL_K * median` alone allows only 1.8 px and threw away
    /// 9 of its 34 frames for residuals of 1.9–3.3 px — a fraction of one star's
    /// width.
    #[test]
    fn a_well_tracked_session_is_not_punished_for_its_own_precision() {
        let gate = seeded(0.6, 5.4);

        for residual in [1.9, 2.4, 2.7] {
            assert_eq!(
                gate.judge(&fit(150, residual), Some(5.4), 200, 200),
                None,
                "{residual} px is a fraction of a 5.4 px star and must still stack"
            );
        }

        // Past half a star width it is smearing, whatever the session median.
        assert_eq!(
            gate.judge(&fit(150, 8.2), Some(5.4), 200, 200),
            Some(RejectionReason::ResidualTooHigh)
        );
    }

    /// The star-size floor tracks the session rather than sitting at a constant:
    /// the same residual is fine on fat stars and smearing on tight ones.
    #[test]
    fn the_star_size_floor_follows_the_session_not_a_constant() {
        assert_eq!(
            seeded(0.6, 8.0).judge(&fit(150, 3.5), Some(8.0), 200, 200),
            None,
            "3.5 px is well inside an 8 px star"
        );
        assert_eq!(
            seeded(0.6, 2.5).judge(&fit(150, 3.5), Some(2.5), 200, 200),
            Some(RejectionReason::ResidualTooHigh),
            "3.5 px is wider than a 2.5 px star"
        );
    }

    #[test]
    fn rejects_bloated_stars() {
        let gate = seeded(0.5, 4.0);
        assert_eq!(
            gate.judge(&fit(180, 0.5), Some(9.0), 200, 200),
            Some(RejectionReason::StarsTooLarge)
        );
    }

    /// `compute_fwhm` counts whole pixels above half maximum, so star size is
    /// quantised and its median wanders frame to frame even on a stable night:
    /// the 250 mm dumbbell fixture spans 1.60–7.57 px around a 5.4 px median
    /// while its residuals hold at 0.6 px. A threshold inside that spread rejects
    /// the estimator, not the sky — at 1.35 that set lost its frame 17 (7.57 px)
    /// while keeping neighbours at 6.82 and 6.48 px.
    #[test]
    fn ordinary_scatter_in_measured_star_size_is_not_defocus() {
        let gate = seeded(0.6, 5.4);

        for fwhm in [6.5, 7.6, 9.0] {
            assert_eq!(
                gate.judge(&fit(150, 0.6), Some(fwhm), 200, 200),
                None,
                "{fwhm} px is inside the spread a 5.4 px session measures"
            );
        }

        // Twice the session's star size is defocus, cloud, or shake.
        assert_eq!(
            gate.judge(&fit(150, 0.6), Some(11.0), 200, 200),
            Some(RejectionReason::StarsTooLarge)
        );
    }

    #[test]
    fn admits_a_registered_frame_during_warmup() {
        let mut gate = FrameGate::default();
        gate.history.record(0.5, Some(4.0));
        // Nothing to compare against yet, so a wide residual still counts.
        assert_eq!(gate.judge(&fit(180, 9.0), Some(12.0), 200, 200), None);
    }

    /// A gate keyed only on accepted frames would latch shut the moment
    /// conditions moved past its threshold: nothing accepted means nothing
    /// recorded, means the median never catches up. Recording every measured
    /// frame lets a sustained change become the new normal.
    #[test]
    fn a_sustained_change_in_conditions_reopens_the_gate() {
        let mut gate = seeded(0.4, 4.0);
        assert!(gate
            .admit(&fit(180, 9.0), Some(4.0), 200, 200)
            .is_some());

        for _ in 0..HISTORY_LEN {
            if gate.admit(&fit(180, 9.0), Some(4.0), 200, 200).is_none() {
                return;
            }
        }
        panic!("gate never reopened after conditions settled at a new level");
    }

    /// A frame rejected for having too few correspondences must not move the
    /// baseline. Its residual is a mean over the handful of pairs the fit chose
    /// for itself — the dumbbell fixture produced one at 8.46 px over 6 of 200
    /// stars, in a set whose other frames sit at 1.3–2.0 px. With the median at
    /// `sorted[HISTORY_LEN / 2]`, a run of them redefines what the gate calls
    /// normal.
    #[test]
    fn a_coincidental_fit_does_not_move_the_baseline() {
        let mut gate = seeded(0.6, 5.4);
        let before = gate.history.median_residual();

        for _ in 0..HISTORY_LEN {
            assert_eq!(
                gate.admit(&fit(4, 0.05), Some(5.4), 200, 200),
                Some(RejectionReason::TooFewCorrespondences)
            );
        }

        assert_eq!(
            gate.history.median_residual(),
            before,
            "a fit the gate called a coincidence redefined the session"
        );
        assert_eq!(
            gate.judge(&fit(150, 2.4), Some(5.4), 200, 200),
            None,
            "the gate latched shut against a frame it admitted before the burst"
        );
    }

    /// The verdicts that *are* measurements still have to land, or the gate
    /// cannot follow a night that genuinely changes.
    #[test]
    fn a_frame_rejected_on_its_own_measurements_still_updates_the_baseline() {
        let mut gate = seeded(0.6, 5.4);
        let before = gate.history.median_residual();

        for _ in 0..HISTORY_LEN {
            gate.admit(&fit(150, 12.0), Some(5.4), 200, 200);
        }

        assert!(
            gate.history.median_residual() > before,
            "a sustained rise in residual never reached the baseline"
        );
    }

    #[test]
    fn a_sharper_frame_takes_over_as_reference_early_on() {
        let mut gate = FrameGate::default();
        gate.set_reference(Some(6.0));
        gate.frames_seen = 3;
        assert!(gate.should_rebase(Some(4.0)));
    }

    #[test]
    fn a_marginally_sharper_frame_is_not_worth_the_integration() {
        let mut gate = FrameGate::default();
        gate.set_reference(Some(6.0));
        gate.frames_seen = 3;
        assert!(!gate.should_rebase(Some(5.5)));
    }

    #[test]
    fn the_reference_settles_once_the_window_closes() {
        let mut gate = FrameGate::default();
        gate.set_reference(Some(6.0));
        gate.frames_seen = REBASE_WINDOW + 1;
        assert!(!gate.should_rebase(Some(2.0)));
    }

    #[test]
    fn an_implausibly_sharp_frame_is_detection_noise_not_a_new_reference() {
        let mut gate = seeded(0.5, 5.0);
        gate.set_reference(Some(6.0));
        gate.frames_seen = 3;
        // 1.6 px against a 5.0 px session median is star detection latching onto
        // noise; the dumbbell fixture's frame 11 reports exactly this.
        assert!(!gate.should_rebase(Some(1.6)));
        assert!(gate.should_rebase(Some(3.5)));
    }

    /// Beating the incumbent is not enough: the reference's own FWHM is a single
    /// noisy sample, so a candidate one quantisation step below it is evidence of
    /// nothing. This is the difference between the two re-bases the bundled
    /// fixtures produce — the Orion set's 2.52 px against a 2.99 px reference in a
    /// session that measures 2.26–2.99 px throughout, and the dumbbell set's
    /// 4.37 px against a 6.28 px session.
    #[test]
    fn beating_only_a_noisy_reference_is_not_worth_a_rebase() {
        let mut gate = seeded(0.6, 2.7);
        gate.set_reference(Some(2.99));
        gate.frames_seen = 1;
        assert!(
            !gate.should_rebase(Some(2.52)),
            "one step of the area-based estimator is not a sharper frame"
        );

        // 6.2 rather than the fixture's exact 6.283 — that is tau, and clippy
        // reads the literal as a mis-typed constant.
        let mut gate = seeded(0.6, 6.2);
        gate.set_reference(Some(6.2));
        gate.frames_seen = 2;
        assert!(
            gate.should_rebase(Some(4.37)),
            "30% sharper than the whole session is a real change"
        );
    }
}
