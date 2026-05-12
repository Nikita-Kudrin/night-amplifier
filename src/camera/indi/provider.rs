//! INDI Camera Provider Implementation

use std::time::Duration;
use tracing::{debug, error, info};

use crate::camera::{Camera, CameraEntry, CameraInfo, CameraProvider, CameraResult, ImageFormat, SensorType};
use crate::indi::client::IndiClient;
use crate::indi::xml::PropertyState;

pub struct IndiProvider {
    pub host: String,
    pub port: u16,
}

impl IndiProvider {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub fn slugify_device_name(name: &str) -> String {
        name.chars()
            .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-")
    }

    pub async fn list_cameras_async(&self) -> CameraResult<Vec<CameraInfo>> {
        let mut client = IndiClient::new();
        if let Err(e) = client.connect(&self.host, self.port, Duration::from_secs(3)).await {
            debug!("INDI provider failed to connect: {}", e);
            return Ok(vec![]);
        }

        // Wait a bit for properties to stream in
        tokio::time::sleep(Duration::from_secs(2)).await;

        let mut entries = Vec::new();
        let devices = client.list_devices().await;

        for (idx, device) in devices.iter().enumerate() {
            if !device.is_ccd() {
                continue;
            }

            let name = device.name.clone();
            let slug = Self::slugify_device_name(&name);
            let id = format!("indi_{}", slug);

            let mut max_width = 0;
            let mut max_height = 0;
            let mut pixel_size_x_um = 0.0;
            let mut pixel_size_y_um = 0.0;
            let mut bit_depth = 16;
            
            if let Some(num) = device.get_number("CCD_INFO", "CCD_MAX_X") {
                max_width = num.value as u32;
            }
            if let Some(num) = device.get_number("CCD_INFO", "CCD_MAX_Y") {
                max_height = num.value as u32;
            }
            if let Some(num) = device.get_number("CCD_INFO", "CCD_PIXEL_SIZE_X") {
                pixel_size_x_um = num.value;
            }
            if let Some(num) = device.get_number("CCD_INFO", "CCD_PIXEL_SIZE_Y") {
                pixel_size_y_um = num.value;
            }
            if let Some(num) = device.get_number("CCD_INFO", "CCD_BITSPERPIXEL") {
                bit_depth = num.value as u8;
            }

            let mut supported_bins = vec![1];
            if let Some(num) = device.get_number("CCD_BINNING", "HOR_BIN") {
                let max_bin = num.max as u8;
                supported_bins = (1..=max_bin).collect();
            }

            let has_cooler = device.properties.contains_key("CCD_TEMPERATURE");
            
            let mut min_gain = 0.0;
            let mut max_gain = 100.0;
            if let Some(num) = device.get_number("CCD_GAIN", "GAIN") {
                min_gain = num.min;
                max_gain = num.max;
            }

            let mut min_exposure_us = 1.0;
            let mut max_exposure_us = 3600_000_000.0;
            if let Some(num) = device.get_number("CCD_EXPOSURE", "CCD_EXPOSURE_VALUE") {
                min_exposure_us = num.min * 1_000_000.0;
                max_exposure_us = num.max * 1_000_000.0;
            }

            // Attempt to determine color or mono
            let bayer_pattern = None;
            let sensor_type = SensorType::Mono;
            // INDI doesn't always expose Bayer pattern clearly until a frame is taken, 
            // but we might check CfaPattern text or format
            // if device.properties.contains_key("CCD_CFA") || device.properties.contains_key("BAYERPAT") {
            //    sensor_type = SensorType::Color;
            // }

            let info = CameraInfo {
                id: idx as i32,
                name: name.clone(),
                max_width,
                max_height,
                pixel_size_x_um,
                pixel_size_y_um,
                supported_bins,
                has_cooler,
                sensor_type,
                bayer_pattern,
                min_gain: min_gain as i32,
                max_gain: max_gain as i32,
                min_exposure_us: min_exposure_us as u64,
                max_exposure_us: max_exposure_us as u64,
                supported_formats: vec![ImageFormat::Raw16, ImageFormat::Raw8],
                bit_depth,
                min_temp_c: device.get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE").map(|n| n.min),
                max_temp_c: device.get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE").map(|n| n.max),
                has_shutter: false,
                is_usb3: false,
                unity_gain: 0,
                hcg_gain: 0,
                sensor_modes: Vec::new(),
                has_dew_heater: false,
            };

            entries.push(info);
        }

        client.disconnect().await;
        Ok(entries)
    }
}

impl CameraProvider for IndiProvider {
    fn name(&self) -> &'static str {
        "indi"
    }

    fn is_available(&self) -> bool {
        // We do async checks elsewhere
        true
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Ok(self.list_cameras()?.len())
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        // Expected to be called asynchronously via tokio::spawn
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(self.list_cameras_async())
        })
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        // This blocks in the caller, but will use tokio internally inside IndiCamera
        let host = self.host.clone();
        let port = self.port;

        let camera = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                crate::camera::indi::camera::IndiCamera::connect(host, port, index).await
            })
        })?;

        Ok(Box::new(camera))
    }
}
