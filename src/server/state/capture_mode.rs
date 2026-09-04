//! The capture modes, and which of them write raw frames to disk.

use serde::{Deserialize, Serialize};

/// Suffix appended to a raw session directory to record the mode that filled it.
const LIVE_VIEW_SUFFIX: &str = "-live";
const WANDERER_SUFFIX: &str = "-wanderer";
const STACKING_SUFFIX: &str = "-stacking";
const GUIDE_SUFFIX: &str = "-guide";

/// Which capture mode a session is running in.
///
/// The first three are derived from `stacking` + `wanderer_mode` rather than stored:
/// that pair is what the frontend sends (see `applyStackingMode` in
/// `CaptureControls.vue`), and this is the only place that reads it as a mode.
/// `stacking: false` is Live view whatever `wanderer_mode` says — the same reading
/// [`crate::server::state::CaptureSettings::to_capture_config`] already takes of that
/// orthogonal-misuse combination, since no frames are integrated there either.
///
/// `Guide` is the exception: it is not derivable from those flags and
/// `CaptureSettings::capture_mode` never returns it. The guide loop names its own mode,
/// because a guide camera is a *position* on the rig rather than something the imaging
/// settings can describe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureMode {
    /// Raw feed, no stacking.
    LiveView,
    /// Stacks while stationary, resets when the telescope moves.
    Wanderer,
    /// Continuous accumulation into one stack.
    Stacking,
    /// The guide camera's free-running loop. Never stacked, never a SER container.
    Guide,
}

impl CaptureMode {
    /// Suffix for this mode's raw session directory, so a night's folders say what
    /// produced them.
    pub fn session_dir_suffix(self) -> &'static str {
        match self {
            Self::LiveView => LIVE_VIEW_SUFFIX,
            Self::Wanderer => WANDERER_SUFFIX,
            Self::Stacking => STACKING_SUFFIX,
            Self::Guide => GUIDE_SUFFIX,
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
        if name.ends_with(GUIDE_SUFFIX) {
            return Some(Self::Guide);
        }
        None
    }
}

/// Which capture modes write their raw frames to disk.
///
/// Independent switches rather than one boolean plus a mode gate: the modes are not
/// alternatives to each other, they are separate occasions on which the same decision
/// applies, and an observer who wants raw subs from a Wanderer sweep does not thereby
/// want them from every focusing run. `guide` is the same idea one step further out —
/// it is an occasion that runs *alongside* the other three rather than instead of them,
/// so a night can be saving imaging subs and guide subs at once, into separate folders.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawFrameSaving {
    #[serde(default)]
    pub live_view: bool,
    #[serde(default)]
    pub wanderer: bool,
    #[serde(default)]
    pub stacking: bool,
    #[serde(default)]
    pub guide: bool,
}

impl RawFrameSaving {
    /// Whether raw frames captured in `mode` should be written.
    pub fn saves(self, mode: CaptureMode) -> bool {
        match mode {
            CaptureMode::LiveView => self.live_view,
            CaptureMode::Wanderer => self.wanderer,
            CaptureMode::Stacking => self.stacking,
            CaptureMode::Guide => self.guide,
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
            CaptureMode::Guide,
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
        assert!(!only_wanderer.saves(CaptureMode::Guide));
    }

    /// The guide switch is orthogonal to the imaging ones: a night saving guide subs
    /// must not thereby start saving imaging subs, and vice versa.
    #[test]
    fn the_guide_switch_is_independent_of_the_imaging_switches() {
        let only_guide = RawFrameSaving {
            guide: true,
            ..Default::default()
        };
        assert!(only_guide.saves(CaptureMode::Guide));
        assert!(!only_guide.saves(CaptureMode::LiveView));
        assert!(!only_guide.saves(CaptureMode::Wanderer));
        assert!(!only_guide.saves(CaptureMode::Stacking));

        let only_stacking = RawFrameSaving {
            stacking: true,
            ..Default::default()
        };
        assert!(!only_stacking.saves(CaptureMode::Guide));
    }

    #[test]
    fn nothing_is_saved_by_default() {
        for mode in [
            CaptureMode::LiveView,
            CaptureMode::Wanderer,
            CaptureMode::Stacking,
            CaptureMode::Guide,
        ] {
            assert!(!RawFrameSaving::default().saves(mode));
        }
    }
}
