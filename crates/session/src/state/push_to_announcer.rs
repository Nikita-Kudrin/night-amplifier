//! What clients were last told about Push-To — target, direction, solve, blocker — so a
//! sync can restate each of them: a lost `position_solved` once left "Searching…" up, and
//! a lost `push_to_blocked` was never sent again.

use std::sync::{Mutex, MutexGuard};

use tokio::sync::broadcast;

use night_amplifier_core::push_to::{PushToBlocker, PushToEvent, PushToEvents};

use crate::events::{PushDirection, PushToTarget, ServerEvent, SolveStatus, SolveVerdict};

/// Every Push-To event goes out through here, recorded and sent under one lock: the state
/// is changed before its event leaves, and two racing announcements reach clients in the
/// order they were recorded. Also the plugin's [`PushToEvents`] port.
pub struct PushToAnnouncer {
    announced: Mutex<Announced>,
    events: broadcast::Sender<ServerEvent>,
}

#[derive(Default)]
struct Announced {
    target: Option<PushToTarget>,
    direction: Option<PushDirection>,
    solve: SolveStatus,
    blocker: Option<String>,
    /// The last solve failed: every direction is the fix before it until one succeeds. A
    /// cancel says nothing about the sky, so it leaves this as it was.
    fix_stale: bool,
}

impl Announced {
    /// What `event` does to a client's copy, handler for handler as
    /// `web/src/composables/events/pushToEvents.ts` does it, so a client that saw every
    /// event and one handed the snapshot hold the same state.
    fn apply(&mut self, event: &ServerEvent) {
        use ServerEvent as E;
        match event {
            E::TargetChanged(target) => self.target = Some(target.clone()),
            E::TargetCleared => *self = Self::default(),
            E::PushDirectionUpdated(direction) => self.direction = Some(direction.clone()),
            E::PushDirectionCleared => self.direction = None,
            E::PlateSolvingStarted { target_name } => {
                self.solve = SolveStatus {
                    in_progress: true,
                    target_name: target_name.clone(),
                    ..SolveStatus::default()
                };
                self.blocker = None;
            }
            E::PlateSolvingProgress { stage, attempt, total } => {
                self.solve.in_progress = true;
                self.solve.stage = Some(crate::events::SolveStage {
                    stage: stage.clone(),
                    attempt: *attempt,
                    total: *total,
                });
            }
            E::PositionSolved { .. } => {
                self.solve.finish(Some(SolveVerdict::Success));
                self.fix_stale = false;
            }
            E::PositionSolveFailed { .. } => {
                self.solve.finish(Some(SolveVerdict::Failed));
                self.fix_stale = true;
            }
            E::PlateSolvingCancelled => self.solve.finish(Some(SolveVerdict::Cancelled)),
            E::PlateSolvingRestarted { .. } => {
                self.solve.finish(None);
                self.blocker = None;
            }
            E::PlateSolvingStatus(status) => self.solve = status.clone(),
            E::PushToBlocked { reason } => self.blocker = reason.clone(),
            _ => {}
        }
    }
}

impl PushToAnnouncer {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            announced: Mutex::default(),
            events,
        }
    }

    fn announced(&self) -> MutexGuard<'_, Announced> {
        self.announced.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, announced: &mut Announced, event: ServerEvent) {
        announced.apply(&event);
        let _ = self.events.send(event);
    }

    /// Record and send a solve event (`plate_solving_*`, `position_solve*`). A failure
    /// restates the arrow it leaves up as stale: the server keeps it (a frame-quality failure
    /// keeps Pro's position too), and it came back looking like a fresh fix.
    pub fn announce(&self, event: ServerEvent) {
        let mut announced = self.announced();
        self.send(&mut announced, event);
        let Some(direction) = announced.direction.as_ref() else {
            return;
        };
        if announced.fix_stale && !direction.stale {
            let restated = PushDirection {
                stale: true,
                ..direction.clone()
            };
            self.send(&mut announced, ServerEvent::PushDirectionUpdated(restated));
        }
    }

    /// The old arrow is dropped with the old target: it was computed for that target, even
    /// when the new one happens to lie the same way.
    pub fn target_changed(&self, target: PushToTarget) {
        let mut announced = self.announced();
        self.send(&mut announced, ServerEvent::TargetChanged(target));
        if announced.direction.is_some() {
            self.send(&mut announced, ServerEvent::PushDirectionCleared);
        }
    }

    /// A client's `target_cleared` forgets the solve too, so one still running is restated.
    pub fn target_cleared(&self) {
        let mut announced = self.announced();
        let running = announced.solve.in_progress.then(|| announced.solve.clone());
        self.send(&mut announced, ServerEvent::TargetCleared);
        if let Some(solve) = running {
            self.send(&mut announced, ServerEvent::PlateSolvingStatus(solve));
        }
    }

    /// Send `direction` unless it matches the last one ([`PushDirection::same_as`]): it is
    /// recomputed every frame but changes only with the position or target, and was once one
    /// message per frame per client. Stamped stale while the last solve has failed. `true`
    /// when sent.
    pub fn announce_direction(&self, mut direction: PushDirection) -> bool {
        let mut announced = self.announced();
        direction.stale = announced.fix_stale;
        if announced.direction.as_ref().is_some_and(|last| last.same_as(&direction)) {
            return false;
        }
        self.send(&mut announced, ServerEvent::PushDirectionUpdated(direction));
        true
    }

    /// Send why solving is idle if that changed, so a state holding for a hundred frames
    /// costs one event. `true` when sent.
    pub fn announce_blocker(&self, blocker: Option<PushToBlocker>) -> bool {
        let reason = blocker.map(|b| b.reason().to_string());
        let mut announced = self.announced();
        if announced.blocker == reason {
            return false;
        }
        self.send(&mut announced, ServerEvent::push_to_blocked(reason));
        true
    }

    pub fn direction(&self) -> Option<PushDirection> {
        self.announced().direction.clone()
    }

    pub fn solve(&self) -> SolveStatus {
        self.announced().solve.clone()
    }

    pub fn blocker(&self) -> Option<String> {
        self.announced().blocker.clone()
    }

    pub fn target_event(&self) -> ServerEvent {
        match &self.announced().target {
            Some(target) => ServerEvent::TargetChanged(target.clone()),
            None => ServerEvent::TargetCleared,
        }
    }

    pub fn direction_event(&self) -> ServerEvent {
        match self.direction() {
            Some(direction) => ServerEvent::PushDirectionUpdated(direction),
            None => ServerEvent::PushDirectionCleared,
        }
    }

    pub fn solve_event(&self) -> ServerEvent {
        ServerEvent::PlateSolvingStatus(self.solve())
    }

    pub fn blocker_event(&self) -> ServerEvent {
        ServerEvent::push_to_blocked(self.blocker())
    }
}

impl PushToEvents for PushToAnnouncer {
    fn emit(&self, event: PushToEvent) {
        self.announce(event.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use night_amplifier_core::push_to::{DirectionHint, PushToDirectionResponse};

    fn announcer() -> (PushToAnnouncer, broadcast::Receiver<ServerEvent>) {
        let (events, receiver) = broadcast::channel(16);
        (PushToAnnouncer::new(events), receiver)
    }

    fn sent(receiver: &mut broadcast::Receiver<ServerEvent>) -> Vec<String> {
        std::iter::from_fn(|| receiver.try_recv().ok()).map(|e| e.to_json()).collect()
    }

    fn direction(angle_deg: f64, distance_deg: f64, is_close: bool) -> PushDirection {
        PushDirection::new(
            PushToDirectionResponse {
                angle_deg,
                distance_deg,
                is_close,
                direction_hint: DirectionHint::NorthEast,
                current_position: None,
                target: None,
                solved_camera: Some("Guide Cam".into()),
            },
            Some(1.2),
        )
    }

    fn m31() -> PushToTarget {
        PushToTarget {
            name: Some("Andromeda".into()),
            designation: Some("M31".into()),
            ra_degrees: 10.68,
            dec_degrees: 41.27,
        }
    }

    #[test]
    fn with_nothing_announced_each_topic_states_none() {
        let (announcer, _rx) = announcer();
        let restated = [
            announcer.target_event(),
            announcer.direction_event(),
            announcer.solve_event(),
            announcer.blocker_event(),
        ]
        .map(|event| event.to_json());

        assert_eq!(
            restated,
            [
                r#"{"type":"target_cleared"}"#,
                r#"{"type":"push_direction_cleared"}"#,
                r#"{"type":"plate_solving_status","in_progress":false,"target_name":null,"stage":null,"last_result":null}"#,
                r#"{"type":"push_to_blocked","reason":null}"#,
            ]
        );
    }

    #[test]
    fn each_topic_restates_what_was_announced() {
        let (announcer, _rx) = announcer();
        announcer.target_changed(m31());
        announcer.announce_direction(direction(90.0, 2.0, false));
        announcer.announce_blocker(Some(PushToBlocker::Settling));

        assert_eq!(announcer.target_event().to_json(), ServerEvent::TargetChanged(m31()).to_json());
        assert_eq!(
            announcer.direction_event().to_json(),
            r#"{"type":"push_direction_updated","angle_deg":90.0,"distance_deg":2.0,"direction_hint":"NE","is_close":false,"fov_deg":1.2,"solved_camera":"Guide Cam","stale":false}"#
        );
        assert_eq!(
            announcer.blocker_event().to_json(),
            r#"{"type":"push_to_blocked","reason":"Waiting for the view to settle"}"#
        );
    }

    #[test]
    fn an_unchanged_direction_is_only_announced_once() {
        let (announcer, _rx) = announcer();

        assert!(announcer.announce_direction(direction(122.68, 4.512, false)));
        assert!(!announcer.announce_direction(direction(122.68, 4.512, false)));
        assert!(
            !announcer.announce_direction(direction(122.680_004, 4.512_000_2, false)),
            "floating-point jitter is not a change"
        );
    }

    #[test]
    fn a_direction_the_user_could_act_on_is_announced() {
        let (announcer, _rx) = announcer();
        assert!(announcer.announce_direction(direction(122.6, 4.5, false)));

        assert!(announcer.announce_direction(direction(124.0, 4.5, false)), "a 1.4° turn");
        assert!(announcer.announce_direction(direction(124.0, 3.9, false)), "half a degree closer");
        assert!(announcer.announce_direction(direction(124.0, 3.9, true)), "now close enough");
    }

    #[test]
    fn the_same_numbers_from_the_other_camera_are_announced() {
        // A chevron is drawn only over the camera that oriented it.
        let (announcer, _rx) = announcer();
        assert!(announcer.announce_direction(direction(90.0, 2.0, false)));

        let from_imaging = PushDirection {
            solved_camera: Some("Imaging Cam".into()),
            ..direction(90.0, 2.0, false)
        };
        assert!(announcer.announce_direction(from_imaging.clone()));
        assert!(!announcer.announce_direction(from_imaging));
    }

    #[test]
    fn a_new_target_clears_the_arrow_and_re_announces_even_an_identical_direction() {
        // Two targets can lie the same way at the same distance; without the clear the
        // client kept numbers computed for the target the user moved away from.
        let (announcer, mut rx) = announcer();
        announcer.announce_direction(direction(90.0, 2.0, false));
        sent(&mut rx);

        announcer.target_changed(m31());

        assert_eq!(
            sent(&mut rx),
            [
                r#"{"type":"target_changed","name":"Andromeda","designation":"M31","ra_degrees":10.68,"dec_degrees":41.27}"#,
                r#"{"type":"push_direction_cleared"}"#,
            ]
        );
        assert_eq!(announcer.direction_event().to_json(), r#"{"type":"push_direction_cleared"}"#);
        assert!(announcer.announce_direction(direction(90.0, 2.0, false)));
    }

    fn failed() -> ServerEvent {
        ServerEvent::position_solve_failed("no match")
    }

    fn solved() -> ServerEvent {
        ServerEvent::position_solved(10.68, 41.27, "", "", None, 1.0, 0.0)
    }

    // The direction outlives a failure, and the page's readout came back looking fresh;
    // the page's verdict is dismissible, so the mark travels with the direction itself.
    #[test]
    fn a_failed_solve_restates_the_arrow_it_leaves_up_as_stale_once() {
        let (announcer, mut rx) = announcer();
        announcer.announce_direction(direction(90.0, 2.0, false));
        sent(&mut rx);

        announcer.announce(failed());
        let after_failure = sent(&mut rx);
        announcer.announce(failed());

        assert_eq!(after_failure.len(), 2);
        assert!(after_failure[0].starts_with(r#"{"type":"position_solve_failed""#));
        assert!(after_failure[1].ends_with(r#""solved_camera":"Guide Cam","stale":true}"#));
        assert_eq!(sent(&mut rx).len(), 1, "already marked: only the second failure goes out");
        assert!(announcer.direction().is_some_and(|d| d.stale));
        assert!(announcer.direction_event().to_json().ends_with(r#""stale":true}"#), "a sync restates the mark");
    }

    #[test]
    fn a_failure_with_no_arrow_up_sends_only_the_failure() {
        let (announcer, mut rx) = announcer();
        announcer.announce(failed());

        assert_eq!(sent(&mut rx).len(), 1);
        assert_eq!(announcer.direction_event().to_json(), r#"{"type":"push_direction_cleared"}"#);
    }

    // After a frame-quality failure Pro keeps its position, and every frame's direction is
    // the old fix's: stamped, so it neither unmarks the arrow nor repeats it.
    #[test]
    fn directions_after_a_failure_are_stale_until_a_solve_succeeds() {
        let (announcer, _rx) = announcer();
        announcer.announce(failed());

        assert!(announcer.announce_direction(direction(90.0, 2.0, false)));
        assert!(announcer.direction().is_some_and(|d| d.stale));
        assert!(!announcer.announce_direction(direction(90.0, 2.0, false)));

        announcer.announce(solved());
        assert!(
            announcer.announce_direction(direction(90.0, 2.0, false)),
            "the same numbers, now confirmed, are news"
        );
        assert!(announcer.direction().is_some_and(|d| !d.stale));
    }

    #[test]
    fn a_cancel_neither_marks_nor_clears_the_arrow() {
        let (announcer, mut rx) = announcer();
        announcer.announce_direction(direction(90.0, 2.0, false));
        announcer.announce(ServerEvent::plate_solving_cancelled());
        assert!(announcer.direction().is_some_and(|d| !d.stale));

        announcer.announce(failed());
        sent(&mut rx);
        announcer.announce(ServerEvent::plate_solving_cancelled());
        assert_eq!(sent(&mut rx).len(), 1);
        assert!(announcer.direction().is_some_and(|d| d.stale));
    }

    // A new target keeps the unconfirmed position its direction is computed from; a
    // cleared one starts over.
    #[test]
    fn a_new_target_keeps_the_mark_and_a_cleared_one_drops_it() {
        let (announcer, _rx) = announcer();
        announcer.announce(failed());

        announcer.target_changed(m31());
        announcer.announce_direction(direction(45.0, 5.0, false));
        assert!(announcer.direction().is_some_and(|d| d.stale));

        announcer.target_cleared();
        announcer.announce_direction(direction(45.0, 5.0, false));
        assert!(announcer.direction().is_some_and(|d| !d.stale));
    }

    #[test]
    fn a_blocker_is_announced_on_each_transition_and_not_between() {
        let (announcer, mut rx) = announcer();

        assert!(!announcer.announce_blocker(None), "nothing blocking is what the snapshot already says");
        assert!(announcer.announce_blocker(Some(PushToBlocker::SolverNotReady)));
        for _ in 0..5 {
            assert!(!announcer.announce_blocker(Some(PushToBlocker::SolverNotReady)));
        }
        assert!(announcer.announce_blocker(Some(PushToBlocker::NoTarget)));
        assert!(announcer.announce_blocker(None), "clearing the notice is news");

        assert_eq!(sent(&mut rx).len(), 3);
        assert_eq!(announcer.blocker_event().to_json(), r#"{"type":"push_to_blocked","reason":null}"#);
    }

    #[test]
    fn the_state_is_recorded_before_its_event_is_sent() {
        let (events, mut rx) = broadcast::channel(16);
        let announcer = std::sync::Arc::new(PushToAnnouncer::new(events));
        let watcher = std::sync::Arc::clone(&announcer);
        let observed = std::thread::spawn(move || {
            let event = rx.blocking_recv().unwrap();
            (event.to_json(), watcher.solve())
        });

        announcer.announce(ServerEvent::position_solve_failed("no match"));

        let (event, solve) = observed.join().unwrap();
        assert_eq!(event, r#"{"type":"position_solve_failed","reason":"no match"}"#);
        assert_eq!(solve.last_result, Some(SolveVerdict::Failed));
    }

    #[test]
    fn a_solve_is_followed_from_start_to_verdict() {
        let (announcer, _rx) = announcer();
        announcer.announce_blocker(Some(PushToBlocker::Settling));

        announcer.emit(PushToEvent::SolveStarted { target_name: Some("M31".into()) });
        announcer.emit(PushToEvent::SolveProgress { stage: "Blind".into(), attempt: 3, total: 4 });
        assert_eq!(
            announcer.solve_event().to_json(),
            r#"{"type":"plate_solving_status","in_progress":true,"target_name":"M31","stage":{"stage":"Blind","attempt":3,"total":4},"last_result":null}"#
        );
        assert_eq!(announcer.blocker(), None, "a solve starting ends the blocker, as on the client");

        for (event, verdict) in [
            (ServerEvent::position_solved(1.0, 2.0, "a", "b", None, 0.9, 0.0), Some(SolveVerdict::Success)),
            (ServerEvent::position_solve_failed("x"), Some(SolveVerdict::Failed)),
            (ServerEvent::plate_solving_cancelled(), Some(SolveVerdict::Cancelled)),
            (ServerEvent::plate_solving_restarted("optics"), None),
        ] {
            announcer.emit(PushToEvent::SolveStarted { target_name: Some("M31".into()) });
            announcer.announce(event);
            let solve = announcer.solve();
            assert!(!solve.in_progress);
            assert_eq!((solve.last_result, solve.stage, solve.target_name.as_deref()), (verdict, None, Some("M31")));
        }
    }

    #[test]
    fn clearing_the_target_forgets_what_a_client_forgets_and_restates_a_running_solve() {
        let (announcer, mut rx) = announcer();
        announcer.target_changed(m31());
        announcer.announce_direction(direction(90.0, 2.0, false));
        announcer.announce_blocker(Some(PushToBlocker::TelescopeMoving));
        announcer.emit(PushToEvent::SolveStarted { target_name: Some("M31".into()) });
        sent(&mut rx);

        announcer.target_cleared();

        assert_eq!(
            sent(&mut rx),
            [
                r#"{"type":"target_cleared"}"#.to_string(),
                announcer.solve_event().to_json(),
            ]
        );
        assert!(announcer.solve().in_progress, "the plugin is still solving");
        assert_eq!(announcer.target_event().to_json(), r#"{"type":"target_cleared"}"#);
        assert_eq!(announcer.direction(), None);
        assert_eq!(announcer.blocker(), None);
    }

    #[test]
    fn clearing_the_target_with_no_solve_running_sends_only_the_clear() {
        let (announcer, mut rx) = announcer();
        announcer.target_changed(m31());
        announcer.announce(ServerEvent::position_solve_failed("no match"));
        sent(&mut rx);

        announcer.target_cleared();

        assert_eq!(sent(&mut rx), [r#"{"type":"target_cleared"}"#]);
        assert_eq!(announcer.solve(), SolveStatus::default());
    }
}
