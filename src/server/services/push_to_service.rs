//! Push-To Navigation Service
//!
//! Service layer for plate solving and telescope navigation guidance.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::super::dto::{
    CatalogEntryResponse, CoordinateResponse, PushToDirectionResponse, PushToStatusResponse,
};
use super::super::events::ServerEvent;
use super::super::state::{AppState, TelescopeSettings};
use crate::push_to::{PushToBlocker, PushToError, PUSH_TO_PLUGIN};

/// Push-To navigation service
pub struct PushToService;

impl PushToService {
    /// Get the current Push-To status
    ///
    /// Read-only on purpose. `PushToState` mirrors the plugin so the stacking
    /// thread can gate plate solving without awaiting it, but a status poll is
    /// the wrong place to repair that mirror: `solving_in_progress` is a latch
    /// owned by `try_plate_solve`, and clearing it from here would let a second
    /// solve start under one already in flight. The mirror is maintained by the
    /// target mutations below and re-synced from `try_plate_solve`'s own
    /// authoritative read of the plugin.
    pub async fn get_status(_state: &AppState) -> PushToStatusResponse {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_status().await
        } else {
            PushToStatusResponse {
                solver_ready: false,
                is_solving: false,
                current_target: None,
                last_position: None,
                direction: None,
            }
        }
    }

    /// Cancel the current plate solving process.
    ///
    /// The event is sent only when a solve was actually in flight. Announcing an
    /// unconditional `PositionSolveFailed` meant every settings save that touched
    /// the telescope block rendered as "Failed to find M31" in the status bar, and
    /// a cancel is not a failure in any case — reporting it as one made a still-good
    /// last position look untrustworthy.
    pub async fn cancel_solve(state: &AppState) -> Result<bool, String> {
        let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) else {
            return Err("Push-To navigation requires Night Amplifier Pro".to_string());
        };

        let was_solving = plugin.cancel_solve().await.map_err(|e| e.to_string())?;
        if was_solving {
            let _ = state.events.send(ServerEvent::plate_solving_cancelled());
        }
        Ok(was_solving)
    }

    /// Abandon any solve in flight and arm a fresh one.
    ///
    /// For changes that invalidate the solve rather than the pointing — focal length,
    /// sensor, binning. A bare cancel is not enough: the star field has not changed,
    /// so the movement detector would report `Idle` on every later frame and nothing
    /// would ever re-solve against the new optics.
    pub async fn restart_solve(state: &AppState, reason: &str) -> Result<(), String> {
        let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) else {
            return Ok(()); // No plugin available; nothing to restart.
        };

        plugin.restart_solve().await.map_err(|e| e.to_string())?;
        let _ = state
            .events
            .send(ServerEvent::plate_solving_restarted(reason));
        Ok(())
    }

    /// Tell the solver which camera is producing frames now.
    ///
    /// Called on connect and on disconnect rather than only on a settings change: the
    /// telescope profile is what the *user* believes the optics are, and two cameras
    /// sharing a sensor format leave it identical. The camera name is the one fact
    /// that always changes, and it is what lets the solver notice that a remembered
    /// field of view was measured through something else.
    ///
    /// No-op without the Pro plugin.
    pub async fn set_active_camera(camera: Option<String>) {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.set_active_camera(camera).await;
        }
    }

    /// Search the catalog
    pub async fn search_catalog(
        _state: &AppState,
        query: &str,
        limit: usize,
    ) -> Vec<CatalogEntryResponse> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.search_catalog(query, limit).await
        } else {
            vec![]
        }
    }

    /// Get all catalog entries of a specific type
    pub async fn get_catalog_by_type(
        _state: &AppState,
        catalog_type_str: &str,
    ) -> Vec<CatalogEntryResponse> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_catalog_by_type(catalog_type_str).await
        } else {
            vec![]
        }
    }

    /// Set target by name
    pub async fn set_target_by_name(
        state: &AppState,
        name: &str,
    ) -> Result<CatalogEntryResponse, String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.set_target_by_name(name).await?;
            state.push_to_target_changed(true).await;
            let _ = state.events.send(ServerEvent::target_changed(
                result.name.clone(),
                Some(result.designation.clone()),
                result.ra_degrees,
                result.dec_degrees,
            ));
            Ok(result)
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Set target by coordinates
    pub async fn set_target_by_coords(
        state: &AppState,
        ra_degrees: f64,
        dec_degrees: f64,
    ) -> Result<CoordinateResponse, String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.set_target_by_coords(ra_degrees, dec_degrees).await?;
            state.push_to_target_changed(true).await;
            // For custom coordinates, name is usually the coordinate string
            let _ = state.events.send(ServerEvent::target_changed(
                Some(result.ra_string.clone() + " " + &result.dec_string),
                None,
                result.ra_degrees,
                result.dec_degrees,
            ));
            Ok(result)
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Clear the current target
    pub async fn clear_target(state: &AppState) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            let result = plugin.clear_target().await;
            // Only mirror a clear that actually happened — a failed clear leaves
            // the plugin holding the target, and claiming otherwise would stop
            // plate solving for a target that is still set.
            if result.is_ok() {
                state.push_to_target_changed(false).await;
            }
            let _ = state.events.send(ServerEvent::target_cleared());
            result
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Get the push direction (if position and target are both set)
    pub async fn get_direction(_state: &AppState) -> Option<PushToDirectionResponse> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.get_direction().await
        } else {
            None
        }
    }

    /// Update the FOV hint for the solver
    pub async fn set_fov(_state: &AppState, fov_degrees: f32) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.set_fov(fov_degrees).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }

    /// Update telescope settings on the solver for precise FOV calculation
    pub async fn set_telescope_settings(
        _state: &AppState,
        settings: TelescopeSettings,
    ) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.set_telescope_settings(settings).await
        } else {
            Ok(()) // No plugin available; not an error
        }
    }

    /// Load a solver database
    pub async fn load_database(_state: &AppState, path: &str) -> Result<(), String> {
        if let Some(plugin) = crate::license::pro_plugin(&PUSH_TO_PLUGIN) {
            plugin.load_database(path).await
        } else {
            Err("Push-To navigation requires Night Amplifier Pro".to_string())
        }
    }
}

/// Server-side mirror of the Push-To plugin, so the stacking thread can decide
/// whether a plate solve is worth preparing a frame for without awaiting the
/// plugin's own locks. The first two fields are caches, not the source of truth
/// (the plugin is) — they exist only to keep `capture::solving::plate_solve_available`
/// synchronous and cheap; every consequential check repeats against the plugin
/// inside `try_plate_solve`. Write `has_target` through
/// [`AppState::set_push_to_has_target`]. The last two are event de-duplication
/// state, owned here since they're about what this server already told clients,
/// which the plugin has no view of.
#[derive(Default)]
pub struct PushToState {
    /// Latch owned by `try_plate_solve`: raised before a solve is spawned, cleared
    /// when it finishes.
    ///
    /// An `Arc<AtomicBool>` rather than a plain field so [`SolveLatch`] can release
    /// it from `Drop`. As a plain field it was a set/clear pair straddling an
    /// `.await`, and any panic in between left it raised for the life of the
    /// process — with plate solving silently dead from that point on.
    solving: Arc<AtomicBool>,
    /// The same, for the cheap movement watch that runs *while* a solve does — see
    /// [`PushToState::try_begin_watch`]. A second latch rather than a second use of
    /// `solving`, because their whole point is that one of them is raised for minutes
    /// and the other for milliseconds.
    watching: Arc<AtomicBool>,
    /// When a frame was last offered to the solver — see
    /// [`PushToState::try_begin_solve`].
    last_attempt: std::sync::Mutex<Option<Instant>>,
    /// When a frame was last offered to the movement watch.
    last_watch: std::sync::Mutex<Option<Instant>>,
    /// Whether the plugin currently holds a target. Written by the target
    /// mutations in [`PushToService`] and re-synced from `try_plate_solve`.
    pub has_target: bool,
    /// Last push direction announced to clients, rounded — see
    /// [`PushToState::direction_is_news`].
    last_direction_key: Option<DirectionKey>,
    /// Last blocker announced to clients. The outer `Option` distinguishes "never
    /// reported" from "reported that nothing is blocking".
    last_blocker: Option<Option<PushToBlocker>>,
}

/// A push direction rounded to the precision a person can act on.
///
/// Rounded rather than compared exactly because the direction is recomputed from
/// floating-point spherical geometry every frame: bit-identical values are not
/// guaranteed even when nothing has changed, and a de-duplication that compares
/// exactly would let the spam straight back through.
type DirectionKey = (i64, i64, bool);

impl PushToState {
    /// Whether a solve is running right now.
    pub fn is_solving(&self) -> bool {
        self.solving.load(Ordering::SeqCst)
    }

    /// Claim the solve slot, or `None` if one is already running or the previous
    /// offer was too recent. The compare-and-swap makes this a claim, not a
    /// check-then-set — two frames arriving together would both pass a bare read.
    /// `min_interval` bounds the movement check's cost: every offered frame runs a
    /// full sensitive star detection over the whole sensor (3008x3008x3 in the field
    /// log) just to decide the view hasn't changed (almost always true) — a no-op at
    /// ~1fps deep-sky, but stops a short-exposure run from paying it per frame.
    pub fn try_begin_solve(&self, now: Instant, min_interval: Duration) -> Option<SolveLatch> {
        if let Some(previous) = *self.last_attempt.lock().unwrap() {
            if now.saturating_duration_since(previous) < min_interval {
                return None;
            }
        }

        let latch = self
            .solving
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| SolveLatch(Arc::clone(&self.solving)))?;

        *self.last_attempt.lock().unwrap() = Some(now);
        Some(latch)
    }

    /// Whether offering a frame right now could possibly do anything, given how
    /// recently the last one was.
    ///
    /// Advisory, not a claim: the compare-and-swap in the `try_begin_*` pair is still
    /// what decides. This exists so `plate_solve_available` can decline *before* the
    /// caller clones a frame handle and spawns a task for an offer that the cadence
    /// floor is about to drop — a live handle at the wrong moment makes the render
    /// task's `Arc::try_unwrap` fail and copy a full-resolution frame instead.
    pub fn offer_is_due(&self, now: Instant, solve_floor: Duration, watch_floor: Duration) -> bool {
        let (last, floor) = if self.is_solving() {
            (self.last_watch.lock().unwrap(), watch_floor)
        } else {
            (self.last_attempt.lock().unwrap(), solve_floor)
        };
        match *last {
            Some(previous) => now.saturating_duration_since(previous) >= floor,
            None => true,
        }
    }

    /// Claim the *watch* slot: permission to run the movement check on this frame
    /// while a solve is in flight, or `None` if one is already running or the last
    /// was too recent.
    ///
    /// Rate-limited on its own clock. `last_attempt` belongs to the solve path and is
    /// stamped once per ladder, so sharing it would let the watch run flat out for the
    /// minutes a full-sky search takes — each run costing a full sensitive detection
    /// over the whole sensor.
    pub fn try_begin_watch(&self, now: Instant, min_interval: Duration) -> Option<WatchLatch> {
        if let Some(previous) = *self.last_watch.lock().unwrap() {
            if now.saturating_duration_since(previous) < min_interval {
                return None;
            }
        }

        let latch = self
            .watching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| WatchLatch(Arc::clone(&self.watching)))?;

        *self.last_watch.lock().unwrap() = Some(now);
        Some(latch)
    }

    /// Whether this direction differs from the last one announced, updating the
    /// record if it does.
    ///
    /// Push direction only really changes when the position or the target changes,
    /// but it was recomputed and broadcast on every captured frame regardless — one
    /// WebSocket message per frame per client, saying the same thing.
    pub fn direction_is_news(&mut self, angle_deg: f64, distance_deg: f64, is_close: bool) -> bool {
        let key = (
            (angle_deg * 10.0).round() as i64,
            (distance_deg * 1000.0).round() as i64,
            is_close,
        );
        if self.last_direction_key == Some(key) {
            return false;
        }
        self.last_direction_key = Some(key);
        true
    }

    /// Forget the last announced direction, so the next one is sent even if it is
    /// numerically identical. Used when the target changes: the arrow means something
    /// different now even when it points the same way.
    pub fn forget_direction(&mut self) {
        self.last_direction_key = None;
    }

    /// Whether this blocker differs from the last one announced, updating the record
    /// if it does. Keeps the "why is nothing happening" notice to one event per
    /// transition instead of one per frame.
    pub fn blocker_is_news(&mut self, blocker: Option<PushToBlocker>) -> bool {
        if self.last_blocker == Some(blocker) {
            return false;
        }
        self.last_blocker = Some(blocker);
        true
    }
}

/// Holds the solve latch raised for as long as it lives.
pub struct SolveLatch(Arc<AtomicBool>);

impl Drop for SolveLatch {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// Holds the watch latch raised for as long as it lives.
pub struct WatchLatch(Arc<AtomicBool>);

impl Drop for WatchLatch {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}
