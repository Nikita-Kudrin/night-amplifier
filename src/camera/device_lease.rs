//! Ownership guard against a stale camera handle closing a device a newer handle has
//! since opened. Vendor SDKs close by device *index*, not an opaque handle, so a close
//! from an abandoned handle lands on whoever holds that index *now* — and handles get
//! abandoned routinely, since a stuck FFI call can't be cancelled and is handed to a
//! detached thread whose `Drop` may fire minutes later. Observed 2026-08-22: exactly
//! this closed device 0 from under a handle the user had already reconnected onto.
//!
//! Every open takes a [`DeviceLease`], stamping its slot's generation; reopening bumps
//! it, invalidating older leases. [`DeviceLease::begin_close`] is the single gate every
//! vendor close must pass, authorizing exactly one close for the current lease.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use tracing::warn;

/// One vendor device slot: the generation of its newest lease, whether that lease is still
/// open, and the device it names, if any. Generations start at 1 so a default-constructed 0
/// can never look current; a slot absent from the map has never been opened.
#[derive(Debug)]
struct Slot {
    generation: u64,
    open: bool,
    device: Option<String>,
}

type SlotTable = HashMap<(&'static str, i32), Slot>;

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
        Self::claim(&mut table, provider, index, None)
    }

    /// Claim a slot no other lease can ever hold, for providers whose SDK
    /// closes by opaque pointer (QHY, ToupTek) rather than by device index.
    /// Reopening cannot alias such a handle, so only the double-close half of
    /// the guard applies: `is_current` stays true until the lease closes, and
    /// the close drops the slot, which nothing can claim again.
    pub fn acquire_unique(provider: &'static str) -> Self {
        let mut table = slots().lock().unwrap_or_else(|e| e.into_inner());
        Self::claim(&mut table, provider, Self::next_unique_index(), None)
    }

    /// [`Self::acquire_unique`] naming the device, refused while an open lease already names
    /// it. Checked and claimed in one step, so discovery and a connect never both open one
    /// QHY device.
    pub fn try_acquire_unique_device(provider: &'static str, device: &str) -> Option<Self> {
        let mut table = slots().lock().unwrap_or_else(|e| e.into_inner());
        if Self::names_open_device(&table, provider, device) {
            return None;
        }
        let index = Self::next_unique_index();
        Some(Self::claim(&mut table, provider, index, Some(device.to_string())))
    }

    /// Whether an open lease names `device` (see [`Self::try_acquire_unique_device`]).
    pub fn is_device_open(provider: &'static str, device: &str) -> bool {
        let table = slots().lock().unwrap_or_else(|e| e.into_inner());
        Self::names_open_device(&table, provider, device)
    }

    fn names_open_device(table: &SlotTable, provider: &'static str, device: &str) -> bool {
        table.iter().any(|(&(slot_provider, _), slot)| {
            slot_provider == provider && slot.open && slot.device.as_deref() == Some(device)
        })
    }

    fn next_unique_index() -> i32 {
        static NEXT: AtomicI32 = AtomicI32::new(0);
        // Negative indices cannot collide with a vendor device index.
        -1 - NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn is_unique(&self) -> bool {
        self.index < 0
    }

    fn claim(
        table: &mut SlotTable,
        provider: &'static str,
        index: i32,
        device: Option<String>,
    ) -> Self {
        let slot = table.entry((provider, index)).or_insert(Slot {
            generation: 0,
            open: false,
            device: None,
        });
        slot.generation += 1;
        slot.open = true;
        slot.device = device;
        Self {
            provider,
            index,
            generation: slot.generation,
            closed: AtomicBool::new(false),
        }
    }

    /// Whether this lease still owns its device slot — i.e. no later open has
    /// superseded it. Read-only; does not affect `begin_close`.
    pub fn is_current(&self) -> bool {
        let table = slots().lock().unwrap_or_else(|e| e.into_inner());
        table
            .get(&(self.provider, self.index))
            .is_some_and(|slot| slot.generation == self.generation)
    }

    /// Whether a handle that has not been closed yet holds `(provider, index)` — one in
    /// use, or one abandoned inside a stuck SDK call.
    ///
    /// Discovery must not open such a device. Opening it would take a lease that
    /// supersedes the live handle's, and closing it again would close the device under
    /// that handle.
    pub fn is_open(provider: &'static str, index: i32) -> bool {
        let table = slots().lock().unwrap_or_else(|e| e.into_inner());
        table.get(&(provider, index)).is_some_and(|slot| slot.open)
    }

    /// Authorize one vendor close call, or explain why not.
    ///
    /// Returns `true` exactly once, and only while this lease still owns the
    /// slot. A superseded lease returns `false` (closing would hit the newer
    /// handle's device) and so does a second call on the same lease (the
    /// explicit `close()` already ran, and `Drop` is following it).
    pub fn begin_close(&self) -> bool {
        let mut table = slots().lock().unwrap_or_else(|e| e.into_inner());
        if self.closed.swap(true, Ordering::SeqCst) {
            return false;
        }
        let key = (self.provider, self.index);
        let current = table
            .get(&key)
            .is_some_and(|slot| slot.generation == self.generation);
        if !current {
            warn!(
                provider = self.provider,
                index = self.index,
                generation = self.generation,
                "Skipping close of a superseded camera handle — the device now belongs to a newer handle"
            );
            return false;
        }
        // A unique slot is never claimed again, so a closed one would only pile up: QHY
        // discovery takes one per idle device on every camera-list refresh.
        if self.is_unique() {
            table.remove(&key);
        } else if let Some(slot) = table.get_mut(&key) {
            slot.open = false;
        }
        true
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

    /// Discovery asks this before it opens a device to read its capabilities: a device a
    /// handle still holds is open until that handle's close, and only the live lease's
    /// close counts — a superseded handle closing late leaves the newer one open.
    #[test]
    fn a_device_stays_open_until_its_current_lease_closes() {
        assert!(!DeviceLease::is_open("test-open", 0), "never opened");

        let first = DeviceLease::acquire("test-open", 0);
        assert!(DeviceLease::is_open("test-open", 0));
        assert!(!DeviceLease::is_open("test-open", 1), "another index is unaffected");

        let reopened = DeviceLease::acquire("test-open", 0);
        assert!(!first.begin_close());
        assert!(DeviceLease::is_open("test-open", 0), "a superseded close is not the device's close");

        assert!(reopened.begin_close());
        assert!(!DeviceLease::is_open("test-open", 0));
    }

    /// A named device has one open lease at a time, so discovery and a connect can never
    /// both hold one QHY device.
    #[test]
    fn a_named_device_is_claimed_by_one_lease_until_it_closes() {
        let device = "QHY268M-test";
        assert!(!DeviceLease::is_device_open("test-named", device), "never opened");

        let held = DeviceLease::try_acquire_unique_device("test-named", device).expect("free");
        assert!(DeviceLease::is_device_open("test-named", device));
        assert!(DeviceLease::try_acquire_unique_device("test-named", device).is_none());
        assert!(!DeviceLease::is_device_open("test-named", "another device"));
        assert!(!DeviceLease::is_device_open("test-named-other", device));

        assert!(held.begin_close());
        assert!(!held.begin_close(), "a lease closes once");
        assert!(!DeviceLease::is_device_open("test-named", device));
        let reclaimed =
            DeviceLease::try_acquire_unique_device("test-named", device).expect("released");
        assert!(reclaimed.begin_close());
    }

    /// The check and the claim are one step: threads racing for a free device get one lease.
    #[test]
    fn racing_claims_on_one_device_yield_one_lease() {
        let claims: Vec<_> = (0..8)
            .map(|_| {
                std::thread::spawn(|| DeviceLease::try_acquire_unique_device("test-race", "QHY-race"))
            })
            .collect();
        let leases: Vec<DeviceLease> = claims
            .into_iter()
            .filter_map(|claim| claim.join().unwrap())
            .collect();
        assert_eq!(leases.len(), 1);
        assert!(leases[0].begin_close());
    }

    /// QHY discovery opens every idle device on each camera-list refresh, and every open takes
    /// a fresh unique slot: closed ones must not pile up in the table `is_device_open` scans.
    #[test]
    fn closed_unique_leases_do_not_accumulate() {
        let table_len = || slots().lock().unwrap_or_else(|e| e.into_inner()).len();
        let before = table_len();
        for _ in 0..1_000 {
            let lease = DeviceLease::try_acquire_unique_device("test-accumulate", "QHY-refreshed")
                .expect("the previous lease closed");
            assert!(lease.begin_close());
        }
        let grown = table_len().saturating_sub(before);
        assert!(grown < 100, "{grown} closed unique slots left behind");
    }
}
