//! The stall ladder's first rung as the capture loops use it: whether a stalled frame
//! restarts the stream or reopens the camera, and what the loop saw around it.
//!
//! One entry point for the imaging loop, its first-frame probe and the guide loop, so the
//! three agree on the verdict, the log line and the fault record.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use crate::server::camera_health::{self, FaultKind};
use crate::server::state::{AppState, CameraPhase, CameraRole};

/// Consecutive stalled frames a loop restarts the stream for in place before it
/// treats the camera as faulted and hands it to the reconnect supervisor.
pub(crate) const STALL_ESCALATION: u32 = 3;

/// What a capture loop should do about a stalled frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StallVerdict {
    /// The shim already stopped the stream; the next `capture()` restarts it.
    RestartInPlace,
    /// Reopen the device, for the reason given.
    Escalate(EscalationReason),
}

/// Why a stall reopens the camera instead of restarting its stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscalationReason {
    /// `STALL_ESCALATION` stalls in a row: restarting in place did not bring a frame back.
    Run,
    /// Restarting in place has failed this camera `failed` times in a row lately, so its
    /// first stall skips the restart. See `camera_health::RestartHistory`.
    RestartsNotRecovering { failed: u32 },
}

/// Counts stalled frames between good ones, shared by the imaging and guide loops.
///
/// A single stall is not a fault — the camera answered, it just lost a frame — so it
/// must not reach `camera_health`, which would count it toward "persistently
/// unresponsive". A run of them is. What each in-place restart achieved is recorded
/// against the camera, so the loop after a reopen can skip restarts that do not work.
#[derive(Default)]
pub(crate) struct StallTracker {
    consecutive: u32,
    /// The last verdict restarted the stream; the next capture says whether that worked.
    restart_pending: bool,
    /// Where this camera's restart outcomes are kept. `None` judges runs alone.
    ledger: Option<(Arc<AppState>, CameraRole, String)>,
    frames: u64,
    last_frame_at: Option<Instant>,
}

impl StallTracker {
    pub(crate) fn for_camera(state: &Arc<AppState>, role: CameraRole, camera_name: &str) -> Self {
        Self {
            ledger: Some((Arc::clone(state), role, camera_name.to_string())),
            ..Self::default()
        }
    }

    pub(crate) fn frame_delivered(&mut self) {
        self.settle_pending_restart(true);
        self.consecutive = 0;
        self.frames += 1;
        self.last_frame_at = Some(Instant::now());
    }

    pub(crate) fn stalled(&mut self) -> StallVerdict {
        self.settle_pending_restart(false);
        self.consecutive += 1;
        if self.consecutive >= STALL_ESCALATION {
            self.consecutive = 0;
            return StallVerdict::Escalate(EscalationReason::Run);
        }
        let distrusted = self
            .ledger
            .as_ref()
            .and_then(|(state, role, name)| camera_health::distrusted_restarts(state, *role, name));
        if let Some(failed) = distrusted {
            self.consecutive = 0;
            return StallVerdict::Escalate(EscalationReason::RestartsNotRecovering { failed });
        }
        self.restart_pending = true;
        StallVerdict::RestartInPlace
    }

    fn settle_pending_restart(&mut self, recovered: bool) {
        if !std::mem::take(&mut self.restart_pending) {
            return;
        }
        if let Some((state, role, name)) = &self.ledger {
            camera_health::record_restart_outcome(state, *role, name, recovered);
        }
    }
}

/// Which loop saw the stall: it picks the log line, and whose camera is "the other one".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StallSite {
    /// The imaging loop, mid-session.
    Main,
    /// The imaging loop's first frame after a (re)open.
    FirstFrame,
    /// The guide camera's loop.
    Guide,
}

impl StallSite {
    fn role(self) -> CameraRole {
        match self {
            Self::Main | Self::FirstFrame => CameraRole::Main,
            Self::Guide => CameraRole::Guide,
        }
    }
}

/// Turn a stalled frame into the loop's next step: decide, log it with its context,
/// record the fault when it escalates, and hand back the verdict.
///
/// The messages are the ones field logs have always carried, so a grep for "restarting
/// the stream in place" against "reopening the camera" still tells the rungs apart.
pub(crate) fn handle_stall(
    stalls: &mut StallTracker,
    site: StallSite,
    state: &Arc<AppState>,
    camera_name: &str,
    budget: Duration,
) -> StallVerdict {
    let verdict = stalls.stalled();
    let context = StallContext::gather(state, site.role(), stalls);
    match (verdict, site) {
        (StallVerdict::RestartInPlace, StallSite::Guide) => warn!(
            camera = %camera_name, ?budget, %context,
            "Guide frame stalled; restarting the stream in place"
        ),
        (StallVerdict::RestartInPlace, StallSite::Main) => warn!(
            camera_name = %camera_name, ?budget, %context,
            "Frame stalled; restarting the stream in place"
        ),
        (StallVerdict::RestartInPlace, StallSite::FirstFrame) => warn!(
            camera_name = %camera_name, ?budget, %context,
            "First frame stalled; restarting the stream in place"
        ),
        (StallVerdict::Escalate(EscalationReason::Run), StallSite::Guide) => error!(
            camera = %camera_name, consecutive = STALL_ESCALATION, %context,
            "Restarting the guide stream did not bring frames back; reopening the camera"
        ),
        (StallVerdict::Escalate(EscalationReason::Run), StallSite::Main) => error!(
            camera_name = %camera_name, consecutive = STALL_ESCALATION, %context,
            "Restarting the stream did not bring frames back; reopening the camera"
        ),
        (StallVerdict::Escalate(EscalationReason::Run), StallSite::FirstFrame) => error!(
            camera_name = %camera_name, consecutive = STALL_ESCALATION, %context,
            "Restarting the stream did not bring the first frame; reopening the camera"
        ),
        (StallVerdict::Escalate(EscalationReason::RestartsNotRecovering { failed }), _) => error!(
            camera_name = %camera_name, role = site.role().label(), failed_restarts = failed,
            ?budget, %context,
            "Frame stalled and restarting in place has not recovered this camera lately; reopening the camera"
        ),
    }
    if let StallVerdict::Escalate(_) = verdict {
        camera_health::record_fault(state, camera_name, FaultKind::Timeout);
    }
    verdict
}

/// What surrounded a stalled frame: enough to tell a camera that stops under load from
/// one that stops on its own. The 2026-09-14 log had 145 guide stalls and none of this,
/// so what set them off could only be inferred by lining up unrelated lines.
struct StallContext {
    frames: u64,
    since_last_frame: Option<Duration>,
    solve_in_flight: Option<bool>,
    other_camera: Option<CameraPhase>,
    host: HostLoad,
}

impl StallContext {
    /// Never waits: the loops are not async, and a lock someone holds reads as unknown.
    fn gather(state: &AppState, role: CameraRole, stalls: &StallTracker) -> Self {
        Self {
            frames: stalls.frames,
            since_last_frame: stalls.last_frame_at.map(|at| at.elapsed()),
            solve_in_flight: state
                .push_to
                .try_read()
                .ok()
                .and_then(|push_to| push_to.as_ref().map(|pt| pt.is_solving())),
            other_camera: state.slot(role.other()).phase.try_read().ok().map(|phase| *phase),
            host: HostLoad::sample(),
        }
    }
}

impl fmt::Display for StallContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "frames={}", self.frames)?;
        field(
            f,
            "since_last_frame",
            self.since_last_frame.map(|d| format!("{:.1}s", d.as_secs_f32())),
        )?;
        field(f, "solve_in_flight", self.solve_in_flight)?;
        field(f, "other_camera", self.other_camera.map(|phase| format!("{phase:?}")))?;
        field(f, "load_1m", self.host.load_1m)?;
        field(f, "mem_available_mib", self.host.mem_available_mib)?;
        field(f, "soc_temp_c", self.host.soc_temp_c)
    }
}

fn field<T: fmt::Display>(f: &mut fmt::Formatter<'_>, name: &str, value: Option<T>) -> fmt::Result {
    match value {
        Some(value) => write!(f, " {name}={value}"),
        None => write!(f, " {name}=unknown"),
    }
}

/// Board load at the moment of a stall. Linux only; elsewhere every field reads unknown.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct HostLoad {
    load_1m: Option<f32>,
    mem_available_mib: Option<u64>,
    soc_temp_c: Option<f32>,
}

impl HostLoad {
    #[cfg(target_os = "linux")]
    fn sample() -> Self {
        let read = |path| std::fs::read_to_string(path).ok();
        Self {
            load_1m: read("/proc/loadavg").as_deref().and_then(parse_load_1m),
            mem_available_mib: read("/proc/meminfo").as_deref().and_then(parse_mem_available_mib),
            soc_temp_c: crate::system_info::cpu_temperature().map(|(_, celsius)| celsius),
        }
    }

    #[cfg(not(target_os = "linux"))]
    fn sample() -> Self {
        Self::default()
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_load_1m(loadavg: &str) -> Option<f32> {
    loadavg.split_whitespace().next()?.parse().ok()
}

#[cfg(any(target_os = "linux", test))]
fn parse_mem_available_mib(meminfo: &str) -> Option<u64> {
    super::channel::parse_meminfo_bytes(meminfo, "MemAvailable").map(|bytes| bytes as u64 / (1024 * 1024))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::services::PushToState;

    #[test]
    fn the_context_line_says_unknown_rather_than_guessing() {
        let context = StallContext {
            frames: 2,
            since_last_frame: Some(Duration::from_millis(4_340)),
            solve_in_flight: None,
            other_camera: Some(CameraPhase::Disconnected),
            host: HostLoad {
                load_1m: Some(3.5),
                mem_available_mib: None,
                soc_temp_c: Some(61.25),
            },
        };
        assert_eq!(
            context.to_string(),
            "frames=2 since_last_frame=4.3s solve_in_flight=unknown other_camera=Disconnected \
             load_1m=3.5 mem_available_mib=unknown soc_temp_c=61.25"
        );
    }

    #[test]
    fn host_load_parses_the_proc_files() {
        assert_eq!(parse_load_1m("3.21 2.00 1.50 2/345 6789\n"), Some(3.21));
        assert_eq!(parse_load_1m(""), None);
        let meminfo = "MemTotal:        8112344 kB\nMemFree:          812344 kB\nMemAvailable:    5242880 kB\n";
        assert_eq!(parse_mem_available_mib(meminfo), Some(5120));
        assert_eq!(parse_mem_available_mib("MemTotal: 1 kB\n"), None);
        assert_eq!(parse_mem_available_mib("MemAvailable: 5242880\n"), None, "no unit is not kB");
    }

    /// The questions 2026-09-14 could only answer by inference: was a solve running, and
    /// what was the other camera doing?
    #[tokio::test]
    async fn the_context_reads_push_to_and_the_other_camera() {
        let (state, _dw) = AppState::new_for_testing();
        let push_to = PushToState::default();
        let _solving = push_to
            .try_begin_solve(Instant::now(), Duration::ZERO)
            .expect("a fresh state has nothing to contend with");
        *state.push_to.write().await = Some(push_to);
        *state.slot(CameraRole::Main).phase.write().await = CameraPhase::Capturing;

        let mut stalls = StallTracker::default();
        stalls.frame_delivered();
        let context = StallContext::gather(&state, CameraRole::Guide, &stalls);

        assert_eq!(context.frames, 1);
        assert!(context.since_last_frame.is_some());
        assert_eq!(context.solve_in_flight, Some(true));
        assert_eq!(context.other_camera, Some(CameraPhase::Capturing));
    }

    #[tokio::test]
    async fn a_held_lock_reads_as_unknown_instead_of_blocking_the_loop() {
        let (state, _dw) = AppState::new_for_testing();
        *state.push_to.write().await = Some(PushToState::default());
        let _held = state.push_to.write().await;

        let context = StallContext::gather(&state, CameraRole::Main, &StallTracker::default());

        assert_eq!(context.solve_in_flight, None);
    }
}
