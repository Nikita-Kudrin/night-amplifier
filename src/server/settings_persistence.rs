//! Settings persistence for saving and loading capture settings
//!
//! Saves settings to a JSON file so they persist across server restarts.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::state::{CaptureSettings, EyepieceSettings, TelescopeSettings};
use crate::background::BackgroundExtractionAlgorithm;
use crate::camera::{add_simulated_directory, get_simulated_directories};
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
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_persisted_settings_roundtrip() {
        let settings = CaptureSettings {
            exposure_us: 2_000_000,
            gain: 150,
            offset: 20,
            bin: 2,
            auto_stretch: false,
            stacking: false,
            rejection_sigma: 3.0,
            rejection_method: RejectionMethod::SigmaClip,
            background_subtraction: false,
            background_extraction_algorithm: BackgroundExtractionAlgorithm::default(),
            save_raw_frames: true,
            save_stacked_image: true,
            stacking_type: StackingType::Planetary,
            weighting_preset: WeightingPreset::Galaxies,
            stretch_aggressiveness: StretchAggressiveness::High,
            saturation_boost: true,
            saturation_boost_strength: 0.7,
            use_simulated_camera: true,
            simulated_preload_images: 10,
            comet_roi: None,
            wanderer_mode: true,
            push_to_fov: Some(2.5),
            planetary_roi: None,
            eyepiece: EyepieceSettings {
                binoview: true,
                screen_width: 140.0,
                screen_height: 67.0,
                screen_measurement: "mm".to_string(),
                screen_resolution_x: 2880,
                screen_resolution_y: 1440,
            },
            telescope: TelescopeSettings {
                focal_length_mm: Some(1000.0),
                pixel_size_x_um: Some(3.76),
                pixel_size_y_um: Some(3.76),
                sensor_width_px: Some(3008),
                sensor_height_px: Some(3008),
                barlow_coeff: Some(1.0),
            },
        };

        let persisted = PersistedSettings::from(&settings);
        let restored: CaptureSettings = persisted.into();

        assert_eq!(restored.exposure_us, settings.exposure_us);
        assert_eq!(restored.gain, settings.gain);
        assert_eq!(restored.offset, settings.offset);
        assert_eq!(restored.bin, settings.bin);
        assert_eq!(restored.auto_stretch, settings.auto_stretch);
        assert_eq!(restored.stacking, settings.stacking);
        assert_eq!(restored.rejection_method, settings.rejection_method);
        assert!((restored.rejection_sigma - settings.rejection_sigma).abs() < f32::EPSILON);
        assert_eq!(
            restored.background_subtraction,
            settings.background_subtraction
        );
        assert_eq!(restored.save_raw_frames, settings.save_raw_frames);
        assert_eq!(restored.save_stacked_image, settings.save_stacked_image);
        assert_eq!(restored.stacking_type, settings.stacking_type);
        assert_eq!(restored.weighting_preset, settings.weighting_preset);
        assert_eq!(
            restored.stretch_aggressiveness,
            settings.stretch_aggressiveness
        );
        assert_eq!(restored.saturation_boost, settings.saturation_boost);
        assert!(
            (restored.saturation_boost_strength - settings.saturation_boost_strength).abs()
                < f32::EPSILON
        );
        assert_eq!(restored.use_simulated_camera, settings.use_simulated_camera);
        assert_eq!(restored.push_to_fov, settings.push_to_fov);
        assert_eq!(restored.eyepiece.binoview, settings.eyepiece.binoview);
        assert_eq!(
            restored.eyepiece.screen_width,
            settings.eyepiece.screen_width
        );
        assert_eq!(
            restored.eyepiece.screen_height,
            settings.eyepiece.screen_height
        );
        assert_eq!(
            restored.eyepiece.screen_measurement,
            settings.eyepiece.screen_measurement
        );
        assert_eq!(
            restored.eyepiece.screen_resolution_x,
            settings.eyepiece.screen_resolution_x
        );
        assert_eq!(
            restored.eyepiece.screen_resolution_y,
            settings.eyepiece.screen_resolution_y
        );
        assert_eq!(
            restored.telescope.focal_length_mm,
            settings.telescope.focal_length_mm
        );
        assert_eq!(
            restored.telescope.pixel_size_x_um,
            settings.telescope.pixel_size_x_um
        );
        assert_eq!(
            restored.telescope.barlow_coeff,
            settings.telescope.barlow_coeff
        );
        assert_eq!(
            restored.telescope.sensor_width_px,
            settings.telescope.sensor_width_px
        );
    }

    #[test]
    fn test_save_and_load_settings() {
        let temp_file = NamedTempFile::new().unwrap();
        let persistence = SettingsPersistence::new(temp_file.path());

        let settings = CaptureSettings {
            exposure_us: 3_000_000,
            gain: 200,
            offset: 15,
            bin: 1,
            auto_stretch: true,
            stacking: true,
            rejection_sigma: 2.8,
            rejection_method: RejectionMethod::WinsorizedSigmaClip,
            background_subtraction: true,
            background_extraction_algorithm: BackgroundExtractionAlgorithm::Rbf,
            save_raw_frames: false,
            save_stacked_image: true,
            stacking_type: StackingType::DeepSky,
            weighting_preset: WeightingPreset::Nebulae,
            stretch_aggressiveness: StretchAggressiveness::High,
            saturation_boost: true,
            saturation_boost_strength: 0.6,
            use_simulated_camera: true,
            simulated_preload_images: 12,
            comet_roi: None,
            wanderer_mode: true,
            push_to_fov: None,
            planetary_roi: None,
            eyepiece: EyepieceSettings {
                binoview: true,
                screen_width: 140.0,
                screen_height: 67.0,
                screen_measurement: "mm".to_string(),
                screen_resolution_x: 2880,
                screen_resolution_y: 1440,
            },
            telescope: TelescopeSettings::default(),
        };

        persistence.save(&settings).unwrap();
        let loaded = persistence.load().unwrap();

        assert_eq!(loaded.exposure_us, settings.exposure_us);
        assert_eq!(loaded.rejection_method, settings.rejection_method);
        assert_eq!(loaded.gain, settings.gain);
        assert_eq!(loaded.stacking_type, settings.stacking_type);
        assert_eq!(loaded.weighting_preset, settings.weighting_preset);
        assert_eq!(
            loaded.stretch_aggressiveness,
            settings.stretch_aggressiveness
        );
        assert_eq!(loaded.saturation_boost, settings.saturation_boost);
        assert!(
            (loaded.saturation_boost_strength - settings.saturation_boost_strength).abs()
                < f32::EPSILON
        );
        assert_eq!(loaded.use_simulated_camera, settings.use_simulated_camera);
        assert_eq!(loaded.push_to_fov, settings.push_to_fov);
        assert_eq!(loaded.eyepiece.binoview, settings.eyepiece.binoview);
    }

    #[test]
    fn test_load_nonexistent_file() {
        let persistence = SettingsPersistence::new("/nonexistent/path/settings.json");
        assert!(persistence.load().is_none());
    }

    #[test]
    fn test_load_invalid_json() {
        let temp_file = NamedTempFile::new().unwrap();
        std::fs::write(temp_file.path(), "invalid json content").unwrap();

        let persistence = SettingsPersistence::new(temp_file.path());
        assert!(persistence.load().is_none());
    }

    #[test]
    fn test_json_format() {
        let temp_file = NamedTempFile::new().unwrap();
        let persistence = SettingsPersistence::new(temp_file.path());

        let settings = CaptureSettings::default();
        persistence.save(&settings).unwrap();

        let contents = std::fs::read_to_string(temp_file.path()).unwrap();
        assert!(contents.contains("exposure_us"));
        assert!(contents.contains("gain"));
        assert!(contents.contains("stacking_type"));
        assert!(contents.contains("weighting_preset"));
        assert!(contents.contains("use_simulated_camera"));

        // Verify it's valid JSON
        let _: serde_json::Value = serde_json::from_str(&contents).unwrap();
    }
}
