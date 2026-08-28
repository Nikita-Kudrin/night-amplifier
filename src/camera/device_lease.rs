//! Ownership guard that stops a stale camera handle from closing a device a
//! newer handle has since opened.
//!
//! Vendor SDKs address cameras by a device *index* — `POACloseCamera(0)`,
//! `ASICloseCamera(0)`, `SVBCloseCamera(0)` — not by an opaque per-open handle.
//! A close issued by an abandoned handle therefore lands on whoever holds that
//! index *now*, not on the object that issued it.
//!
//! Handles get abandoned as a matter of routine. There is no way to cancel a
//! stuck synchronous FFI call in Rust, so `capture_frame_bounded` and
//! `poll_camera_status_bounded` hand the call to a detached thread and stop
//! waiting for it. That thread's `Drop` runs whenever the SDK finally returns,
//! which can be minutes later — long after a reconnect has reopened the same
//! index. Observed in the field on 2026-08-22: a `capture()` that stalled for
//! 30 s was abandoned, the user reconnected 9 s later onto a working handle,
//! and ~80 s after that every call started returning `POA_ERROR_NOT_OPENED`
//! because the abandoned handle's destructor had closed device 0 underneath
//! the live one.
//!
//! Every open takes a [`DeviceLease`], which stamps the current generation of
//! its `(provider, index)` slot. Opening the same slot again bumps the
//! generation, which retroactively invalidates every older lease.
//! [`DeviceLease::begin_close`] is the single gate every vendor close call must
//! pass through: it authorizes exactly one close, and only for the lease that
//! still owns the slot.
//!
//! Providers whose SDK closes by opaque pointer rather than index (QHY,
//! ToupTek) cannot hit the id-reuse case, but they take a lease anyway so the
//! double-close half of the guard applies uniformly.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing::warn;

/// Current generation per `(provider, index)`. A slot absent from the map has
/// never been opened; generations start at 1 so a default-constructed 0 can
/// never look current.
type SlotTable = HashMap<(&'static str, i32), u64>;

fn slots() -> &'static Mutex<SlotTable> {
    static SLOTS: OnceLock<Mutex<SlotTable>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Proof that a handle is the current owner of one vendor device slot.
///
/// Held by the shim-level camera struct for as long as the handle exists. See
/// the module docs for what it guards against.
#[derive(Debug)]
pub struct DeviceLease {
    provider: &'static str,
    index: i32,
    generation: u64,
    closed: AtomicBool,
}

impl DeviceLease {
    /// Claim `(provider, index)`, superseding any lease previously issued for
    /// it. Call this once per successful vendor open.
    pub fn acquire(provider: &'static str, index: i32) -> Self {
        let mut table = slots().lock().unwrap_or_else(|e| e.into_inner());
        let generation = table
            .entry((provider, index))
            .and_modify(|g| *g += 1)
            .or_insert(1);
        Self {
            provider,
            index,
            generation: *generation,
            closed: AtomicBool::new(false),
        }
    }

    /// Claim a slot no other lease can ever hold, for providers whose SDK
    /// closes by opaque pointer (QHY, ToupTek) rather than by device index.
    /// Reopening cannot alias such a handle, so only the double-close half of
    /// the guard applies and `is_current` stays true for the handle's life.
    pub fn acquire_unique(provider: &'static str) -> Self {
        static NEXT: AtomicI32 = AtomicI32::new(0);
        // Negative indices cannot collide with a vendor device index.
        let index = -1 - NEXT.fetch_add(1, Ordering::Relaxed);
        Self::acquire(provider, index)
    }

    /// Whether this lease still owns its device slot — i.e. no later open has
    /// superseded it. Read-only; does not affect `begin_close`.
    pub fn is_current(&self) -> bool {
        let table = slots().lock().unwrap_or_else(|e| e.into_inner());
        table.get(&(self.provider, self.index)) == Some(&self.generation)
    }

    /// Authorize one vendor close call, or explain why not.
    ///
    /// Returns `true` exactly once, and only while this lease still owns the
    /// slot. A superseded lease returns `false` (closing would hit the newer
    /// handle's device) and so does a second call on the same lease (the
    /// explicit `close()` already ran, and `Drop` is following it).
    pub fn begin_close(&self) -> bool {
        if !self.is_current() {
            warn!(
                provider = self.provider,
                index = self.index,
                generation = self.generation,
                "Skipping close of a superseded camera handle — the device now belongs to a newer handle"
            );
            return false;
        }
        !self.closed.swap(true, Ordering::SeqCst)
    }

    pub fn provider(&self) -> &'static str {
        self.provider
    }

    pub fn index(&self) -> i32 {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test uses its own provider name so the shared slot table cannot
    /// couple tests running in parallel.
    #[test]
    fn first_lease_owns_the_slot_and_may_close_once() {
        let lease = DeviceLease::acquire("test-single", 0);
        assert!(lease.is_current());
        assert!(lease.begin_close(), "first close should be authorized");
        assert!(
            !lease.begin_close(),
            "second close on the same lease must be refused"
        );
    }

    #[test]
    fn reopening_supersedes_the_previous_lease() {
        let stale = DeviceLease::acquire("test-supersede", 0);
        let fresh = DeviceLease::acquire("test-supersede", 0);

        assert!(!stale.is_current());
        assert!(fresh.is_current());
        assert!(
            !stale.begin_close(),
            "a superseded lease must not close the device the new lease owns"
        );
        assert!(fresh.begin_close());
    }

    /// The field failure in miniature: a handle is abandoned, a reconnect opens
    /// the same index, and the abandoned handle's destructor runs afterwards.
    #[test]
    fn abandoned_handle_closing_late_cannot_kill_the_reconnected_device() {
        let abandoned = DeviceLease::acquire("test-late-drop", 0);
        let reconnected = DeviceLease::acquire("test-late-drop", 0);

        // The stuck SDK call finally returns and the abandoned handle drops.
        assert!(!abandoned.begin_close());

        // The reconnected handle is untouched and still usable.
        assert!(reconnected.is_current());
        assert!(reconnected.begin_close());
    }

    #[test]
    fn unique_leases_never_supersede_each_other() {
        let first = DeviceLease::acquire_unique("test-unique");
        let second = DeviceLease::acquire_unique("test-unique");

        assert!(
            first.is_current(),
            "an opaque-handle lease is never aliased"
        );
        assert!(second.is_current());
        assert_ne!(first.index(), second.index());

        // The double-close half of the guard still applies to each.
        assert!(first.begin_close());
        assert!(!first.begin_close());
        assert!(second.begin_close());
    }

    #[test]
    fn slots_are_independent_across_indices_and_providers() {
        let a0 = DeviceLease::acquire("test-slots-a", 0);
        let a1 = DeviceLease::acquire("test-slots-a", 1);
        let b0 = DeviceLease::acquire("test-slots-b", 0);

        assert!(a0.is_current());
        assert!(a1.is_current());
        assert!(b0.is_current());

        let a0_again = DeviceLease::acquire("test-slots-a", 0);
        assert!(!a0.is_current(), "index 0 was superseded");
        assert!(a1.is_current(), "index 1 must be unaffected");
        assert!(b0.is_current(), "the other provider must be unaffected");
        assert!(a0_again.is_current());
    }
}
