//! Wanderer mode: restarting the stack when the telescope has been moved.
//!
//! A sub that cannot be placed against the reference is the signal, but one such sub is
//! not proof: a thin session's fits sit close to the credibility bar (the Cat's Eye's
//! weakest at −7.7 against −6), so a single marginal sub used to wipe the integration.
//! Movement is confirmed by [`CONFIRMING_SUBS`] in a row, at the cost of one sub of
//! latency on a real slew.

use super::frame_gate::RejectionReason;

/// Consecutive "moved" subs that restart the stack.
const CONFIRMING_SUBS: usize = 2;

/// The run of subs that could not be placed against the reference.
#[derive(Debug, Default)]
pub(super) struct Wanderer {
    moved_run: usize,
}

impl Wanderer {
    /// Whether this sub confirms the telescope moved, so the stack must restart. A sub
    /// that does not read as movement ends the run.
    pub(super) fn confirms_movement(
        &mut self,
        wanderer_mode: bool,
        stacking_enabled: bool,
        registration_succeeded: bool,
        rejected_because: Option<RejectionReason>,
    ) -> bool {
        if !detected_movement(wanderer_mode, stacking_enabled, registration_succeeded, rejected_because) {
            self.moved_run = 0;
            return false;
        }
        self.moved_run += 1;
        if self.moved_run < CONFIRMING_SUBS {
            return false;
        }
        self.moved_run = 0;
        true
    }

    /// Drops the run: the stack it was counted against is gone.
    pub(super) fn forget(&mut self) {
        self.moved_run = 0;
    }
}

/// Whether this sub reads as the user having moved the telescope. Only a frame that
/// couldn't be placed against the reference counts — before the frame gate existed,
/// every rejection meant that, but the gate also rejects frames that aligned fine yet
/// were soft or loose, and resetting on those would restart the stack every time a
/// cloud crosses. A mode reporting no reason (comet, planetary) keeps the original
/// behaviour: not stacking is the only signal available.
fn detected_movement(
    wanderer_mode: bool,
    stacking_enabled: bool,
    registration_succeeded: bool,
    rejected_because: Option<RejectionReason>,
) -> bool {
    if !wanderer_mode || !stacking_enabled || registration_succeeded {
        return false;
    }
    rejected_because.is_none_or(|reason| reason.means_the_sky_moved())
}

#[cfg(test)]
mod tests {
    use super::detected_movement as moved;
    use super::*;

    const LOST: Option<RejectionReason> = Some(RejectionReason::TooFewCorrespondences);

    #[test]
    fn wanderer_resets_when_the_frame_cannot_be_placed_at_all() {
        for reason in [
            RejectionReason::NoStars,
            RejectionReason::TooFewStars,
            RejectionReason::RegistrationFailed,
            RejectionReason::TooFewCorrespondences,
        ] {
            assert!(
                moved(true, true, false, Some(reason)),
                "{reason:?} means the field no longer matches the reference"
            );
        }
    }

    /// The regression the frame gate introduced: it rejects frames that aligned
    /// perfectly well but were soft or loose, and Wanderer read every rejection
    /// as the user having swung the scope. A cloud crossing would restart the
    /// stack, which is the opposite of what the mode is for.
    #[test]
    fn wanderer_holds_the_stack_through_a_cloud() {
        for reason in [
            RejectionReason::ResidualTooHigh,
            RejectionReason::StarsTooLarge,
            RejectionReason::StackerError,
        ] {
            assert!(
                !moved(true, true, false, Some(reason)),
                "{reason:?} is a bad frame, not a new target"
            );
        }
    }

    #[test]
    fn wanderer_leaves_a_stacked_frame_alone() {
        assert!(!moved(true, true, true, None));
    }

    /// Comet and planetary report no reason, so "did not stack" stays the only
    /// signal available to them.
    #[test]
    fn a_mode_without_reasons_keeps_the_original_wanderer_behaviour() {
        assert!(moved(true, true, false, None));
    }

    #[test]
    fn wanderer_does_nothing_when_it_is_off_or_stacking_is_not_running() {
        assert!(!moved(false, true, false, None), "wanderer mode is off");
        assert!(!moved(true, false, false, None), "stacking is not running");
    }

    /// A slew: every sub from it on is lost, and the second confirms it.
    #[test]
    fn a_second_lost_sub_in_a_row_restarts_the_stack() {
        let mut wanderer = Wanderer::default();

        assert!(!wanderer.confirms_movement(true, true, false, LOST), "one lost sub is not proof");
        assert!(wanderer.confirms_movement(true, true, false, LOST));
        assert!(!wanderer.confirms_movement(true, true, false, LOST), "the restart starts a new run");
    }

    /// A thin session's marginal sub between two that stack is not a slew.
    #[test]
    fn a_lone_lost_sub_holds_the_stack() {
        let mut wanderer = Wanderer::default();

        for _ in 0..5 {
            assert!(!wanderer.confirms_movement(true, true, false, LOST));
            assert!(!wanderer.confirms_movement(true, true, true, None));
        }
    }

    /// A soft sub is not movement either, so it ends a run as a stacked one does.
    #[test]
    fn a_soft_sub_ends_the_run() {
        let mut wanderer = Wanderer::default();

        assert!(!wanderer.confirms_movement(true, true, false, LOST));
        assert!(!wanderer.confirms_movement(true, true, false, Some(RejectionReason::ResidualTooHigh)));
        assert!(!wanderer.confirms_movement(true, true, false, LOST));
    }

    #[test]
    fn a_forgotten_run_starts_again() {
        let mut wanderer = Wanderer::default();

        assert!(!wanderer.confirms_movement(true, true, false, LOST));
        wanderer.forget();
        assert!(!wanderer.confirms_movement(true, true, false, LOST));
    }
}
