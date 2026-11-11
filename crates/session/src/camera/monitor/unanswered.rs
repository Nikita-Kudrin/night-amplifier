//! Whether a parked camera has stopped answering its status reads.
//!
//! 2026-11-03: an idle Ares-C PRO read -300 °C, the SDK's failure value, for 2.5 minutes
//! before the observer pressed Start, and that Start then waited out a stall for a frame
//! the camera could not give. A run of implausible readings on a handle that once read
//! plausibly is taken as that camera, and reopened before anyone asks it for a frame. A
//! camera whose thermometer never reads plausibly is left alone, or it would be reopened
//! forever.

/// Implausible readings in a row that make a parked camera unresponsive: 10 s at the
/// monitor's 2 s poll. The night's readings stayed at -300 °C until the reopen.
#[cfg(not(test))]
pub(super) const UNANSWERED_READS: u32 = 5;
/// Two polls, so a session test sees a spell, and the one after a reopen, within seconds.
#[cfg(test)]
pub(super) const UNANSWERED_READS: u32 = 2;

/// One handle's status readings, for one monitor.
#[derive(Debug, Default)]
pub(super) struct UnansweredReads {
    answered_plausibly: bool,
    in_a_row: u32,
}

impl UnansweredReads {
    pub(super) fn plausible(&mut self) {
        self.answered_plausibly = true;
        self.in_a_row = 0;
    }

    /// Count an implausible reading. `parked`: idle or pre-cooling, the phases nothing
    /// else judges the camera in. Returns the run once it makes the camera unresponsive.
    pub(super) fn implausible(&mut self, parked: bool) -> Option<u32> {
        if !parked || !self.answered_plausibly {
            self.in_a_row = 0;
            return None;
        }
        self.in_a_row += 1;
        (self.in_a_row >= UNANSWERED_READS).then_some(self.in_a_row)
    }

    /// A spell recovery declined to reopen for: counted again only after a plausible read,
    /// so a camera left parked is reported once a spell, not at every poll.
    pub(super) fn leave_alone(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(reads: &mut UnansweredReads, implausible: u32) -> Option<u32> {
        (0..implausible).map(|_| reads.implausible(true)).last().flatten()
    }

    #[test]
    fn five_implausible_reads_after_a_plausible_one_make_the_camera_unresponsive() {
        let mut reads = UnansweredReads::default();
        reads.plausible();
        assert_eq!(after(&mut reads, UNANSWERED_READS - 1), None, "four are not enough");
        assert_eq!(reads.implausible(true), Some(UNANSWERED_READS));
    }

    #[test]
    fn a_plausible_reading_starts_the_count_again() {
        let mut reads = UnansweredReads::default();
        reads.plausible();
        after(&mut reads, UNANSWERED_READS - 1);
        reads.plausible();
        assert_eq!(after(&mut reads, UNANSWERED_READS - 1), None);
    }

    #[test]
    fn a_spell_left_alone_is_counted_again_only_after_a_plausible_read() {
        let mut reads = UnansweredReads::default();
        reads.plausible();
        assert_eq!(after(&mut reads, UNANSWERED_READS), Some(UNANSWERED_READS));
        reads.leave_alone();
        assert_eq!(after(&mut reads, 3 * UNANSWERED_READS), None, "the same spell, still going");
        reads.plausible();
        assert_eq!(after(&mut reads, UNANSWERED_READS), Some(UNANSWERED_READS), "a new spell");
    }

    #[test]
    fn a_camera_that_never_read_plausibly_is_left_alone() {
        let mut reads = UnansweredReads::default();
        assert_eq!(after(&mut reads, 3 * UNANSWERED_READS), None);
    }

    /// Warming up after a Disconnect has its own deadline, and a capture or guide loop its
    /// own stall ladder: neither is the monitor's to judge, nor counts toward it.
    #[test]
    fn readings_outside_the_parked_phases_are_not_counted() {
        let mut reads = UnansweredReads::default();
        reads.plausible();
        after(&mut reads, UNANSWERED_READS - 1);
        assert_eq!(reads.implausible(false), None);
        assert_eq!(after(&mut reads, UNANSWERED_READS - 1), None, "the run started again");
    }
}
