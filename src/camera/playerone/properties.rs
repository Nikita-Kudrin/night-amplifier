use crate::{CameraInfo, CfaPattern, ImageFormat, SensorType};
use playerone_sdk::{
    BayerPattern, CameraDescription, CameraProperties, ImageFormat as POAImageFormat,
};

pub fn camera_info_from_description(desc: &CameraDescription) -> CameraInfo {
    camera_info_from_properties(desc.properties())
}

pub fn camera_info_from_properties(props: &CameraProperties) -> CameraInfo {
    let sensor_type = if props.is_color_camera {
        SensorType::Color
    } else {
        SensorType::Mono
    };

    let bayer_pattern = match props.bayer_pattern {
        BayerPattern::RG => Some(CfaPattern::Rggb),
        BayerPattern::BG => Some(CfaPattern::Bggr),
        BayerPattern::GR => Some(CfaPattern::Grbg),
        BayerPattern::GB => Some(CfaPattern::Gbrg),
        BayerPattern::MONO => None,
    };

    let supported_formats: Vec<ImageFormat> = props
        .img_formats
        .iter()
        .filter_map(|f| {
            let format: POAImageFormat = (*f).into();
            match format {
                POAImageFormat::RAW8 | POAImageFormat::MONO8 => Some(ImageFormat::Raw8),
                POAImageFormat::RAW16 => Some(ImageFormat::Raw16),
                POAImageFormat::RGB24 => Some(ImageFormat::Rgb24),
            }
        })
        .collect();

    let (min_temp_c, max_temp_c) = if props.is_has_cooler {
        (Some(-40.0), Some(20.0))
    } else {
        (None, None)
    };

    CameraInfo {
        name: props.camera_model_name.clone(),
        id: props.camera_id as i32,
        max_width: props.max_width,
        max_height: props.max_height,
        pixel_size_x_um: props.pixel_size,
        pixel_size_y_um: props.pixel_size,
        sensor_type,
        bayer_pattern,
        has_cooler: props.is_has_cooler,
        min_temp_c,
        max_temp_c,
        has_shutter: false,
        is_usb3: props.is_usb_3_speed,
        bit_depth: props.bit_depth as u8,
        supported_bins: props.bins.iter().map(|&b| b as u8).collect(),
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
