//! INDI Camera Implementation

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

use crate::camera::{Camera, CameraError, CameraInfo, CameraResult, CameraStatus, CaptureConfig};
use crate::frame::Frame;
use crate::indi::client::IndiClient;
use crate::indi::fits_decoder::FitsDecoder;
use crate::indi::xml::{BlobEnable, SwitchState};

pub struct IndiCamera {
    client: IndiClient,
    device_name: String,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
    decode_buffer: Vec<u8>,
}

impl IndiCamera {
    pub async fn connect(host: String, port: u16, index: usize) -> CameraResult<Self> {
        let mut client = IndiClient::new();
        client
            .connect(&host, port, Duration::from_secs(3))
            .await
            .map_err(|e| CameraError::OpenFailed(e.to_string()))?;

        tokio::time::sleep(Duration::from_secs(2)).await;

        let devices = client.list_devices().await;
        let ccd_devices: Vec<_> = devices.into_iter().filter(|d| d.is_ccd()).collect();

        if index >= ccd_devices.len() {
            return Err(CameraError::OpenFailed("Camera index out of bounds".to_string()));
        }

        let device = &ccd_devices[index];
        let device_name = device.name.clone();

        // Ensure BLOBs are enabled for this connection
        client
            .enable_blob(&device_name, Some("CCD1"), BlobEnable::Also)
            .await
            .map_err(|e| CameraError::OpenFailed(e.to_string()))?;

        // Extract some basic CameraInfo from device
        let info = CameraInfo {
            id: index as i32,
            name: device_name.clone(),
            max_width: device.get_number("CCD_INFO", "CCD_MAX_X").map(|n| n.value as u32).unwrap_or(0),
            max_height: device.get_number("CCD_INFO", "CCD_MAX_Y").map(|n| n.value as u32).unwrap_or(0),
            pixel_size_x_um: device.get_number("CCD_INFO", "CCD_PIXEL_SIZE_X").map(|n| n.value).unwrap_or(0.0),
            pixel_size_y_um: device.get_number("CCD_INFO", "CCD_PIXEL_SIZE_Y").map(|n| n.value).unwrap_or(0.0),
            supported_bins: vec![1, 2, 3, 4],
            has_cooler: device.properties.contains_key("CCD_TEMPERATURE"),
            sensor_type: crate::camera::SensorType::Mono, // simplified
            bayer_pattern: None,
            min_gain: device.get_number("CCD_GAIN", "GAIN").map(|n| n.min as i32).unwrap_or(0),
            max_gain: device.get_number("CCD_GAIN", "GAIN").map(|n| n.max as i32).unwrap_or(100),
            min_exposure_us: 1,
            max_exposure_us: 3600_000_000,
            supported_formats: vec![crate::camera::ImageFormat::Raw16, crate::camera::ImageFormat::Raw8],
            bit_depth: 16,
            min_temp_c: device.get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE").map(|n| n.min),
            max_temp_c: device.get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE").map(|n| n.max),
            has_shutter: false,
            is_usb3: false,
            unity_gain: 0,
            hcg_gain: 0,
            sensor_modes: Vec::new(),
            has_dew_heater: false,
        };

        Ok(Self {
            client,
            device_name,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            decode_buffer: Vec::new(),
        })
    }

    async fn check_connection(&self) -> CameraResult<()> {
        if !self.client.is_connected().await {
            Err(CameraError::Disconnected)
        } else {
            Ok(())
        }
    }
}

impl Camera for IndiCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<crate::camera::GainPresets> {
        Ok(crate::camera::GainPresets::default())
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.check_connection().await?;
                
                let mut status = CameraStatus {
                    temperature_c: 0.0,
                    cooler_on: false,
                    cooler_power: None,
                    dew_heater_on: false,
                    is_exposing: false,
                    current_gain: 0,
                    current_offset: 0,
                    current_exposure_us: 0,
                };

                if let Some(dev) = self.client.get_device(&self.device_name).await {
                    if let Some(num) = dev.get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE") {
                        status.temperature_c = num.value;
                    }
                    if let Some(sw) = dev.get_switch("CCD_COOLER", "COOLER_ON") {
                        status.cooler_on = sw.value == SwitchState::On;
                    }
                    if let Some(num) = dev.get_number("CCD_COOLER_POWER", "CCD_COOLER_VALUE") {
                        status.cooler_power = Some(num.value);
                    }
                }

                Ok(status)
            })
        })
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        self.cancel_flag.store(false, Ordering::SeqCst);

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.check_connection().await?;

                // Set Binning
                let bin = config.bin as f64;
                let _ = self.client.set_number(&self.device_name, "CCD_BINNING", vec![("HOR_BIN", bin), ("VER_BIN", bin)]).await;

                // Set Gain
                let _ = self.client.set_number(&self.device_name, "CCD_GAIN", vec![("GAIN", config.gain as f64)]).await;

                // Set Offset
                let _ = self.client.set_number(&self.device_name, "CCD_OFFSET", vec![("OFFSET", config.offset as f64)]).await;

                // Trigger exposure
                let exp_s = config.exposure_us as f64 / 1_000_000.0;
                self.client.set_number(&self.device_name, "CCD_EXPOSURE", vec![("CCD_EXPOSURE_VALUE", exp_s)])
                    .await
                    .map_err(|e| CameraError::ExposureFailed(e.to_string()))?;

                let timeout = Duration::from_micros(config.exposure_us as u64) + Duration::from_secs(5);
                
                // Wait for blob or cancellation
                loop {
                    if self.cancel_flag.load(Ordering::SeqCst) {
                        // Abort exposure
                        let _ = self.client.set_switch(&self.device_name, "CCD_ABORT_EXPOSURE", vec![("ABORT", SwitchState::On)]).await;
                        return Err(CameraError::Cancelled);
                    }

                    match tokio::time::timeout(Duration::from_millis(100), self.client.wait_for_blob(&self.device_name, "CCD1", timeout)).await {
                        Ok(Ok(blob)) => {
                            // Decode blob using pre-allocated buffer
                            FitsDecoder::decode_base64_blob(&blob.value, &mut self.decode_buffer)
                                .map_err(|e| CameraError::ExposureFailed(e.to_string()))?;

                            return FitsDecoder::parse_fits_buffer(&self.decode_buffer)
                                .map_err(|e| CameraError::ExposureFailed(e.to_string()));
                        }
                        Ok(Err(e)) => {
                            if matches!(e, crate::indi::error::IndiError::Disconnected) {
                                return Err(CameraError::Disconnected);
                            }
                            return Err(CameraError::ExposureFailed(e.to_string()));
                        }
                        Err(_) => {
                            // Timeout in this iteration, check cancellation and loop again
                            self.check_connection().await?;
                            continue;
                        }
                    }
                }
            })
        })
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        self.cancel_flag.clone()
    }

    fn close(&mut self) -> CameraResult<()> {
        let mut client = self.client.clone();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async move {
                client.disconnect().await;
            });
        });
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "indi"
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                self.client.set_number(&self.device_name, "CCD_TEMPERATURE", vec![("CCD_TEMPERATURE_VALUE", temp_c as f64)]).await
                    .map_err(|e| CameraError::CoolingFailed(e.to_string()))
            })
        })
    }

    fn set_cooler(&mut self, on: bool) -> CameraResult<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let state = if on { SwitchState::On } else { SwitchState::Off };
                let other = if on { SwitchState::Off } else { SwitchState::On };
                self.client.set_switch(&self.device_name, "CCD_COOLER", vec![("COOLER_ON", state), ("COOLER_OFF", other)]).await
                    .map_err(|e| CameraError::CoolingFailed(e.to_string()))
            })
        })
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        // INDI doesn't have a standard dew heater property, mostly vendor specific.
        Ok(())
    }
}

impl Drop for IndiCamera {
    fn drop(&mut self) {
        let mut client = self.client.clone();
        tokio::spawn(async move {
            client.disconnect().await;
        });
    }
}
