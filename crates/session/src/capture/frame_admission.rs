//! What became of a frame offered to the stack, and why one was kept out.

use night_amplifier_core::registration::{AdaptiveRegistrationResult, AffineTransform};

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
    /// One of a stack's first subs whose stars line up on the elected reference far
    /// worse than the others' do: cloud, a bump, a trail (see `context::probation`).
    Outvoted,
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
            Self::Outvoted => "its stars line up far worse than the session's other first subs",
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
    /// `QualityHistory`). [`TooFewCorrespondences`] doesn't: its residual is a mean
    /// over whatever pairs a coincidence happened to make, on an unrelated scale
    /// (6 of 200 stars at 8.46px vs. neighbours' 1.3-2.0px) — with a 50-frame window,
    /// 26 such frames would drag the median low enough to latch the gate shut.
    pub(in crate::capture) fn measures_the_sky(&self) -> bool {
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
            | Self::StackerError
            | Self::Outvoted => false,
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
    /// `Some` when this frame closed a probation and the stack was rebuilt from it.
    pub restacked: Option<Restacked>,
}

/// What a probation's commit did to the stack (see `context::probation`). Its pixels
/// changed whatever the counts say: set-wide weights, and Pro's batch rejection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Restacked {
    /// Earlier subs the rebuilt stack took back out.
    pub retracted: usize,
    /// Earlier subs it let in after all.
    pub reinstated: usize,
    /// The rebuilt stack sits on another reference, so its pixels moved.
    pub rereferenced: bool,
}

impl FrameAdmission {
    pub(in crate::capture) fn rejected(
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
            restacked: None,
        }
    }

    pub(in crate::capture) fn accepted(result: &AdaptiveRegistrationResult, rebased: bool) -> Self {
        Self {
            added: true,
            rejected_because: None,
            rebased,
            matched_stars: result.support.pairs,
            mean_residual: result.mean_residual,
            // A re-based frame *is* the new reference: it landed on itself.
            transform: Some(if rebased { AffineTransform::identity() } else { result.transform }),
            restacked: None,
        }
    }

    /// A verdict from a mode that aligns without star matching or a gate (comet,
    /// planetary): it says whether the frame joined, never why it did not.
    pub(in crate::capture) fn unreasoned(added: bool) -> Self {
        Self {
            added,
            rejected_because: None,
            rebased: false,
            matched_stars: 0,
            mean_residual: f32::NAN,
            transform: None,
            restacked: None,
        }
    }
}
