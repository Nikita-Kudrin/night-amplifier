//! Deciding whether a frame is worth stacking. `AdaptiveRegistration` may hand back a
//! fit no better than chance (its credibility says so), so "registration succeeded"
//! alone admits coincidental correspondences that smear the stack — judged here
//! against diagnostics registration already computes. Every other limit derives from
//! the session's own frames, not a fixed constant: mount, seeing, and focal length move
//! the numbers too much (250mm dob ~0.5px vs Orion ~5.5px median residual, both
//! normal), so a frame is an outlier only relative to its neighbours.

use std::collections::VecDeque;

use night_amplifier_core::registration::{AdaptiveRegistrationResult, AffineTransform};

/// Absolute floor for the residual gate, in pixels.
///
/// Without it a run of near-perfect early frames would set a threshold so tight
/// that everything after it is rejected.
const RESIDUAL_FLOOR_PX: f32 = 1.5;

/// How far above the session's median residual a frame may sit before its
/// transform is treated as a bad fit rather than ordinary scatter.
const RESIDUAL_K: f32 = 3.0;

/// Second floor for the residual gate, as a fraction of median star size.
/// `RESIDUAL_K * median_residual` alone is scale-multiplicative and backwards: the
/// better a rig tracks, the tighter its gate — on the dumbbell fixture (0.6px
/// residual, 5.4px stars) that rule set a 1.8px limit and threw away 9/34 frames
/// at 1.9-3.3px, well inside a star's width, while the 4x-worse-tracking Orion
/// fixture rejected nothing. The floor follows the stars instead: adding half a
/// star width recovers 31/35 frames at 6.077px stacked FWHM (vs 26/5.863px with no
/// floor, 34/6.180px ungated) — most of the sharpening and most of the integration.
const RESIDUAL_FWHM_K: f32 = 0.5;

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

/// Share of the headroom above the session's usual doubled-star share (see
/// `detection::doubled_star_share`) a sub that would not register has to climb before
/// it is read as bumped or trailed rather than as the sky having moved. Headroom, not
/// a fixed step, because the share tops out at 1: clean subs sit at 0–20 % on sparse
/// fields and ~55 % on every sub of a dense one, where a fixed +50 % could never fire.
/// Every visibly doubled sub in the fixtures cleared it.
///
/// Only failed registrations are judged on it. Dropping *registered* doubled subs too
/// was measured and does not pay: 2.5 % smaller stars on the worst-tracked set for 6.6 %
/// more sky noise, and no sharper on the galaxies.
const DOUBLED_MARGIN: f32 = 0.5;

/// Consecutive unregistered, bloated subs after which they are read as the sky having
/// moved after all. Nothing that fails to register updates the session's star size, so
/// without a limit a new field with larger stars (a Barlow added, a low target in poor
/// seeing) would hold Wanderer's old stack all night. A gust or a cloud bloats a sub or
/// two; this many in a row means the stack has not grown for as long either way.
const BLOATED_RUN_LIMIT: usize = WARMUP_FRAMES;

/// Frames during which a sharper arrival can still take over as the reference.
const REBASE_WINDOW: usize = 10;

/// How much sharper a candidate must be to justify discarding the integration
/// built so far, as a fraction of the incumbent's FWHM. Held clear of the same
/// quantisation that bounds [`FWHM_K`]: at 0.85 the Orion fixture re-based on its
/// first frame — 2.52px against a 2.99px reference, one step of the area-based
/// estimator in a set that measures 2.26-2.99px throughout.
///
/// A re-base costs the integration built so far *and* drops the preview to a
/// single sub, so it must be paid for by more than the estimator's own resolution.
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
    /// A transform was fitted, but its pairs are no more than chance produces at
    /// that star density, or it changes the scale (see `Support::is_credible`).
    TooFewCorrespondences,
    /// The fit is far looser than the rest of the session's.
    ResidualTooHigh,
    /// The stars are far larger than the rest of the session's — defocus,
    /// cloud, or shake.
    StarsTooLarge,
    /// Every star appears twice, or smeared into a line: the mount moved during the
    /// exposure. Judged only on subs that would not register.
    StarsDoubled,
    /// The accumulator refused the frame.
    StackerError,
}

impl RejectionReason {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::NoStars => "star detection failed",
            Self::TooFewStars => "too few stars",
            Self::RegistrationFailed => "registration failed",
            Self::TooFewCorrespondences => "the fitted transform is no better than chance",
            Self::ResidualTooHigh => "registration residual far above the session median",
            Self::StarsTooLarge => "stars far larger than the session median",
            Self::StarsDoubled => "stars doubled or trailed — the mount moved during the exposure",
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
    /// [`RejectionReason::StarsTooLarge`] do — their fit covers most of the star
    /// field; dropping them would make the baseline self-referential (see
    /// [`QualityHistory`]). [`TooFewCorrespondences`] doesn't: its residual is a mean
    /// over whatever pairs a coincidence happened to make, on an unrelated scale
    /// (6 of 200 stars at 8.46px vs. neighbours' 1.3-2.0px) — with a 50-frame window,
    /// 26 such frames would drag the median low enough to latch the gate shut.
    fn measures_the_sky(&self) -> bool {
        matches!(self, Self::ResidualTooHigh | Self::StarsTooLarge)
    }

    /// Whether this verdict means the frame couldn't be placed against the
    /// reference at all, vs. being placed badly. What Wanderer mode watches: a user
    /// swinging a dobsonian to a new object makes the field stop matching, and the
    /// stack must restart — but a frame that *did* align, merely soft or loose from
    /// a passing cloud or gust, shouldn't throw away the integration.
    /// [`TooFewCorrespondences`] counts as "could not align": a fit chance explains
    /// means the fields don't overlap, whatever the fitter produced.
    pub fn means_the_sky_moved(&self) -> bool {
        match self {
            Self::NoStars
            | Self::TooFewStars
            | Self::RegistrationFailed
            | Self::TooFewCorrespondences => true,
            Self::ResidualTooHigh
            | Self::StarsTooLarge
            | Self::StarsDoubled
            | Self::StackerError => false,
        }
    }
}

/// What became of one frame offered to the stack.
#[derive(Debug, Clone, Copy)]
pub struct FrameAdmission {
    /// Whether the frame joined the stack.
    pub added: bool,
    /// Why it did not. `None` when it did.
    pub rejected_because: Option<RejectionReason>,
    /// Whether this frame replaced the reference, discarding prior integration.
    pub rebased: bool,
    /// Stars the fitted transform lines up within 1.5 px (`Support::pairs`); 0 if
    /// registration failed. Not the rung's own correspondence count, which on a thin sub
    /// counts nebula clutter (Cat's Eye: 47–63 against 5–13 real pairs).
    pub matched_stars: usize,
    /// Mean residual of the rung fit's correspondences, in pixels, as the gate judged
    /// it; NaN if there were none.
    pub mean_residual: f32,
    /// Where the frame landed: target -> reference coordinates. `Some` only for a frame
    /// that joined the stack.
    pub transform: Option<AffineTransform>,
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
            transform: None,
        }
    }

    pub(super) fn accepted(result: &AdaptiveRegistrationResult, rebased: bool) -> Self {
        Self {
            added: true,
            rejected_because: None,
            rebased,
            matched_stars: result.support.pairs,
            mean_residual: result.mean_residual,
            // A re-based frame *is* the new reference: it landed on itself.
            transform: Some(if rebased { AffineTransform::identity() } else { result.transform }),
        }
    }

    /// A verdict from a mode that aligns without star matching or a gate (comet,
    /// planetary): it says whether the frame joined, never why it did not.
    pub(super) fn unreasoned(added: bool) -> Self {
        Self {
            added,
            rejected_because: None,
            rebased: false,
            matched_stars: 0,
            mean_residual: f32::NAN,
            transform: None,
        }
    }
}

/// Rolling medians of the registration residual and star size seen this session.
///
/// Every frame that yields a measurement is recorded, including ones the gate
/// rejects — recording only accepted frames would be self-referential: once
/// focus or tracking degrades past the threshold, nothing is accepted, nothing
/// updates the median, and the gate rejects every remaining frame of the night.
/// Medians tolerate up to half the window as outliers, so bad bursts barely move
/// the limit while a sustained change becomes the new normal.
#[derive(Default)]
struct QualityHistory {
    residuals: VecDeque<f32>,
    fwhms: VecDeque<f32>,
    /// Doubled-star shares of every sub that had stars, registered or not: what the
    /// field looks like is measured whether or not the sub aligned.
    doubling: VecDeque<f32>,
}

impl QualityHistory {
    fn record(&mut self, residual: f32, fwhm: Option<f32>) {
        push_bounded(&mut self.residuals, residual);
        if let Some(fwhm) = fwhm {
            push_bounded(&mut self.fwhms, fwhm);
        }
    }

    fn record_doubling(&mut self, share: f32) {
        push_bounded(&mut self.doubling, share);
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

    /// The lower quartile, not the median: on a rough night most subs may be doubled,
    /// and a median would make their share the session's normal — the next bump of a
    /// field already proven to be this one would then read as movement. The quartile
    /// holds until three quarters of the window are doubled.
    fn baseline_doubling(&self) -> Option<f32> {
        (self.doubling.len() >= WARMUP_FRAMES).then(|| quantile(&self.doubling, 4)).flatten()
    }
}

fn push_bounded(values: &mut VecDeque<f32>, value: f32) {
    if values.len() == HISTORY_LEN {
        values.pop_front();
    }
    values.push_back(value);
}

fn median(values: &VecDeque<f32>) -> Option<f32> {
    quantile(values, 2)
}

/// The value `1 / divisor` of the way up the sorted values.
fn quantile(values: &VecDeque<f32>, divisor: usize) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted: Vec<f32> = values.iter().copied().collect();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(sorted[sorted.len() / divisor])
}

/// What a sub's own star list says about it, measured before registration.
#[derive(Debug, Clone, Copy)]
pub struct StarField {
    pub fwhm: Option<f32>,
    /// See `detection::doubled_star_share`.
    pub doubling: f32,
}

impl StarField {
    pub fn of(stars: &[night_amplifier_core::detection::Star]) -> Self {
        Self {
            fwhm: night_amplifier_core::detection::compute_median_fwhm(stars),
            doubling: night_amplifier_core::detection::doubled_star_share(stars),
        }
    }
}

/// Judges arriving frames against what this session has looked like so far.
#[derive(Default)]
pub struct FrameGate {
    history: QualityHistory,
    /// Sharpness of the frame the stack is currently registered against.
    reference_fwhm: Option<f32>,
    /// The reference's doubled-star share: the baseline until the session has its own.
    reference_doubling: Option<f32>,
    /// Frames offered since the stack began, counted whether or not they were
    /// accepted — this is what closes the re-basing window.
    frames_seen: usize,
    /// Consecutive subs that would not register and read as bloated. See
    /// [`BLOATED_RUN_LIMIT`].
    bloated_run: usize,
}

impl FrameGate {
    /// Notes the sharpness and doubled-star share of the frame the stack is now
    /// registered against.
    pub fn set_reference(&mut self, fwhm: Option<f32>, doubling: f32) {
        self.reference_fwhm = fwhm;
        self.reference_doubling = Some(doubling);
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

    /// Judges a frame and folds its measurements into the baseline, in that order.
    ///
    /// Both halves live here because both orderings are wrong in a different way:
    /// recording first lets a frame define the yardstick it's measured against;
    /// recording nothing lets a sustained change latch the gate shut for the rest of
    /// the night. What's recorded is everything the frame actually measured — see
    /// [`RejectionReason::measures_the_sky`].
    pub fn admit(
        &mut self,
        result: &AdaptiveRegistrationResult,
        stars: StarField,
    ) -> Option<RejectionReason> {
        let verdict = self.judge(result, stars.fwhm);

        if verdict.is_none_or(|reason| reason.measures_the_sky()) {
            self.history.record(result.mean_residual, stars.fwhm);
        }
        self.history.record_doubling(stars.doubling);
        // It registered: the field is still there.
        self.bloated_run = 0;

        verdict
    }

    /// Why a sub with stars would not register: a soft verdict when its own stars
    /// explain it, else that it could not be placed against the reference at all.
    ///
    /// Wanderer restarts the stack on the latter, so a bump or a gust that left every
    /// star doubled or bloated must not read as the telescope having been swung away —
    /// a session's bumped subs used to throw away its whole integration one by one.
    /// Doubled stars also need `is_this_field`: a dense *new* field scores as high as a
    /// bumped one against a sparse session's baseline (Orion at ~55 % after a ring
    /// session at 0 %), and only the field itself can tell the two apart. Bloated stars
    /// have no such proof, so a long enough run of them is movement after all
    /// ([`BLOATED_RUN_LIMIT`]).
    pub fn explain_unregistered(
        &mut self,
        stars: StarField,
        is_this_field: impl FnOnce() -> bool,
    ) -> RejectionReason {
        let verdict = if self.is_doubled(stars.doubling) && is_this_field() {
            RejectionReason::StarsDoubled
        } else if self.is_bloated(stars.fwhm) && self.bloated_run < BLOATED_RUN_LIMIT {
            RejectionReason::StarsTooLarge
        } else {
            RejectionReason::RegistrationFailed
        };
        self.bloated_run = match verdict {
            RejectionReason::StarsTooLarge => self.bloated_run + 1,
            _ => 0,
        };
        self.history.record_doubling(stars.doubling);
        verdict
    }

    fn is_doubled(&self, doubling: f32) -> bool {
        self.history
            .baseline_doubling()
            .or(self.reference_doubling)
            .is_some_and(|baseline| doubling >= baseline + DOUBLED_MARGIN * (1.0 - baseline))
    }

    /// Stars far larger than the session's. Needs a warmed-up history: one sub's
    /// size is no yardstick.
    fn is_bloated(&self, fwhm: Option<f32>) -> bool {
        if self.history.measured() < WARMUP_FRAMES {
            return false;
        }
        matches!((fwhm, self.history.median_fwhm()), (Some(fwhm), Some(median)) if fwhm > FWHM_K * median)
    }

    /// Returns why this frame should not be averaged into the stack, or `None`
    /// if it passes.
    fn judge(
        &self,
        result: &AdaptiveRegistrationResult,
        fwhm: Option<f32>,
    ) -> Option<RejectionReason> {
        // Not a share of the star list: a thin sub's true fit pairs 10-19 of 200
        // detections (the rest are noise), while a coincidence on a bright nebula's
        // noise maxima paired 53 — see `Support`.
        if !result.is_credible() {
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

        if self.is_bloated(fwhm) {
            return Some(RejectionReason::StarsTooLarge);
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
    /// The reference sets a hard sharpness floor on everything stacked onto it, and
    /// frame one is picked blind, so a sharper frame arriving early is worth more
    /// than the few frames of integration restarting costs. Only ever called for
    /// frames that already passed [`Self::judge`], which keeps a bogus FWHM from a
    /// noise-latched detection out.
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
#[path = "frame_gate_tests.rs"]
mod tests;
