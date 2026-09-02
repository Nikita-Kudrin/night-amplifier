//! Tests for settings persistence

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::NamedTempFile;

    use crate::background::BackgroundExtractionAlgorithm;
    use crate::render::StretchAggressiveness;
    use crate::server::settings_persistence::{PersistedSettings, SettingsPersistence};
    use crate::server::state::{
        CameraCaptureProfile, CaptureSettings, DenoiseSettings, EyepieceSettings, PreviewResolution,
        SensorCorrectionSettings, TelescopeSettings,
    };
    use crate::stacking::{RejectionMethod, StackingType, WeightingPreset};

    #[test]
    fn test_persisted_settings_roundtrip() {
        let settings = CaptureSettings {
            auto_reconnect: true,
            auto_resume_capture: true,
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
            auto_stretch_intensity: 0.3,
            saturation_boost: true,
            saturation_boost_strength: 0.7,
            use_simulated_camera: true,
            simulated_preload_images: 10,
            show_focus_image: true,
            force_focus_image_now: false,
            comet_roi: None,
            cooler_enabled: true,
            planetary_roi: None,
            planetary_auto_tracking: false,
            planetary_multi_point_alignment: true,
            dew_heater_enabled: true,
            dew_heater_power: 30,
            wanderer_mode: true,
            sensor_correction: SensorCorrectionSettings {
                hot_pixel_rejection: false,
                hot_pixel_sigma: 7.5,
                fpn_removal: false,
                superpixel_debayer: true,
            },
            preview_resolution: PreviewResolution::Qhd1440,
            denoise: DenoiseSettings {
                chroma: true,
                chroma_strength: 0.8,
                luma: false,
                luma_strength: 0.5,
                star_protection: 0.25,
            },
            eyepiece: EyepieceSettings {
                binoview: true,
                screen_width: 140.0,
                screen_height: 67.0,
                screen_measurement: "mm".to_string(),
                screen_resolution_x: 2880,
                screen_resolution_y: 1440,
                circular_view: true,
                intensity: 0.5,
                black_floor: 0.04,
                darker_sky: false,
                dither: true,
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
            camera_profiles: HashMap::from([
                (
                    "PlayerOne/Neptune-C II".to_string(),
                    CameraCaptureProfile {
                        exposure_us: 500_000,
                        gain: 220,
                        offset: 8,
                        bin: 1,
                        cooler_enabled: false,
                        target_temp_c: None,
                        sensor_mode_override: None,
                        cooler_fast_mode: false,
                        dew_heater_enabled: true,
                        dew_heater_power: 15,
                    },
                ),
                (
                    "ZWO/ASI2600MC".to_string(),
                    CameraCaptureProfile {
                        exposure_us: 10_000_000,
                        gain: 100,
                        offset: 50,
                        bin: 1,
                        cooler_enabled: true,
                        target_temp_c: Some(-10.0),
                        sensor_mode_override: None,
                        cooler_fast_mode: true,
                        dew_heater_enabled: false,
                        dew_heater_power: 0,
                    },
                ),
            ]),
            last_camera_name: Some("Neptune-C II".to_string()),
            target_temp_c: Some(-10.0),
            cooler_fast_mode: true,
            sensor_mode_override: Some(crate::camera::DualSamplingMode::LowReadoutNoise),
            eula_accepted: false,
            indi_server_host: "127.0.0.1".to_string(),
            indi_server_port: 7624,
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
        assert!(
            (restored.auto_stretch_intensity - settings.auto_stretch_intensity).abs()
                < f32::EPSILON
        );
        assert_eq!(restored.saturation_boost, settings.saturation_boost);
        assert!(
            (restored.saturation_boost_strength - settings.saturation_boost_strength).abs()
                < f32::EPSILON
        );
        assert_eq!(restored.use_simulated_camera, settings.use_simulated_camera);
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
            restored.eyepiece.circular_view,
            settings.eyepiece.circular_view
        );
        assert_eq!(restored.eyepiece.intensity, settings.eyepiece.intensity);
        assert_eq!(restored.sensor_correction, settings.sensor_correction);
        assert_eq!(restored.denoise, settings.denoise);
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
        assert_eq!(restored.camera_profiles.len(), 2);
        let neptune_profile = restored
            .camera_profiles
            .get("PlayerOne/Neptune-C II")
            .expect("Neptune-C II profile should be persisted");
        assert_eq!(neptune_profile.exposure_us, 500_000);
        assert_eq!(neptune_profile.gain, 220);
        assert!(!neptune_profile.cooler_enabled);
        assert_eq!(neptune_profile.target_temp_c, None);
        let zwo_profile = restored
            .camera_profiles
            .get("ZWO/ASI2600MC")
            .expect("ASI2600MC profile should be persisted");
        assert_eq!(zwo_profile.gain, 100);
        assert!(zwo_profile.cooler_enabled);
        assert_eq!(zwo_profile.target_temp_c, Some(-10.0));
        assert_eq!(restored.last_camera_name, Some("Neptune-C II".to_string()));
        assert!(restored.cooler_enabled);
        assert_eq!(restored.target_temp_c, Some(-10.0));
        assert!(restored.dew_heater_enabled);
        assert_eq!(restored.dew_heater_power, 30);
        assert_eq!(
            restored.planetary_auto_tracking,
            settings.planetary_auto_tracking
        );
        assert_eq!(
            restored.planetary_multi_point_alignment,
            settings.planetary_multi_point_alignment
        );
    }

    #[test]
    fn test_save_and_load_settings() {
        let temp_file = NamedTempFile::new().unwrap();
        let persistence = SettingsPersistence::new(temp_file.path());

        let settings = CaptureSettings {
            auto_reconnect: true,
            auto_resume_capture: true,
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
            auto_stretch_intensity: 0.3,
            saturation_boost: true,
            saturation_boost_strength: 0.6,
            use_simulated_camera: true,
            simulated_preload_images: 2,
            show_focus_image: false,
            force_focus_image_now: false,
            comet_roi: None,
            wanderer_mode: true,
            planetary_roi: None,
            planetary_auto_tracking: true,
            planetary_multi_point_alignment: false,
            sensor_correction: SensorCorrectionSettings::default(),
            denoise: DenoiseSettings::default(),
            preview_resolution: PreviewResolution::default(),
            eyepiece: EyepieceSettings {
                binoview: true,
                screen_width: 140.0,
                screen_height: 67.0,
                screen_measurement: "mm".to_string(),
                screen_resolution_x: 2880,
                screen_resolution_y: 1440,
                circular_view: true,
                intensity: 0.5,
                black_floor: 0.04,
                darker_sky: false,
                dither: true,
            },
            telescope: TelescopeSettings::default(),
            camera_telescope_profiles: HashMap::new(),
            camera_profiles: HashMap::new(),
            last_camera_name: None,
            cooler_enabled: false,
            target_temp_c: None,
            cooler_fast_mode: false,
            sensor_mode_override: None,
            dew_heater_enabled: false,
            dew_heater_power: 0,
            eula_accepted: false,
            indi_server_host: "127.0.0.1".to_string(),
            indi_server_port: 7624,
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
        assert_eq!(loaded.eyepiece.binoview, settings.eyepiece.binoview);
        assert_eq!(
            loaded.eyepiece.circular_view,
            settings.eyepiece.circular_view
        );
        assert_eq!(loaded.eyepiece.intensity, settings.eyepiece.intensity);
        assert_eq!(
            loaded.planetary_auto_tracking,
            settings.planetary_auto_tracking
        );
        assert_eq!(
            loaded.planetary_multi_point_alignment,
            settings.planetary_multi_point_alignment
        );
    }

    #[test]
    fn test_load_settings_without_planetary_fields() {
        // Verify backward compatibility: a settings.json from before the planetary
        // tracking commits (missing the fields) deserializes with correct defaults.
        let json = serde_json::json!({
            "exposure_us": 1000,
            "gain": 100,
            "offset": 10,
            "bin": 1,
            "auto_stretch": true,
            "stacking": false,
            "rejection_sigma": 2.5,
            "rejection_method": "None",
            "background_subtraction": true,
            "background_extraction_algorithm": "grid_bilinear",
            "save_raw_frames": false,
            "save_stacked_image": false,
            "stacking_type": "deep_sky",
            "weighting_preset": "balanced",
            "stretch_aggressiveness": "medium",
            "saturation_boost": false,
            "saturation_boost_strength": 0.5,
            "use_simulated_camera": false,
            "simulated_preload_images": 5,
            "wanderer_mode": false
        });

        let persisted: PersistedSettings = serde_json::from_value(json).unwrap();
        assert!(
            persisted.planetary_auto_tracking,
            "Missing field should default to true"
        );
        assert!(
            !persisted.planetary_multi_point_alignment,
            "Missing field should default to false"
        );
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

    #[test]
    fn settings_written_by_an_older_version_still_load() {
        // `push_to_fov` was removed when Push-To moved its solver state into the Pro
        // plugin's own file. Every existing installation has that key on disk, so
        // loading must ignore it rather than fall back to defaults and silently wipe
        // the user's configuration. This is the guarantee to keep if `PersistedSettings`
        // ever gains `#[serde(deny_unknown_fields)]`.
        let temp_file = NamedTempFile::new().unwrap();
        let persistence = SettingsPersistence::new(temp_file.path());
        persistence.save(&CaptureSettings::default()).unwrap();

        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(temp_file.path()).unwrap()).unwrap();
        json["push_to_fov"] = serde_json::json!(0.2032);
        json["some_future_unknown_key"] = serde_json::json!("whatever");
        json["exposure_us"] = serde_json::json!(1_234_567u64);
        std::fs::write(
            temp_file.path(),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();

        let loaded = persistence
            .load()
            .expect("settings containing a removed key must still load");
        assert_eq!(loaded.exposure_us, 1_234_567);
    }

    #[test]
    fn saved_settings_no_longer_carry_push_to_fov() {
        let temp_file = NamedTempFile::new().unwrap();
        let persistence = SettingsPersistence::new(temp_file.path());
        persistence.save(&CaptureSettings::default()).unwrap();

        let contents = std::fs::read_to_string(temp_file.path()).unwrap();
        assert!(
            !contents.contains("push_to_fov"),
            "Pro solver state must not leak back into the Community settings file"
        );
    }
}
