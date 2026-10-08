//! INDI Camera Implementation

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;

use crate::camera::exposure::{Acquisition, ExposureLoop, Poll, Progress, SdkExposure};
use crate::camera::{
    BufferPool, Camera, CameraError, CameraInfo, CameraResult, CameraStatus, CaptureConfig,
    RawFrame,
};
use crate::indi::client::{BlobWatch, IndiClient};
use crate::indi::error::IndiError;
use crate::indi::fits_decoder::FitsDecoder;
use crate::indi::xml::{BlobEnable, SwitchState};

/// The BLOB property an INDI CCD sends its frames on.
const FRAME_BLOB: &str = "CCD1";

/// How long one poll waits for a BLOB before checking for a cancel.
const POLL_WAIT: Duration = Duration::from_millis(5);

pub struct IndiCamera {
    client: IndiClient,
    device_name: String,
    info: CameraInfo,
    exposure: ExposureLoop,
    decode_buffer: Vec<u8>,
    pool: BufferPool,
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
            return Err(CameraError::OpenFailed(
                "Camera index out of bounds".to_string(),
            ));
        }

        let device = &ccd_devices[index];
        let device_name = device.name.clone();

        // Ensure BLOBs are enabled for this connection
        client
            .enable_blob(&device_name, Some(FRAME_BLOB), BlobEnable::Also)
            .await
            .map_err(|e| CameraError::OpenFailed(e.to_string()))?;

        // Extract some basic CameraInfo from device
        let info = CameraInfo {
            id: index as i32,
            name: device_name.clone(),
            max_width: device
                .get_number("CCD_INFO", "CCD_MAX_X")
                .map(|n| n.value as u32)
                .unwrap_or(0),
            max_height: device
                .get_number("CCD_INFO", "CCD_MAX_Y")
                .map(|n| n.value as u32)
                .unwrap_or(0),
            pixel_size_x_um: device
                .get_number("CCD_INFO", "CCD_PIXEL_SIZE_X")
                .map(|n| n.value)
                .unwrap_or(0.0),
            pixel_size_y_um: device
                .get_number("CCD_INFO", "CCD_PIXEL_SIZE_Y")
                .map(|n| n.value)
                .unwrap_or(0.0),
            supported_bins: vec![1, 2, 3, 4],
            has_cooler: device.properties.contains_key("CCD_TEMPERATURE"),
            sensor_type: crate::camera::SensorType::Mono, // simplified
            bayer_pattern: None,
            min_gain: device
                .get_number("CCD_GAIN", "GAIN")
                .map(|n| n.min as i32)
                .unwrap_or(0),
            max_gain: device
                .get_number("CCD_GAIN", "GAIN")
                .map(|n| n.max as i32)
                .unwrap_or(100),
            min_exposure_us: 1,
            max_exposure_us: 3600_000_000,
            supported_formats: vec![
                crate::camera::ImageFormat::Raw16,
                crate::camera::ImageFormat::Raw8,
            ],
            bit_depth: 16,
            min_temp_c: device
                .get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE")
                .map(|n| n.min),
            max_temp_c: device
                .get_number("CCD_TEMPERATURE", "CCD_TEMPERATURE_VALUE")
                .map(|n| n.max),
            has_shutter: false,
            is_usb3: false,
            unity_gain: 0,
            hcg_gain: 0,
            sensor_modes: Vec::new(),
            has_dew_heater: false,
            serial: None,
        };

        Ok(Self {
            client,
            device_name,
            info,
            exposure: ExposureLoop::new(),
            decode_buffer: Vec::new(),
            pool: BufferPool::new(),
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

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
        tokio::task::block_in_place(|| {
            let runtime = Handle::current();
            runtime.block_on(self.check_connection())?;
            let supports_video = runtime
                .block_on(self.client.get_device(&self.device_name))
                .is_some_and(|device| device.properties.contains_key("CCD_VIDEO_STREAM"));
            let Self {
                client,
                device_name,
                info,
                exposure,
                decode_buffer,
                pool,
            } = self;
            let mut sdk = IndiExposure {
                client,
                device: device_name,
                info,
                runtime,
                supports_video,
                exposure_s: config.exposure_us as f64 / 1_000_000.0,
                decode_buffer,
                pool,
                blobs: None,
            };
            exposure.capture(&mut sdk, config)
        })
    }

    fn invalidate_config_cache(&mut self) {
        self.exposure.invalidate();
    }

    fn cancel(&self) {
        self.exposure.cancel();
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        self.exposure.cancel_token()
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
                self.client
                    .set_number(
                        &self.device_name,
                        "CCD_TEMPERATURE",
                        vec![("CCD_TEMPERATURE_VALUE", temp_c)],
                    )
                    .await
                    .map_err(|e| CameraError::CoolingFailed(e.to_string()))
            })
        })
    }

    fn set_cooler(&mut self, on: bool) -> CameraResult<()> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let state = if on {
                    SwitchState::On
                } else {
                    SwitchState::Off
                };
                let other = if on {
                    SwitchState::Off
                } else {
                    SwitchState::On
                };
                self.client
                    .set_switch(
                        &self.device_name,
                        "CCD_COOLER",
                        vec![("COOLER_ON", state), ("COOLER_OFF", other)],
                    )
                    .await
                    .map_err(|e| CameraError::CoolingFailed(e.to_string()))
            })
        })
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        // INDI doesn't have a standard dew heater property, mostly vendor specific.
        Ok(())
    }
}

/// INDI's calls for the shared [`ExposureLoop`]: properties over the client, frames as
/// FITS BLOBs. Property writes that fail are ignored, as they always were — a driver
/// without, say, `CCD_OFFSET` still exposes.
struct IndiExposure<'a> {
    client: &'a IndiClient,
    device: &'a str,
    info: &'a CameraInfo,
    runtime: Handle,
    supports_video: bool,
    /// `CCD_EXPOSURE` both sets the length and triggers, so it is sent at start.
    exposure_s: f64,
    decode_buffer: &'a mut Vec<u8>,
    pool: &'a mut BufferPool,
    /// Taken before an exposure is triggered and dropped with the adapter, so a
    /// stream's frames never queue up between captures — only one in flight is read.
    blobs: Option<BlobWatch>,
}

impl IndiExposure<'_> {
    fn set_switch(&self, property: &str, on: &str, off: &str) {
        let elements = vec![(on, SwitchState::On), (off, SwitchState::Off)];
        let _ = self
            .runtime
            .block_on(self.client.set_switch(self.device, property, elements));
    }

    fn set_numbers(&self, property: &str, elements: Vec<(&str, f64)>) {
        let _ = self
            .runtime
            .block_on(self.client.set_number(self.device, property, elements));
    }

    fn decode(&mut self, blob: &str) -> CameraResult<RawFrame> {
        FitsDecoder::decode_base64_blob(blob, self.decode_buffer)
            .and_then(|()| FitsDecoder::parse_fits_buffer(self.decode_buffer, self.pool))
            .map_err(|e| CameraError::ExposureFailed(e.to_string()))
    }
}

impl SdkExposure for IndiExposure<'_> {
    fn info(&self) -> &CameraInfo {
        self.info
    }

    fn acquisition(&self, config: &CaptureConfig) -> Acquisition {
        if config.is_continuous() && self.supports_video {
            Acquisition::Stream
        } else {
            Acquisition::Single
        }
    }

    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        let frame_type = vec![
            ("FRAME_LIGHT", SwitchState::On),
            ("FRAME_BIAS", SwitchState::Off),
            ("FRAME_DARK", SwitchState::Off),
            ("FRAME_FLAT", SwitchState::Off),
        ];
        let _ = self
            .runtime
            .block_on(self.client.set_switch(self.device, "CCD_FRAME_TYPE", frame_type));

        let bin = f64::from(config.bin);
        self.set_numbers("CCD_BINNING", vec![("HOR_BIN", bin), ("VER_BIN", bin)]);

        let (x, y, w, h) = config
            .roi
            .unwrap_or((0, 0, self.info.max_width, self.info.max_height));
        self.set_numbers(
            "CCD_FRAME",
            vec![
                ("X", f64::from(x)),
                ("Y", f64::from(y)),
                ("WIDTH", f64::from(w)),
                ("HEIGHT", f64::from(h)),
            ],
        );

        self.set_numbers("CCD_GAIN", vec![("GAIN", f64::from(config.gain))]);
        self.set_numbers("CCD_OFFSET", vec![("OFFSET", f64::from(config.offset))]);
        Ok(())
    }

    /// Watches for the BLOB before anything can send one.
    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()> {
        self.blobs = Some(self.client.watch_blobs(self.device, FRAME_BLOB));
        if acquisition == Acquisition::Stream {
            self.set_switch("CCD_VIDEO_STREAM", "STREAM_ON", "STREAM_OFF");
        }
        self.runtime
            .block_on(self.client.set_number(
                self.device,
                "CCD_EXPOSURE",
                vec![("CCD_EXPOSURE_VALUE", self.exposure_s)],
            ))
            .map_err(|e| CameraError::ExposureFailed(e.to_string()))
    }

    /// Stops the stream only: reconfiguring or switching to single exposures never
    /// aborted an exposure.
    fn end_stream(&mut self) {
        self.set_switch("CCD_VIDEO_STREAM", "STREAM_OFF", "STREAM_ON");
    }

    fn abort(&mut self, acquisition: Acquisition) {
        if acquisition == Acquisition::Stream {
            self.end_stream();
        }
        let _ = self.runtime.block_on(self.client.set_switch(
            self.device,
            "CCD_ABORT_EXPOSURE",
            vec![("ABORT", SwitchState::On)],
        ));
    }

    /// INDI frames arrive self-describing; this is only the size the stall budget expects
    /// — the same estimate the capture watchdog derives its own timeout from.
    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize> {
        Ok(config.frame_bytes(self.info))
    }

    fn poll(&mut self, _progress: &Progress, _buffer: &mut [u8]) -> Poll {
        let (client, device) = (self.client, self.device);
        let blobs = self
            .blobs
            .get_or_insert_with(|| client.watch_blobs(device, FRAME_BLOB));
        let failed = |error| Poll::Failed {
            error,
            stream_ended: false,
        };
        match self.runtime.block_on(blobs.next(POLL_WAIT)) {
            Ok(Some(blob)) => {
                match self.decode(&blob.value) {
                    Ok(frame) => Poll::Delivered(frame),
                    Err(error) => failed(error),
                }
            }
            Ok(None) if self.runtime.block_on(client.is_connected()) => Poll::Pending,
            Ok(None) | Err(IndiError::Disconnected) => failed(CameraError::Disconnected),
            Err(e) => failed(CameraError::ExposureFailed(e.to_string())),
        }
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

#[cfg(test)]
#[path = "camera_tests.rs"]
mod tests;
