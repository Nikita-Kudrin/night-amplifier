use super::*;
use crate::CfaPattern;

#[test]
fn test_capture_config_defaults() {
    let config = CaptureConfig::default();
    assert_eq!(config.exposure_us, 1_000_000);
    assert_eq!(config.gain, 0);
    assert_eq!(config.bin, 1);
    assert_eq!(config.format, ImageFormat::Raw16);
    assert!(config.roi.is_none());
}

#[test]
fn test_capture_config_builder() {
    let config = CaptureConfig::new()
        .with_exposure_us(500_000)
        .with_gain(100)
        .with_offset(20)
        .with_bin(2)
        .with_format(ImageFormat::Raw8)
        .with_roi(100, 100, 800, 600)
        .with_target_temp(-10.0)
        .with_timeout(Duration::from_secs(60));

    assert_eq!(config.exposure_us, 500_000);
    assert_eq!(config.gain, 100);
    assert_eq!(config.offset, 20);
    assert_eq!(config.bin, 2);
    assert_eq!(config.format, ImageFormat::Raw8);
    assert_eq!(config.roi, Some((100, 100, 800, 600)));
    assert_eq!(config.target_temp_c, Some(-10.0));
    assert!(config.cooler_enabled);
    assert_eq!(config.timeout, Duration::from_secs(60));
}

#[test]
fn test_capture_config_from_duration() {
    let config = CaptureConfig::new().with_exposure(Duration::from_secs(5));
    assert_eq!(config.exposure_us, 5_000_000);
}

#[test]
fn test_image_format_bytes_per_pixel() {
    assert_eq!(ImageFormat::Raw8.bytes_per_pixel(), 1);
    assert_eq!(ImageFormat::Raw16.bytes_per_pixel(), 2);
    assert_eq!(ImageFormat::Rgb24.bytes_per_pixel(), 3);
}

#[test]
fn best_raw_format_prefers_raw16() {
    let supported = vec![ImageFormat::Raw8, ImageFormat::Raw16, ImageFormat::Rgb24];
    assert_eq!(
        ImageFormat::best_raw_format(&supported),
        Some(ImageFormat::Raw16)
    );
}

#[test]
fn best_raw_format_falls_back_to_raw8() {
    let supported = vec![ImageFormat::Raw8, ImageFormat::Rgb24];
    assert_eq!(
        ImageFormat::best_raw_format(&supported),
        Some(ImageFormat::Raw8)
    );
}

#[test]
fn best_raw_format_returns_none_when_no_raw_format() {
    let supported = vec![ImageFormat::Rgb24];
    assert_eq!(ImageFormat::best_raw_format(&supported), None);
}

#[test]
fn test_capture_config_validation() {
    let info = CameraInfo {
        name: "Test Camera".to_string(),
        id: 0,
        max_width: 1920,
        max_height: 1080,
        pixel_size_x_um: 2.9,
        pixel_size_y_um: 2.9,
        sensor_type: SensorType::Color,
        bayer_pattern: Some(CfaPattern::Rggb),
        has_cooler: false,
        min_temp_c: None,
        max_temp_c: None,
        has_shutter: false,
        is_usb3: true,
        bit_depth: 12,
        supported_bins: vec![1, 2, 4],
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
        min_exposure_us: 100,
        max_exposure_us: 3600_000_000,
        min_gain: 0,
        max_gain: 500,
        unity_gain: 100,
        hcg_gain: 120,
        sensor_modes: Vec::new(),
    };

    // Valid config
    let config = CaptureConfig::default();
    assert!(config.validate(&info).is_ok());

    // Invalid exposure (too low)
    let config = CaptureConfig::new().with_exposure_us(1);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "exposure_us"
    ));

    // Invalid gain
    let config = CaptureConfig::new().with_gain(1000);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "gain"
    ));

    // Invalid binning
    let config = CaptureConfig::new().with_bin(3);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "bin"
    ));

    // Invalid format
    let config = CaptureConfig::new().with_format(ImageFormat::Rgb24);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "format"
    ));

    // Invalid ROI (out of bounds)
    let config = CaptureConfig::new().with_roi(1800, 1000, 200, 200);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "roi"
    ));

    // Invalid ROI (odd dimensions)
    let config = CaptureConfig::new().with_roi(0, 0, 801, 600);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::InvalidParameter { name, .. }) if name == "roi"
    ));

    // Cooler on camera without cooler
    let config = CaptureConfig::new().with_cooler(true);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::ParameterNotSupported(_))
    ));

    // Sensor mode on camera without mode selection
    let config = CaptureConfig::new().with_sensor_mode(DualSamplingMode::LowReadoutNoise);
    assert!(matches!(
        config.validate(&info),
        Err(CameraError::ParameterNotSupported(ref name)) if name == "sensor_mode"
    ));

    // Sensor mode on camera that advertises it
    let mut info_with_modes = info.clone();
    info_with_modes.sensor_modes = vec![SensorMode {
        index: 0,
        name: "Normal".to_string(),
        description: String::new(),
    }];
    let config = CaptureConfig::new().with_sensor_mode(DualSamplingMode::Normal);
    assert!(config.validate(&info_with_modes).is_ok());
}

#[test]
fn test_sensor_type() {
    assert_ne!(SensorType::Mono, SensorType::Color);
}

#[test]
fn test_gain_presets_struct() {
    let presets = GainPresets {
        highest_dr: 0,
        hcg: 120,
        unity: 100,
        lowest_rn: 300,
        offset_highest_dr: 10,
        offset_hcg: 30,
        offset_unity: 20,
        offset_lowest_rn: 50,
    };

    assert_eq!(presets.unity, 100);
    assert_eq!(presets.hcg, 120);
}
