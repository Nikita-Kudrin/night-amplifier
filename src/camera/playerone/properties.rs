use super::ffi_types::{POABayerPattern, POABool, POACameraProperties, POAImgFormat};
use super::shim::CameraDescription;
use crate::{CameraInfo, CfaPattern, ImageFormat, SensorType};

pub fn camera_info_from_description(desc: &CameraDescription) -> CameraInfo {
    camera_info_from_properties(desc.properties())
}

pub fn camera_info_from_properties(props: &POACameraProperties) -> CameraInfo {
    let sensor_type = if props.isColorCamera == POABool::POA_TRUE {
        SensorType::Color
    } else {
        SensorType::Mono
    };

    let bayer_pattern = match props.bayerPattern {
        POABayerPattern::POA_BAYER_RG => Some(CfaPattern::Rggb),
        POABayerPattern::POA_BAYER_BG => Some(CfaPattern::Bggr),
        POABayerPattern::POA_BAYER_GR => Some(CfaPattern::Grbg),
        POABayerPattern::POA_BAYER_GB => Some(CfaPattern::Gbrg),
        _ => None,
    };

    let supported_formats: Vec<ImageFormat> = props
        .imgFormats
        .iter()
        .take_while(|&&f| f != POAImgFormat::POA_END)
        .filter_map(|&f| match f {
            POAImgFormat::POA_RAW8 | POAImgFormat::POA_MONO8 => Some(ImageFormat::Raw8),
            POAImgFormat::POA_RAW16 => Some(ImageFormat::Raw16),
            POAImgFormat::POA_RGB24 => Some(ImageFormat::Rgb24),
            _ => None,
        })
        .collect();

    let (min_temp_c, max_temp_c) = if props.isHasCooler == POABool::POA_TRUE {
        (Some(-40.0), Some(20.0))
    } else {
        (None, None)
    };

    let name = unsafe { std::ffi::CStr::from_ptr(props.cameraModelName.as_ptr()) }
        .to_string_lossy()
        .into_owned();

    CameraInfo {
        name,
        id: props.cameraID as i32,
        max_width: props.maxWidth as u32,
        max_height: props.maxHeight as u32,
        pixel_size_x_um: props.pixelSize as f64,
        pixel_size_y_um: props.pixelSize as f64,
        sensor_type,
        bayer_pattern,
        has_cooler: props.isHasCooler == POABool::POA_TRUE,
        has_dew_heater: false,
        min_temp_c,
        max_temp_c,
        has_shutter: false,
        is_usb3: props.isUSB3Speed == POABool::POA_TRUE,
        bit_depth: props.bitDepth as u8,
        supported_bins: props
            .bins
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect(),
        supported_formats,
        min_exposure_us: 1,
        max_exposure_us: 3600_000_000,
        min_gain: 0,
        max_gain: 500,
        unity_gain: 100,
        hcg_gain: 120,
        sensor_modes: Vec::new(),
    }
}
