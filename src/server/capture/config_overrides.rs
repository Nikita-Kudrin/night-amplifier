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
///
/// Silent: `to_capture_config` fills a mode on every call, so a log line here fired before
/// every guide exposure (5,363 lines, 17% of the 2026-09-07 field log). The capability is
/// already in the connect-time `Camera specifications` line.
pub(crate) fn apply_sensor_mode_support_override(
    config: &mut crate::camera::CaptureConfig,
    info: &crate::camera::CameraInfo,
) {
    if info.sensor_modes.is_empty() {
        config.sensor_mode = None;
    }
}

/// Environment switch forcing the guide camera's acquisition mode: `snap`, `video` or `auto`.
pub(crate) const GUIDE_ACQUISITION_ENV: &str = "NIGHT_AMPLIFIER_GUIDE_ACQUISITION";

/// Apply [`GUIDE_ACQUISITION_ENV`] to a guide camera's config.
///
/// A field experiment, not a setting: 2026-09-14 the USB 2 guide camera's video stream
/// latched until a reopen, and a snap exposure transfers only the frame the loop asks for.
/// A night with this set says whether that avoids the latch. Read once; the loop builds a
/// config per frame.
pub(crate) fn apply_guide_acquisition_override(config: &mut crate::camera::CaptureConfig) {
    static MODE: std::sync::OnceLock<crate::camera::AcquisitionMode> = std::sync::OnceLock::new();
    config.acquisition = *MODE
        .get_or_init(|| guide_acquisition(std::env::var(GUIDE_ACQUISITION_ENV).ok().as_deref()));
}

fn guide_acquisition(raw: Option<&str>) -> crate::camera::AcquisitionMode {
    let Some(raw) = raw else {
        return crate::camera::AcquisitionMode::Auto;
    };
    crate::camera::AcquisitionMode::parse(raw).unwrap_or_else(|| {
        warn!(value = %raw, "Ignoring {GUIDE_ACQUISITION_ENV}: expected snap, video or auto");
        crate::camera::AcquisitionMode::Auto
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::AcquisitionMode;

    #[test]
    fn the_guide_acquisition_switch_keeps_the_ordinary_rule_unless_set() {
        assert_eq!(guide_acquisition(None), AcquisitionMode::Auto);
        assert_eq!(guide_acquisition(Some("snap")), AcquisitionMode::Snap);
        assert_eq!(guide_acquisition(Some("Video")), AcquisitionMode::Video);
        assert_eq!(
            guide_acquisition(Some("sometimes")),
            AcquisitionMode::Auto,
            "a typo must not change how the camera runs"
        );
    }
}
