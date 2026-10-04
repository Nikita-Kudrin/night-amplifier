//! Settings persistence for saving and loading capture settings
//!
//! Saves settings to a JSON file so they persist across server restarts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{debug, info, warn};

use super::state::{CaptureSettings, RawFrameSaving};
use crate::camera::{add_simulated_directory, get_simulated_directories};

pub const DEFAULT_SETTINGS_FILE: &str = "settings.json";

/// What `settings.json` holds: the settings, plus the simulated-camera directories, which
/// live in the camera registry rather than in `CaptureSettings`.
pub(crate) struct SettingsFile {
    pub settings: CaptureSettings,
    pub simulated_directories: Vec<String>,
}

impl SettingsFile {
    /// Reads a file's JSON, migrating whatever an older build wrote on the way.
    pub(crate) fn from_json(mut value: Value) -> serde_json::Result<Self> {
        let Some(file) = value.as_object_mut() else {
            return Err(serde::de::Error::custom("a settings file is a JSON object"));
        };
        migrate(file);
        let simulated_directories = match file.remove("simulated_directories") {
            Some(directories) => Vec::deserialize(directories)?,
            None => Vec::new(),
        };
        Ok(Self {
            settings: CaptureSettings::deserialize(value)?.sanitized(),
            simulated_directories,
        })
    }

    /// The file's text. Serialised directly rather than through a `Value`, which would
    /// widen every `f32` and write `0.3` as `0.30000001192092896`.
    pub(crate) fn to_json(&self) -> serde_json::Result<String> {
        #[derive(Serialize)]
        struct Written<'a> {
            #[serde(flatten)]
            settings: &'a CaptureSettings,
            simulated_directories: &'a [String],
        }
        let settings = self.settings.clone().sanitized();
        serde_json::to_string_pretty(&Written {
            settings: &settings,
            simulated_directories: &self.simulated_directories,
        })
    }
}

/// Rewrites the keys an older build wrote into the shape `CaptureSettings` reads.
///
/// `save_raw_frames` was the single switch before the per-mode ones. A pre-per-mode build
/// only ever saved raw frames in Stacking — the gate was `stacking && !wanderer_mode` — so
/// that is the one switch a legacy `true` may turn on. Without this an upgrade reads the
/// old key as an unknown field and drops it, and the first save writes the loss back.
fn migrate(file: &mut Map<String, Value>) {
    let legacy_raw_saving = file.remove("save_raw_frames").and_then(|v| v.as_bool());
    if file.get("raw_frame_saving").is_none_or(Value::is_null) {
        let saving = RawFrameSaving {
            stacking: legacy_raw_saving.unwrap_or(false),
            ..RawFrameSaving::default()
        };
        let saving = serde_json::to_value(saving).expect("a struct of bools serialises");
        file.insert("raw_frame_saving".to_string(), saving);
    }
}

/// Settings persistence manager
#[derive(Debug, Clone)]
pub struct SettingsPersistence {
    file_path: PathBuf,
}

impl Default for SettingsPersistence {
    fn default() -> Self {
        Self::new(DEFAULT_SETTINGS_FILE)
    }
}

impl SettingsPersistence {
    /// Create a new settings persistence manager with the given file path
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            file_path: path.as_ref().to_path_buf(),
        }
    }

    /// Load settings from the JSON file
    ///
    /// Returns None if the file doesn't exist or cannot be parsed.
    /// Also restores persisted simulated camera directories.
    pub fn load(&self) -> Option<CaptureSettings> {
        if !self.file_path.exists() {
            debug!(
                "Settings file not found at {:?}, using defaults",
                self.file_path
            );
            return None;
        }

        let parsed = std::fs::read_to_string(&self.file_path)
            .map_err(|e| format!("Failed to read settings file {:?}: {e}", self.file_path))
            .and_then(|contents| {
                serde_json::from_str(&contents)
                    .and_then(SettingsFile::from_json)
                    .map_err(|e| format!("Failed to parse settings file {:?}: {e}", self.file_path))
            });
        let file = match parsed {
            Ok(file) => file,
            Err(message) => {
                warn!("{message}. Using defaults.");
                return None;
            }
        };
        info!("Loaded settings from {:?}", self.file_path);
        restore_simulated_directories(&file.simulated_directories);
        Some(file.settings)
    }

    /// Save settings to the JSON file
    pub fn save(&self, settings: &CaptureSettings) -> Result<(), SettingsPersistenceError> {
        let file = SettingsFile {
            settings: settings.clone(),
            simulated_directories: get_simulated_directories()
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
        };
        let json = file
            .to_json()
            .map_err(|e| SettingsPersistenceError::SerializationFailed(e.to_string()))?;

        std::fs::write(&self.file_path, json)
            .map_err(|e| SettingsPersistenceError::WriteFailed(e.to_string()))?;

        debug!("Saved settings to {:?}", self.file_path);
        Ok(())
    }

    /// Get the path to the settings file
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }
}

/// Re-registers the simulated cameras a previous run had, one directory at a time.
fn restore_simulated_directories(directories: &[String]) {
    for dir_path in directories {
        match add_simulated_directory(PathBuf::from(dir_path)) {
            Ok(true) => info!(directory = %dir_path, "Restored simulated camera directory"),
            Ok(false) => debug!(directory = %dir_path, "Simulated camera directory already exists"),
            Err(e) => warn!(
                directory = %dir_path,
                error = %e,
                "Failed to restore simulated camera directory"
            ),
        }
    }
}

/// Errors that can occur during settings persistence
#[derive(Debug, thiserror::Error)]
pub enum SettingsPersistenceError {
    #[error("Failed to serialize settings: {0}")]
    SerializationFailed(String),
    #[error("Failed to write settings file: {0}")]
    WriteFailed(String),
}

#[cfg(test)]
#[path = "settings_persistence_tests.rs"]
mod tests;
