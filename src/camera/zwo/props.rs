use crate::camera::types::{CameraInfo, ImageFormat, SensorType};
use crate::CfaPattern;
use cameraunit_asi::CameraUnitASI;

/// Parsed camera properties from Display output
pub(crate) struct ParsedProps {
    pub is_color: bool,
    pub bayer_pattern: Option<CfaPattern>,
    pub has_cooler: bool,
    pub has_shutter: bool,
    pub is_usb3: bool,
    pub bit_depth: u8,
    pub supported_bins: Vec<u8>,
    pub supported_formats: Vec<ImageFormat>,
}

/// Build CameraInfo from the available camera data
pub(crate) fn build_camera_info(
    cam: &CameraUnitASI,
    info_handle: &cameraunit_asi::CameraInfoASI,
    id: i32,
) -> CameraInfo {
    use cameraunit::{CameraInfo as CUCameraInfo, CameraUnit};

    // Get basic info from the CameraInfo trait
    let name = info_handle.camera_name().to_string();
    let max_width = info_handle.get_ccd_width();
    let max_height = info_handle.get_ccd_height();
    let pixel_size_um = info_handle.get_pixel_size().unwrap_or(0.0) as f64;

    // Parse props display output for additional info
    let props = cam.get_props();
    let props_str = format!("{}", props);
    let parsed = parse_props_display(&props_str);

    // Get exposure and gain limits from CameraUnit trait
    let min_exposure_us = cam
        .get_min_exposure()
        .map(|d| d.as_micros() as u64)
        .unwrap_or(1);
    let max_exposure_us = cam
        .get_max_exposure()
        .map(|d| d.as_micros() as u64)
        .unwrap_or(3600_000_000);
    let min_gain = cam.get_min_gain().unwrap_or(0) as i32;
    let max_gain = cam.get_max_gain().unwrap_or(500) as i32;

    CameraInfo {
        name,
        id,
        max_width,
        max_height,
        pixel_size_um,
        sensor_type: if parsed.is_color {
            SensorType::Color
        } else {
            SensorType::Mono
        },
        bayer_pattern: parsed.bayer_pattern,
        has_cooler: parsed.has_cooler,
        has_shutter: parsed.has_shutter,
        is_usb3: parsed.is_usb3,
        bit_depth: parsed.bit_depth,
        supported_bins: parsed.supported_bins,
        supported_formats: parsed.supported_formats,
        min_exposure_us,
        max_exposure_us,
        min_gain,
        max_gain,
        unity_gain: 120,
        hcg_gain: 100,
    }
}

/// Parse ASICameraProps Display output to extract properties
pub(crate) fn parse_props_display(s: &str) -> ParsedProps {
    let mut parsed = ParsedProps {
        is_color: false,
        bayer_pattern: None,
        has_cooler: false,
        has_shutter: false,
        is_usb3: false,
        bit_depth: 12,
        supported_bins: vec![1],
        supported_formats: vec![ImageFormat::Raw8, ImageFormat::Raw16],
    };

    for line in s.lines() {
        let line = line.trim();

        // Parse "Color: true/false, Shutter: ..., Cooler: ..., USB3: ..."
        if line.starts_with("Color:") {
            for part in line.split(',') {
                let part = part.trim();
                if part.starts_with("Color:") {
                    parsed.is_color = part.contains("true");
                } else if part.starts_with("Shutter:") {
                    parsed.has_shutter = part.contains("true");
                } else if part.starts_with("Cooler:") {
                    parsed.has_cooler = part.contains("true");
                } else if part.starts_with("USB3:") {
                    parsed.is_usb3 = part.contains("true");
                }
            }
        }

        // Parse Bayer Pattern
        if line.starts_with("Bayer Pattern:") {
            if line.contains("BayerRG") {
                parsed.bayer_pattern = Some(CfaPattern::Rggb);
            } else if line.contains("BayerBG") {
                parsed.bayer_pattern = Some(CfaPattern::Bggr);
            } else if line.contains("BayerGR") {
                parsed.bayer_pattern = Some(CfaPattern::Grbg);
            } else if line.contains("BayerGB") {
                parsed.bayer_pattern = Some(CfaPattern::Gbrg);
            }
        }

        // Parse Bins
        if line.starts_with("Bins:") {
            if let Some(start) = line.find('[') {
                if let Some(end) = line.find(']') {
                    let bins_str = &line[start + 1..end];
                    let bins: Vec<u8> = bins_str
                        .split(',')
                        .filter_map(|s| s.trim().parse().ok())
                        .collect();
                    if !bins.is_empty() {
                        parsed.supported_bins = bins;
                    }
                }
            }
        }

        // Parse Bit Depth
        if line.starts_with("Pixel Size:") {
            if let Some(idx) = line.find("Bit Depth:") {
                let rest = &line[idx + 10..];
                if let Ok(depth) = rest.trim().parse::<u8>() {
                    parsed.bit_depth = depth;
                }
            }
        }
    }

    // Add RGB24 format for color cameras
    if parsed.is_color && !parsed.supported_formats.contains(&ImageFormat::Rgb24) {
        parsed.supported_formats.push(ImageFormat::Rgb24);
    }

    parsed
}
