//! The settings in force, shared as immutable snapshots.

use std::sync::{Arc, RwLock, RwLockWriteGuard};

use super::CaptureSettings;

/// The settings in force.
///
/// Readers take a [`snapshot`](Self::snapshot) — an `Arc`, never a guard held across work —
/// so a capture thread never waits on a writer, never `block_on`s to read, and never sees
/// half an edit. Writers [`update`](Self::update) a copy under one write lock; a reader
/// still holding the previous snapshot keeps it.
pub struct SettingsStore {
    current: RwLock<Arc<CaptureSettings>>,
}

impl SettingsStore {
    pub fn new(settings: CaptureSettings) -> Self {
        Self {
            current: RwLock::new(Arc::new(settings)),
        }
    }

    pub fn snapshot(&self) -> Arc<CaptureSettings> {
        Arc::clone(&self.current.read().unwrap_or_else(|e| e.into_inner()))
    }

    /// Edits the settings every later snapshot sees, returning what `edit` returns. `edit`
    /// runs under the write lock: keep I/O, awaits and other locks out of it.
    pub fn update<R>(&self, edit: impl FnOnce(&mut CaptureSettings) -> R) -> R {
        edit(Arc::make_mut(&mut self.write()))
    }

    /// [`Self::update`] for an edit that may refuse: on `Err` the settings in force are
    /// left exactly as they were, however far `edit` got.
    pub fn try_update<R, E>(
        &self,
        edit: impl FnOnce(&mut CaptureSettings) -> Result<R, E>,
    ) -> Result<R, E> {
        let mut current = self.write();
        let mut edited = CaptureSettings::clone(&current);
        let result = edit(&mut edited)?;
        *current = Arc::new(edited);
        Ok(result)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Arc<CaptureSettings>> {
        self.current.write().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot is a value: an update after it is taken never reaches it.
    #[test]
    fn a_snapshot_keeps_the_settings_it_was_taken_with() {
        let store = SettingsStore::new(CaptureSettings::default());
        let before = store.snapshot();
        let gain = before.gain;

        let returned = store.update(|settings| {
            settings.gain = gain + 7;
            "edited"
        });

        assert_eq!(returned, "edited");
        assert_eq!(before.gain, gain);
        assert_eq!(store.snapshot().gain, gain + 7);
    }

    /// A refused edit is not half-applied: the store keeps the very snapshot it held.
    #[test]
    fn a_refused_edit_leaves_the_settings_in_force() {
        let store = SettingsStore::new(CaptureSettings::default());
        let before = store.snapshot();

        let refused: Result<(), &str> = store.try_update(|settings| {
            settings.gain += 7;
            Err("refused")
        });

        assert_eq!(refused, Err("refused"));
        assert!(Arc::ptr_eq(&before, &store.snapshot()), "a refusal replaced the snapshot");

        store.try_update(|settings| Ok::<_, ()>(settings.bin = 2)).unwrap();
        assert_eq!(store.snapshot().bin, 2);
        assert_eq!(before.bin, CaptureSettings::default().bin);
    }
}
