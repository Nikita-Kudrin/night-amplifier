//! Tests for settings persistence

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::NamedTempFile;

    use crate::background::BackgroundExtractionAlgorithm;
    use crate::render::StretchAggressiveness;
    use crate::server::settings_persistence::{PersistedSettings, SettingsPersistence};
    use crate::server::state::{CaptureSettings, EyepieceSettings, TelescopeSettings};
    use crate::stacking::{RejectionMethod, StackingType, WeightingPreset};

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
            camera_telescope_profiles: HashMap::from([(
                "Neptune-C II".to_string(),
                TelescopeSettings {
                    focal_length_mm: Some(1000.0),
                    pixel_size_x_um: Some(2.9),
                    pixel_size_y_um: Some(2.9),
                    sensor_width_px: Some(2712),
                    sensor_height_px: Some(1538),
                    barlow_coeff: Some(1.0),
                },
            )]),
            last_camera_name: Some("Neptune-C II".to_string()),
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
        assert_eq!(restored.camera_telescope_profiles.len(), 1);
        assert!(restored
            .camera_telescope_profiles
            .contains_key("Neptune-C II"));
        assert_eq!(restored.last_camera_name, Some("Neptune-C II".to_string()));
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
            camera_telescope_profiles: HashMap::new(),
            last_camera_name: None,
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
