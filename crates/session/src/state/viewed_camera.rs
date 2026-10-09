//! Which camera the eyepiece pages show: the operator's choice on `/`, followed by every
//! `/eyepiece` and `/eyepiece_quality` viewer, who have no control of their own.
//!
//! Runtime state, never persisted: a fresh start has no guide camera to show.

use tokio::sync::{broadcast, watch};

use super::CameraRole;
use crate::events::ServerEvent;

/// What a [`ViewedCamera::select`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewSelection {
    Changed,
    Unchanged,
    /// The role holds no camera.
    Refused,
}

/// The viewed camera, as a watch channel so the eyepiece sockets switch the moment it moves.
///
/// Every change is announced as `viewed_camera_changed` from under the channel's lock, so
/// two changes racing each other are announced in the order they happened: read after the
/// lock, the later one's event could go out first and leave every client on the wrong camera.
pub struct ViewedCamera {
    tx: watch::Sender<CameraRole>,
    events: broadcast::Sender<ServerEvent>,
}

impl ViewedCamera {
    pub fn new(events: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            tx: watch::Sender::new(CameraRole::Main),
            events,
        }
    }

    pub fn get(&self) -> CameraRole {
        *self.tx.borrow()
    }

    pub fn subscribe(&self) -> watch::Receiver<CameraRole> {
        self.tx.subscribe()
    }

    /// The viewed camera, as the event that replaces a client's copy.
    pub fn event(&self) -> ServerEvent {
        ServerEvent::ViewedCameraChanged { camera: self.get() }
    }

    /// Show `role` if `occupied` says a camera holds it and still delivers frames.
    ///
    /// `occupied` runs under the channel's lock, and a guide camera on its way out falls
    /// back only after its roster says so (`WarmingUp`, or the entry gone), so a select
    /// racing a disconnect either sees that or lands first and is reverted — never Guide
    /// over a role that will show nothing new.
    pub fn select(&self, role: CameraRole, occupied: impl FnOnce(CameraRole) -> bool) -> ViewSelection {
        let mut refused = false;
        let changed = self.tx.send_if_modified(|current| {
            if *current == role {
                return false;
            }
            if role == CameraRole::Guide && !occupied(role) {
                refused = true;
                return false;
            }
            *current = role;
            self.announce(role);
            true
        });
        match (refused, changed) {
            (true, _) => ViewSelection::Refused,
            (false, true) => ViewSelection::Changed,
            (false, false) => ViewSelection::Unchanged,
        }
    }

    /// Back to the imaging camera; `true` when the view moved.
    pub fn fall_back_to_main(&self) -> bool {
        self.tx.send_if_modified(|current| {
            if *current == CameraRole::Main {
                return false;
            }
            *current = CameraRole::Main;
            self.announce(CameraRole::Main);
            true
        })
    }

    /// Under the channel's lock; a broadcast send never blocks.
    fn announce(&self, camera: CameraRole) {
        let _ = self.events.send(ServerEvent::ViewedCameraChanged { camera });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewed() -> ViewedCamera {
        ViewedCamera::new(broadcast::channel(16).0)
    }

    fn announced(rx: &mut broadcast::Receiver<ServerEvent>) -> Vec<CameraRole> {
        std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|event| match event {
                ServerEvent::ViewedCameraChanged { camera } => Some(camera),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_change_and_nothing_else_is_announced() {
        let (events, mut rx) = broadcast::channel(16);
        let viewed = ViewedCamera::new(events);

        viewed.select(CameraRole::Guide, |_| false);
        viewed.select(CameraRole::Main, |_| true);
        viewed.select(CameraRole::Guide, |_| true);
        viewed.select(CameraRole::Guide, |_| true);
        viewed.fall_back_to_main();
        viewed.fall_back_to_main();

        assert_eq!(announced(&mut rx), [CameraRole::Guide, CameraRole::Main]);
    }

    /// The order guarantee: the event is out before the lock is released, so the next
    /// change — which has to take the lock — cannot announce first.
    #[test]
    fn a_change_is_announced_before_the_next_can_land() {
        let (events, mut rx) = broadcast::channel(16);
        let viewed = std::sync::Arc::new(ViewedCamera::new(events));

        viewed.select(CameraRole::Guide, |_| {
            let racing = std::sync::Arc::clone(&viewed);
            // Blocks on the lock this select holds, so it can only run after it.
            std::thread::spawn(move || racing.fall_back_to_main());
            std::thread::sleep(std::time::Duration::from_millis(50));
            true
        });
        while viewed.get() != CameraRole::Main {
            std::thread::yield_now();
        }

        assert_eq!(announced(&mut rx), [CameraRole::Guide, CameraRole::Main]);
    }

    #[test]
    fn starts_on_the_imaging_camera() {
        assert_eq!(viewed().get(), CameraRole::Main);
    }

    #[test]
    fn the_guide_camera_is_refused_while_its_role_is_empty() {
        let viewed = viewed();
        assert_eq!(viewed.select(CameraRole::Guide, |_| false), ViewSelection::Refused);
        assert_eq!(viewed.get(), CameraRole::Main);
    }

    #[test]
    fn selecting_the_current_camera_changes_nothing() {
        let viewed = viewed();
        let mut rx = viewed.subscribe();
        assert_eq!(viewed.select(CameraRole::Main, |_| true), ViewSelection::Unchanged);
        assert!(!rx.has_changed().unwrap());

        assert_eq!(viewed.select(CameraRole::Guide, |_| true), ViewSelection::Changed);
        rx.mark_unchanged();
        assert_eq!(viewed.select(CameraRole::Guide, |_| true), ViewSelection::Unchanged);
        assert!(!rx.has_changed().unwrap());
    }

    /// Main never asks the roster: the imaging stream keeps its last frame without a camera.
    #[test]
    fn the_imaging_camera_is_always_selectable() {
        let viewed = viewed();
        viewed.select(CameraRole::Guide, |_| true);
        assert_eq!(viewed.select(CameraRole::Main, |_| panic!("asked the roster")), ViewSelection::Changed);
    }

    #[test]
    fn subscribers_see_every_switch() {
        let viewed = viewed();
        let mut rx = viewed.subscribe();
        viewed.select(CameraRole::Guide, |_| true);
        assert!(rx.has_changed().unwrap());
        assert_eq!(*rx.borrow_and_update(), CameraRole::Guide);
        assert!(viewed.fall_back_to_main());
        assert_eq!(*rx.borrow_and_update(), CameraRole::Main);
        assert!(!viewed.fall_back_to_main(), "already on Main");
        assert!(!rx.has_changed().unwrap());
    }
}
