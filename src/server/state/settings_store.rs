//! The settings in force, shared as immutable snapshots.

use std::sync::{Arc, RwLock};

use super::CaptureSettings;

/// The settings in force.
///
/// Readers take a [`snapshot`](Self::snapshot) — an `Arc`, never a guard held across work —
/// so a capture thread never waits on a writer, never `block_on`s to read, and never sees
/// half an edit. Writers [`update`](Self::update) a copy under one write lock; a reader
/// still holding the previous snapshot keeps it. Every update bumps the
/// [`version`](Self::version), so a per-frame cache can tell "unchanged" without comparing.
pub struct SettingsStore {
    current: RwLock<Versioned>,
}

struct Versioned {
    version: u64,
    settings: Arc<CaptureSettings>,
}

impl SettingsStore {
    pub fn new(settings: CaptureSettings) -> Self {
        Self {
            current: RwLock::new(Versioned {
                version: 0,
                settings: Arc::new(settings),
            }),
        }
    }

    pub fn snapshot(&self) -> Arc<CaptureSettings> {
        Arc::clone(&self.read().settings)
    }

    /// The snapshot with the version it carries.
    pub fn versioned(&self) -> (u64, Arc<CaptureSettings>) {
        let current = self.read();
        (current.version, Arc::clone(&current.settings))
    }

    pub fn version(&self) -> u64 {
        self.read().version
    }

    /// Edits the settings every later snapshot sees, returning what `edit` returns. `edit`
    /// runs under the write lock: keep I/O, awaits and other locks out of it.
    pub fn update<R>(&self, edit: impl FnOnce(&mut CaptureSettings) -> R) -> R {
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        let result = edit(Arc::make_mut(&mut current.settings));
        current.version += 1;
        result
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Versioned> {
        self.current.read().unwrap_or_else(|e| e.into_inner())
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

        store.update(|settings| settings.gain = gain + 7);

        assert_eq!(before.gain, gain);
        assert_eq!(store.snapshot().gain, gain + 7);
    }

    #[test]
    fn every_update_bumps_the_version_the_snapshot_carries() {
        let store = SettingsStore::new(CaptureSettings::default());
        let (first, _) = store.versioned();
        let returned = store.update(|settings| {
            settings.bin = 2;
            "edited"
        });
        let (second, settings) = store.versioned();

        assert_eq!(returned, "edited");
        assert_eq!(second, first + 1);
        assert_eq!(settings.bin, 2);
        assert_eq!(store.version(), second);
    }
}
