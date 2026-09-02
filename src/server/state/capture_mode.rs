//! The three capture modes, and which of them write raw frames to disk.

use serde::{Deserialize, Serialize};

/// Suffix appended to a raw session directory to record the mode that filled it.
const LIVE_VIEW_SUFFIX: &str = "-live";
const WANDERER_SUFFIX: &str = "-wanderer";
const STACKING_SUFFIX: &str = "-stacking";

/// Which of the three capture modes a session is running in.
///
/// Derived from `stacking` + `wanderer_mode` rather than stored: that pair is what the
/// frontend sends (see `applyStackingMode` in `CaptureControls.vue`), and this is the
/// only place that reads it as a mode. `stacking: false` is Live view whatever
/// `wanderer_mode` says — the same reading [`crate::server::state::CaptureSettings::to_capture_config`]
/// already takes of that orthogonal-misuse combination, since no frames are integrated
/// there either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Raw feed, no stacking.
    LiveView,
    /// Stacks while stationary, resets when the telescope moves.
    Wanderer,
    /// Continuous accumulation into one stack.
    Stacking,
}

impl CaptureMode {
    /// Suffix for this mode's raw session directory, so a night's folders say what
    /// produced them.
    pub fn session_dir_suffix(self) -> &'static str {
        match self {
            Self::LiveView => LIVE_VIEW_SUFFIX,
            Self::Wanderer => WANDERER_SUFFIX,
            Self::Stacking => STACKING_SUFFIX,
        }
    }

    /// Read the mode back out of a raw session directory name.
    ///
    /// The directory name is the only record of which mode opened a session — nothing
    /// else outlives the capture — so a mid-session mode change compares against this
    /// rather than against a second copy of the mode held in memory that could drift
    /// from the folder actually being written to.
    pub fn from_session_dir_name(name: &str) -> Option<Self> {
        if name.ends_with(LIVE_VIEW_SUFFIX) {
            return Some(Self::LiveView);
        }
        if name.ends_with(WANDERER_SUFFIX) {
            return Some(Self::Wanderer);
        }
        if name.ends_with(STACKING_SUFFIX) {
            return Some(Self::Stacking);
        }
        None
    }
}

/// Which capture modes write their raw frames to disk.
///
/// Three independent switches rather than one boolean plus a mode gate: the modes are
/// not alternatives to each other, they are three separate occasions on which the same
/// decision applies, and an observer who wants raw subs from a Wanderer sweep does not
/// thereby want them from every focusing run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFrameSaving {
    #[serde(default)]
    pub live_view: bool,
    #[serde(default)]
    pub wanderer: bool,
    #[serde(default)]
    pub stacking: bool,
}

impl RawFrameSaving {
    /// Whether raw frames captured in `mode` should be written.
    pub fn saves(self, mode: CaptureMode) -> bool {
        match mode {
            CaptureMode::LiveView => self.live_view,
            CaptureMode::Wanderer => self.wanderer,
            CaptureMode::Stacking => self.stacking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every suffix must survive a round trip through the directory name, or a
    /// mid-session mode change compares a mode against a folder it cannot classify and
    /// rolls the session on every settings update.
    #[test]
    fn every_mode_round_trips_through_its_directory_suffix() {
        for mode in [
            CaptureMode::LiveView,
            CaptureMode::Wanderer,
            CaptureMode::Stacking,
        ] {
            let name = format!("02-09-2026_21-14-08{}", mode.session_dir_suffix());
            assert_eq!(
                CaptureMode::from_session_dir_name(&name),
                Some(mode),
                "{name} did not read back as {mode:?}"
            );
        }
    }

    /// A second session inside the same wall-clock second is disambiguated with a
    /// counter, which `DiskWriterHandle::start_session` puts *before* the suffix
    /// precisely so the name still ends with the mode it names.
    #[test]
    fn a_disambiguated_directory_still_names_its_mode() {
        assert_eq!(
            CaptureMode::from_session_dir_name("02-09-2026_21-14-08_2-stacking"),
            Some(CaptureMode::Stacking)
        );
    }

    /// A directory that predates the suffixes (or one a user renamed) classifies as
    /// nothing rather than guessing, so the caller can leave it alone.
    #[test]
    fn an_unsuffixed_directory_names_no_mode() {
        assert_eq!(
            CaptureMode::from_session_dir_name("02-09-2026_21-14-08"),
            None
        );
    }

    #[test]
    fn saves_reads_the_switch_belonging_to_the_mode() {
        let only_wanderer = RawFrameSaving {
            wanderer: true,
            ..Default::default()
        };
        assert!(only_wanderer.saves(CaptureMode::Wanderer));
        assert!(!only_wanderer.saves(CaptureMode::LiveView));
        assert!(!only_wanderer.saves(CaptureMode::Stacking));
    }

    #[test]
    fn nothing_is_saved_by_default() {
        for mode in [
            CaptureMode::LiveView,
            CaptureMode::Wanderer,
            CaptureMode::Stacking,
        ] {
            assert!(!RawFrameSaving::default().saves(mode));
        }
    }
}
