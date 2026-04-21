use crate::camera::types::{CameraInfo, ImageFormat, SensorType};
use crate::CfaPattern;
use super::shim::{Camera as ZwoShimCamera, CameraInfoASI};
use super::ffi_types::*;

/// Build CameraInfo from the available camera data
pub(crate) fn build_camera_info(
    cam: &ZwoShimCamera,
    info: &CameraInfoASI,
    id: i32,
) -> CameraInfo {
    let bayer_pattern = match info.bayer_pattern {
        ASI_BAYER_PATTERN_ASI_BAYER_RG => Some(CfaPattern::Rggb),
        ASI_BAYER_PATTERN_ASI_BAYER_BG => Some(CfaPattern::Bggr),
        ASI_BAYER_PATTERN_ASI_BAYER_GR => Some(CfaPattern::Grbg),
        ASI_BAYER_PATTERN_ASI_BAYER_GB => Some(CfaPattern::Gbrg),
        _ => None,
    };

    let mut supported_formats = Vec::new();
    for &fmt in &info.supported_video_format {
        match fmt {
            ASI_IMG_TYPE_ASI_IMG_RAW8 | ASI_IMG_TYPE_ASI_IMG_Y8 => supported_formats.push(ImageFormat::Raw8),
            ASI_IMG_TYPE_ASI_IMG_RAW16 => supported_formats.push(ImageFormat::Raw16),
            ASI_IMG_TYPE_ASI_IMG_RGB24 => supported_formats.push(ImageFormat::Rgb24),
            _ => {}
        }
    }

    // Get exposure and gain limits
    let min_exposure_us = 32;
    let max_exposure_us = 2000_000_000;
    let min_gain = 0;
    let max_gain = 600;

    CameraInfo {
        name: info.name.clone(),
        id,
        max_width: info.max_width as u32,
        max_height: info.max_height as u32,
        pixel_size_x_um: info.pixel_size as f64,
        pixel_size_y_um: info.pixel_size as f64,
        sensor_type: if info.is_color_cam {
            SensorType::Color
        } else {
            SensorType::Mono
        },
        bayer_pattern,
        has_cooler: info.is_cooler_cam,
        min_temp_c: if info.is_cooler_cam { Some(-40.0) } else { None },
        max_temp_c: if info.is_cooler_cam { Some(20.0) } else { None },
        has_shutter: info.mechanical_shutter,
        is_usb3: info.is_usb3_camera,
        bit_depth: info.bit_depth as u8,
        supported_bins: info.supported_bins.iter().map(|&b| b as u8).collect(),
        supported_formats,
        min_exposure_us,
        max_exposure_us,
        min_gain,
        max_gain,
        unity_gain: 120,
        hcg_gain: 100,
        sensor_modes: Vec::new(),
    }
}
