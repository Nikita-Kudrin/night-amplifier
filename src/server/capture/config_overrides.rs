use crate::camera::{Camera, ImageFormat, SensorMode};
use crate::server::state::CaptureSettings;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
/// Override the capture format with the best raw format advertised by the
/// camera (`Raw16` preferred, `Raw8` as fallback). Leaves the config untouched
/// if neither is advertised, letting the provider surface a clear SDK error.
pub(crate) fn apply_best_raw_format(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if let Some(format) = crate::camera::ImageFormat::best_raw_format(&info.supported_formats) {
        if config.format != format {
            debug!(
                camera = %camera_name,
                selected = ?format,
                requested = ?config.format,
                supported = ?info.supported_formats,
                "Adjusted capture format to best available raw format"
            );
            config.format = format;
        }
    } else {
        warn!(
            camera = %camera_name,
            supported = ?info.supported_formats,
            "Camera advertises neither Raw16 nor Raw8 — capture may fail"
        );
    }
}

/// Drop cooler-related fields when the camera has no cooler. Saved settings
/// may carry `cooler_enabled = true` from a previous cooled camera; without
/// this override `CaptureConfig::validate` would reject the config and
/// capture would fail before the first frame.
pub(crate) fn apply_cooler_support_override(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if info.has_cooler {
        return;
    }
    if config.cooler_enabled || config.target_temp_c.is_some() {
        debug!(
            camera = %camera_name,
            "Camera has no cooler; clearing cooler_enabled / target_temp_c from capture config"
        );
        config.cooler_enabled = false;
        config.target_temp_c = None;
    }
}

/// Drop `sensor_mode` when the camera doesn't advertise sensor modes.
/// `CaptureSettings::to_capture_config` fills `sensor_mode` unconditionally
/// from the explicit override or from `stacking_type.desired_sensor_mode()`
/// — neither is aware of the active camera's capabilities. Without this
/// override, `CaptureConfig::validate` rejects the request with
/// `ParameterNotSupported("sensor_mode")` for any camera that reports an
/// empty `sensor_modes` list (e.g. Player One uncooled planetary models).
pub(crate) fn apply_sensor_mode_support_override(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
    camera_name: &str,
) {
    if !info.sensor_modes.is_empty() {
        return;
    }
    if config.sensor_mode.is_some() {
        debug!(
            camera = %camera_name,
            "Camera advertises no sensor modes; clearing sensor_mode from capture config"
        );
        config.sensor_mode = None;
    }
}
