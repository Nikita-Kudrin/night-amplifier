//! Settings persistence for saving and loading capture settings
//!
//! Saves settings to a JSON file so they persist across server restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::state::{CameraCaptureProfile, CaptureSettings, EyepieceSettings, TelescopeSettings};
use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::{add_simulated_directory, get_simulated_directories, DualSamplingMode};
use crate::planetary::AlignmentRoi;
use crate::render::StretchAggressiveness;
use crate::stacking::{RejectionMethod, StackingType, WeightingPreset};

const DEFAULT_SETTINGS_FILE: &str = "settings.json";

/// Persisted settings structure matching CaptureSettings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedSettings {
    pub exposure_us: u64,
    pub gain: i32,
    pub offset: i32,
    pub bin: u8,
    pub auto_stretch: bool,
    pub stacking: bool,
    pub rejection_sigma: f32,
    #[serde(default)]
    pub rejection_method: RejectionMethod,
    pub background_subtraction: bool,
    #[serde(default)]
    pub background_extraction_algorithm: BackgroundExtractionAlgorithm,
    pub save_raw_frames: bool,
    pub save_stacked_image: bool,
    pub stacking_type: StackingType,
    #[serde(default)]
    pub weighting_preset: WeightingPreset,
    /// Auto stretch aggressiveness (Low, Medium, High)
    #[serde(default)]
    pub stretch_aggressiveness: StretchAggressiveness,
    /// Enable shadow saturation boost (defaults to false if not present)
    #[serde(default)]
    pub saturation_boost: bool,
    /// Shadow saturation boost strength (defaults to 0.5 if not present)
    #[serde(default = "default_saturation_strength")]
    pub saturation_boost_strength: f32,
    /// Use simulated camera (defaults to false if not present)
    #[serde(default)]
    pub use_simulated_camera: bool,
    /// Number of images to preload for simulated camera (defaults to 5 if not present)
    #[serde(default = "default_preload_images")]
    pub simulated_preload_images: usize,
    /// Persisted simulated camera directories (only simulated cameras are persisted)
    #[serde(default)]
    pub simulated_directories: Vec<String>,
    /// Region of interest for comet nucleus tracking
    #[serde(default)]
    pub comet_roi: Option<AlignmentRoi>,
    /// Enable "Wanderer" mode
    #[serde(default)]
    pub wanderer_mode: bool,
    /// Region of interest for planetary alignment
    #[serde(default)]
    pub planetary_roi: Option<AlignmentRoi>,
    /// Deduced field of view from successful plate solves
    #[serde(default)]
    pub push_to_fov: Option<f32>,
    #[serde(default)]
    pub eyepiece: EyepieceSettings,
    #[serde(default)]
    pub telescope: TelescopeSettings,
    /// Per-camera telescope profiles keyed by camera name
    #[serde(default)]
    pub camera_telescope_profiles: HashMap<String, TelescopeSettings>,
    /// Per-camera capture profiles keyed by `"{provider}/{model_name}"`
    #[serde(default)]
    pub camera_profiles: HashMap<String, CameraCaptureProfile>,
    /// Name of the last active camera
    #[serde(default)]
    pub last_camera_name: Option<String>,
    /// Whether the cooler should be active during capture
    #[serde(default)]
    pub cooler_enabled: bool,
    /// Target sensor temperature in Celsius
    #[serde(default)]
    pub target_temp_c: Option<f64>,
    /// Manual override for camera sensor mode (Player One dual sampling)
    #[serde(default)]
    pub sensor_mode_override: Option<DualSamplingMode>,
}

fn default_preload_images() -> usize {
    5
}

fn default_saturation_strength() -> f32 {
    0.5
}

impl From<&CaptureSettings> for PersistedSettings {
    fn from(settings: &CaptureSettings) -> Self {
        // Get current simulated directories from the registry
        let simulated_directories = get_simulated_directories()
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();

        Self {
            exposure_us: settings.exposure_us,
            gain: settings.gain,
            offset: settings.offset,
            bin: settings.bin,
            auto_stretch: settings.auto_stretch,
            stacking: settings.stacking,
            rejection_sigma: settings.rejection_sigma,
            rejection_method: settings.rejection_method,
            background_subtraction: settings.background_subtraction,
            background_extraction_algorithm: settings.background_extraction_algorithm,
            save_raw_frames: settings.save_raw_frames,
            save_stacked_image: settings.save_stacked_image,
            stacking_type: settings.stacking_type,
            weighting_preset: settings.weighting_preset,
            stretch_aggressiveness: settings.stretch_aggressiveness,
            saturation_boost: settings.saturation_boost,
            saturation_boost_strength: settings.saturation_boost_strength,
            use_simulated_camera: settings.use_simulated_camera,
            simulated_preload_images: settings.simulated_preload_images,
            simulated_directories,
            comet_roi: settings.comet_roi,
            planetary_roi: settings.planetary_roi,
            wanderer_mode: settings.wanderer_mode,
            push_to_fov: settings.push_to_fov,
            eyepiece: settings.eyepiece.clone(),
            telescope: settings.telescope.clone(),
            camera_telescope_profiles: settings.camera_telescope_profiles.clone(),
            camera_profiles: settings.camera_profiles.clone(),
            last_camera_name: settings.last_camera_name.clone(),
            cooler_enabled: settings.cooler_enabled,
            target_temp_c: settings.target_temp_c,
            sensor_mode_override: settings.sensor_mode_override,
        }
    }
}

impl From<PersistedSettings> for CaptureSettings {
    fn from(persisted: PersistedSettings) -> Self {
        Self {
            exposure_us: persisted.exposure_us,
            gain: persisted.gain,
            offset: persisted.offset,
            bin: persisted.bin,
            auto_stretch: persisted.auto_stretch,
            stacking: persisted.stacking,
            rejection_sigma: persisted.rejection_sigma,
            rejection_method: persisted.rejection_method,
            background_subtraction: persisted.background_subtraction,
            background_extraction_algorithm: persisted.background_extraction_algorithm,
            save_raw_frames: persisted.save_raw_frames,
            save_stacked_image: persisted.save_stacked_image,
            stacking_type: persisted.stacking_type,
            weighting_preset: persisted.weighting_preset,
            stretch_aggressiveness: persisted.stretch_aggressiveness,
            saturation_boost: persisted.saturation_boost,
            saturation_boost_strength: persisted.saturation_boost_strength,
            use_simulated_camera: persisted.use_simulated_camera,
            simulated_preload_images: persisted.simulated_preload_images,
            comet_roi: persisted.comet_roi,
            planetary_roi: persisted.planetary_roi,
            wanderer_mode: persisted.wanderer_mode,
            push_to_fov: persisted.push_to_fov,
            eyepiece: persisted.eyepiece,
            telescope: persisted.telescope,
            camera_telescope_profiles: persisted.camera_telescope_profiles,
            camera_profiles: persisted.camera_profiles,
            last_camera_name: persisted.last_camera_name,
            cooler_enabled: persisted.cooler_enabled,
            target_temp_c: persisted.target_temp_c,
            sensor_mode_override: persisted.sensor_mode_override,
        }
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

        match std::fs::read_to_string(&self.file_path) {
            Ok(contents) => match serde_json::from_str::<PersistedSettings>(&contents) {
                Ok(persisted) => {
                    info!("Loaded settings from {:?}", self.file_path);

                    // Restore persisted simulated camera directories
                    for dir_path in &persisted.simulated_directories {
                        let path = PathBuf::from(dir_path);
                        match add_simulated_directory(path) {
                            Ok(true) => {
                                info!(
                                    directory = %dir_path,
                                    "Restored simulated camera directory"
                                );
                            }
                            Ok(false) => {
                                debug!(
                                    directory = %dir_path,
                                    "Simulated camera directory already exists"
                                );
                            }
                            Err(e) => {
                                warn!(
                                    directory = %dir_path,
                                    error = %e,
                                    "Failed to restore simulated camera directory"
                                );
                            }
                        }
                    }

                    Some(persisted.into())
                }
                Err(e) => {
                    warn!(
                        "Failed to parse settings file {:?}: {}. Using defaults.",
                        self.file_path, e
                    );
                    None
                }
            },
            Err(e) => {
                warn!(
                    "Failed to read settings file {:?}: {}. Using defaults.",
                    self.file_path, e
                );
                None
            }
        }
    }

    /// Save settings to the JSON file
    pub fn save(&self, settings: &CaptureSettings) -> Result<(), SettingsPersistenceError> {
        let persisted = PersistedSettings::from(settings);
        let json = serde_json::to_string_pretty(&persisted)
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
