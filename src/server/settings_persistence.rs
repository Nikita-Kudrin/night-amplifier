//! Settings persistence for saving and loading capture settings
//!
//! Saves settings to a JSON file so they persist across server restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

use super::state::{
    CameraCaptureProfile, CaptureSettings, DenoiseSettings, EyepieceSettings, PreviewResolution,
    RawFrameSaving, SensorCorrectionSettings, TelescopeSettings,
};
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
    /// Which capture modes write their raw frames to disk.
    ///
    /// `None` marks a settings file written before the per-mode switches existed, which
    /// is what [`PersistedSettings::resolved_raw_frame_saving`] migrates from
    /// `save_raw_frames`. A file this version wrote always carries the group.
    #[serde(default)]
    pub raw_frame_saving: Option<RawFrameSaving>,
    /// The single pre-per-mode switch, read only to migrate a settings file written by
    /// an older build. Never written back — dropping it is what completes the migration.
    #[serde(default, skip_serializing)]
    pub save_raw_frames: Option<bool>,
    pub save_stacked_image: bool,
    pub stacking_type: StackingType,
    #[serde(default)]
    pub weighting_preset: WeightingPreset,
    /// Auto stretch aggressiveness (Low, Medium, High)
    #[serde(default)]
    pub stretch_aggressiveness: StretchAggressiveness,
    /// Auto stretch color intensity
    #[serde(default = "default_auto_stretch_intensity")]
    pub auto_stretch_intensity: f32,
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
    /// Show the focus image when waiting for frames
    #[serde(default = "default_show_focus_image")]
    pub show_focus_image: bool,
    /// Force showing the focus image even when the stream is active
    #[serde(default)]
    pub force_focus_image_now: bool,
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
    /// Enable auto tracking of planetary ROI
    #[serde(default = "default_planetary_auto_tracking")]
    pub planetary_auto_tracking: bool,
    /// Enable multi-point alignment for planetary (Pro only)
    #[serde(default)]
    pub planetary_multi_point_alignment: bool,
    // NOTE: `push_to_fov` used to live here. Plate solving is a Pro feature and now
    // persists its own solver state (position + per-rig FOV) in `push_to_state.json`,
    // which the Pro plugin owns end to end. Files written by older versions still
    // contain the key; serde ignores it, and Pro relearns the FOV on its first solve.
    #[serde(default)]
    pub eyepiece: EyepieceSettings,
    #[serde(default)]
    pub sensor_correction: SensorCorrectionSettings,
    #[serde(default)]
    pub denoise: DenoiseSettings,
    #[serde(default)]
    pub preview_resolution: PreviewResolution,
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
    /// Bypass the 5 °C/min cool/warm ramp (advanced users only)
    #[serde(default)]
    pub cooler_fast_mode: bool,
    #[serde(default)]
    pub sensor_mode_override: Option<DualSamplingMode>,
    /// Whether anti-dew heater is enabled
    #[serde(default = "default_dew_heater_enabled")]
    pub dew_heater_enabled: bool,
    /// Anti-dew heater power level (0-100)
    #[serde(default = "default_dew_heater_power")]
    pub dew_heater_power: i32,
    /// Reopen the camera automatically after an unexpected dropout.
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    /// Resume the interrupted capture after an automatic reconnect.
    #[serde(default = "default_true")]
    pub auto_resume_capture: bool,
    /// Whether the user has accepted the End User License Agreement
    #[serde(default)]
    pub eula_accepted: bool,
    #[serde(default = "default_indi_server_host")]
    pub indi_server_host: String,
    #[serde(default = "default_indi_server_port")]
    pub indi_server_port: u16,
}

fn default_true() -> bool {
    true
}

fn default_indi_server_host() -> String {
    "127.0.0.1".to_string()
}

fn default_indi_server_port() -> u16 {
    7624
}

fn default_dew_heater_enabled() -> bool {
    true
}

fn default_dew_heater_power() -> i32 {
    10
}

fn default_preload_images() -> usize {
    5
}

fn default_show_focus_image() -> bool {
    true
}

fn default_saturation_strength() -> f32 {
    0.5
}

fn default_auto_stretch_intensity() -> f32 {
    0.3
}

fn default_planetary_auto_tracking() -> bool {
    true
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
            raw_frame_saving: Some(settings.raw_frame_saving),
            save_raw_frames: None,
            save_stacked_image: settings.save_stacked_image,
            stacking_type: settings.stacking_type,
            weighting_preset: settings.weighting_preset,
            stretch_aggressiveness: settings.stretch_aggressiveness,
            auto_stretch_intensity: settings.auto_stretch_intensity,
            saturation_boost: settings.saturation_boost,
            saturation_boost_strength: settings.saturation_boost_strength,
            use_simulated_camera: settings.use_simulated_camera,
            simulated_preload_images: settings.simulated_preload_images,
            show_focus_image: settings.show_focus_image,
            force_focus_image_now: settings.force_focus_image_now,
            simulated_directories,
            comet_roi: settings.comet_roi,
            planetary_roi: settings.planetary_roi,
            planetary_auto_tracking: settings.planetary_auto_tracking,
            planetary_multi_point_alignment: settings.planetary_multi_point_alignment,
            wanderer_mode: settings.wanderer_mode,
            eyepiece: settings.eyepiece.clone(),
            sensor_correction: settings.sensor_correction.clone(),
            denoise: settings.denoise.clone(),
            preview_resolution: settings.preview_resolution,
            telescope: settings.telescope.clone(),
            camera_telescope_profiles: settings.camera_telescope_profiles.clone(),
            camera_profiles: settings.camera_profiles.clone(),
            last_camera_name: settings.last_camera_name.clone(),
            cooler_enabled: settings.cooler_enabled,
            target_temp_c: settings.target_temp_c,
            cooler_fast_mode: settings.cooler_fast_mode,
            sensor_mode_override: settings.sensor_mode_override,
            dew_heater_enabled: settings.dew_heater_enabled,
            dew_heater_power: settings.dew_heater_power,
            auto_reconnect: settings.auto_reconnect,
            auto_resume_capture: settings.auto_resume_capture,
            eula_accepted: settings.eula_accepted,
            indi_server_host: settings.indi_server_host.clone(),
            indi_server_port: settings.indi_server_port,
        }
    }
}

impl PersistedSettings {
    /// The per-mode selection this file describes, migrating an older file on the way.
    ///
    /// A pre-per-mode build only ever saved raw frames in Stacking — the gate was
    /// `stacking && !wanderer_mode` — so that is the one switch a legacy `true` may
    /// turn on. Without this an upgrade reads the old key as an unknown field and
    /// silently drops it, and the first save writes the loss back out for good.
    fn resolved_raw_frame_saving(&self) -> RawFrameSaving {
        if let Some(raw_frame_saving) = self.raw_frame_saving {
            return raw_frame_saving;
        }
        RawFrameSaving {
            stacking: self.save_raw_frames.unwrap_or(false),
            ..RawFrameSaving::default()
        }
    }
}

impl From<PersistedSettings> for CaptureSettings {
    fn from(persisted: PersistedSettings) -> Self {
        let raw_frame_saving = persisted.resolved_raw_frame_saving();
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
            raw_frame_saving,
            save_stacked_image: persisted.save_stacked_image,
            stacking_type: persisted.stacking_type,
            weighting_preset: persisted.weighting_preset,
            stretch_aggressiveness: persisted.stretch_aggressiveness,
            auto_stretch_intensity: persisted.auto_stretch_intensity,
            saturation_boost: persisted.saturation_boost,
            saturation_boost_strength: persisted.saturation_boost_strength,
            use_simulated_camera: persisted.use_simulated_camera,
            simulated_preload_images: persisted.simulated_preload_images,
            show_focus_image: persisted.show_focus_image,
            force_focus_image_now: persisted.force_focus_image_now,
            comet_roi: persisted.comet_roi,
            planetary_roi: persisted.planetary_roi,
            planetary_auto_tracking: persisted.planetary_auto_tracking,
            planetary_multi_point_alignment: persisted.planetary_multi_point_alignment,
            wanderer_mode: persisted.wanderer_mode,
            eyepiece: persisted.eyepiece,
            sensor_correction: persisted.sensor_correction,
            denoise: persisted.denoise,
            preview_resolution: persisted.preview_resolution,
            telescope: persisted.telescope,
            camera_telescope_profiles: persisted.camera_telescope_profiles,
            camera_profiles: persisted.camera_profiles,
            last_camera_name: persisted.last_camera_name,
            cooler_enabled: persisted.cooler_enabled,
            target_temp_c: persisted.target_temp_c,
            cooler_fast_mode: persisted.cooler_fast_mode,
            sensor_mode_override: persisted.sensor_mode_override,
            dew_heater_enabled: persisted.dew_heater_enabled,
            dew_heater_power: persisted.dew_heater_power,
            auto_reconnect: persisted.auto_reconnect,
            auto_resume_capture: persisted.auto_resume_capture,
            eula_accepted: persisted.eula_accepted,
            indi_server_host: persisted.indi_server_host,
            indi_server_port: persisted.indi_server_port,
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
