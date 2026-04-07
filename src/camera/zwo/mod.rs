//! ZWO ASI camera implementation
//!
//! Uses the `cameraunit_asi` crate for safe Rust bindings to the ZWO ASI SDK.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cameraunit_asi::{get_camera_ids, num_cameras, open_camera, ASIImageFormat, CameraUnitASI};

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, SensorType};

mod props;
mod utils;

use props::{build_camera_info, parse_props_display};
use utils::image_to_bytes;

/// ZWO camera provider
pub struct ZwoProvider;

impl ZwoProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZwoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for ZwoProvider {
    fn name(&self) -> &'static str {
        "ZWO"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn camera_count(&self) -> CameraResult<usize> {
        let count = catch_ffi_panic("ZWO::num_cameras", num_cameras).map_err(CameraError::from)?;
        Ok(count.max(0) as usize)
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let ids =
            catch_ffi_panic("ZWO::get_camera_ids", get_camera_ids).map_err(CameraError::from)?;
        match ids {
            Some(map) => {
                let mut cameras = Vec::new();
                for (id, _name) in map {
                    if let Ok(Ok((cam, info))) =
                        catch_ffi_panic("ZWO::open_camera", || open_camera(id))
                    {
                        cameras.push(build_camera_info(&cam, &info, id));
                    }
                }
                Ok(cameras)
            }
            None => Ok(Vec::new()),
        }
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = ZwoCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

/// ZWO camera handle
pub struct ZwoCamera {
    camera: CameraUnitASI,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
}

impl ZwoCamera {
    /// Get the number of connected ZWO cameras
    pub fn camera_count() -> CameraResult<usize> {
        let count = catch_ffi_panic("ZWO::num_cameras", num_cameras).map_err(CameraError::from)?;
        Ok(count.max(0) as usize)
    }

    /// List all connected cameras
    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        let ids =
            catch_ffi_panic("ZWO::get_camera_ids", get_camera_ids).map_err(CameraError::from)?;
        match ids {
            Some(map) => {
                let mut cameras = Vec::new();
                for (id, _name) in map {
                    if let Ok(Ok((cam, info))) =
                        catch_ffi_panic("ZWO::open_camera", || open_camera(id))
                    {
                        cameras.push(build_camera_info(&cam, &info, id));
                    }
                }
                Ok(cameras)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Open a camera by index
    pub fn open(index: usize) -> CameraResult<Self> {
        let ids = catch_ffi_panic("ZWO::get_camera_ids", get_camera_ids)
            .map_err(CameraError::from)?
            .ok_or(CameraError::NoCamerasFound)?;

        if ids.is_empty() {
            return Err(CameraError::NoCamerasFound);
        }

        let mut sorted_ids: Vec<i32> = ids.keys().cloned().collect();
        sorted_ids.sort();

        if index >= sorted_ids.len() {
            return Err(CameraError::InvalidCameraIndex {
                index,
                count: sorted_ids.len(),
            });
        }

        let camera_id = sorted_ids[index];
        let (camera, camera_info_handle) =
            catch_ffi_panic("ZWO::open_camera", || open_camera(camera_id))
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::OpenFailed(format!("{:?}", e)))?;

        let info = build_camera_info(&camera, &camera_info_handle, camera_id);

        Ok(Self {
            camera,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Open a camera by name
    pub fn open_by_name(name: &str) -> CameraResult<Self> {
        let ids = catch_ffi_panic("ZWO::get_camera_ids", get_camera_ids)
            .map_err(CameraError::from)?
            .ok_or(CameraError::NoCamerasFound)?;

        for (id, cam_name) in &ids {
            if cam_name.contains(name) {
                let (camera, camera_info_handle) =
                    catch_ffi_panic("ZWO::open_camera", || open_camera(*id))
                        .map_err(CameraError::from)?
                        .map_err(|e| CameraError::OpenFailed(format!("{:?}", e)))?;

                let info = build_camera_info(&camera, &camera_info_handle, *id);

                return Ok(Self {
                    camera,
                    info,
                    cancel_flag: Arc::new(AtomicBool::new(false)),
                });
            }
        }

        Err(CameraError::OpenFailed(format!(
            "Camera '{}' not found",
            name
        )))
    }

    fn apply_config(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        use cameraunit::CameraUnit;

        let exposure = Duration::from_micros(config.exposure_us);
        catch_ffi_panic("ZWO::set_exposure", || self.camera.set_exposure(exposure))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set exposure: {:?}", e),
            })?;

        let gain = config.gain as i64;
        catch_ffi_panic("ZWO::set_gain_raw", || self.camera.set_gain_raw(gain))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set gain: {:?}", e),
            })?;

        let format = match config.format {
            ImageFormat::Raw8 => ASIImageFormat::ImageRAW8,
            ImageFormat::Raw16 => ASIImageFormat::ImageRAW16,
            ImageFormat::Rgb24 => ASIImageFormat::ImageRGB24,
        };
        catch_ffi_panic("ZWO::set_image_fmt", || self.camera.set_image_fmt(format))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set image format: {:?}", e),
            })?;

        let roi = if let Some((x, y, w, h)) = config.roi {
            cameraunit::ROI {
                x_min: x,
                y_min: y,
                width: w,
                height: h,
                bin_x: config.bin as u32,
                bin_y: config.bin as u32,
            }
        } else {
            let width = self.info.max_width / config.bin as u32;
            let height = self.info.max_height / config.bin as u32;
            cameraunit::ROI {
                x_min: 0,
                y_min: 0,
                width,
                height,
                bin_x: config.bin as u32,
                bin_y: config.bin as u32,
            }
        };

        catch_ffi_panic("ZWO::set_roi", || self.camera.set_roi(&roi))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set ROI: {:?}", e),
            })?;

        if self.info.has_cooler && config.cooler_enabled {
            use cameraunit::CameraInfo;
            if let Some(temp) = config.target_temp_c {
                catch_ffi_panic("ZWO::set_temperature", || {
                    self.camera.set_temperature(temp as f32)
                })
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))?;
            }
            catch_ffi_panic("ZWO::set_cooler", || self.camera.set_cooler(true))
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))?;
        }

        Ok(())
    }

    fn get_capture_dimensions(&self, config: &CaptureConfig) -> (u32, u32) {
        if let Some((_, _, w, h)) = config.roi {
            (w, h)
        } else {
            (
                self.info.max_width / config.bin as u32,
                self.info.max_height / config.bin as u32,
            )
        }
    }

    fn buffer_to_frame(
        &self,
        image: cameraunit::DynamicSerialImage,
        width: u32,
        height: u32,
        config: &CaptureConfig,
    ) -> CameraResult<Frame> {
        let (pixel_format, channels) = match config.format {
            ImageFormat::Raw8 => {
                if self.info.sensor_type == SensorType::Color {
                    (PixelFormat::Bayer8, 1)
                } else {
                    (PixelFormat::Rgb8, 1)
                }
            }
            ImageFormat::Raw16 => {
                if self.info.sensor_type == SensorType::Color {
                    (PixelFormat::Bayer16, 1)
                } else {
                    (PixelFormat::Rgb16, 1)
                }
            }
            ImageFormat::Rgb24 => (PixelFormat::Rgb8, 3),
        };

        let buffer = image_to_bytes(image);

        if self.info.sensor_type == SensorType::Color && channels == 1 {
            let pattern = self.info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
            Frame::from_bayer(
                &buffer,
                width as usize,
                height as usize,
                pixel_format,
                pattern,
            )
            .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        } else {
            Frame::from_raw(
                &buffer,
                width as usize,
                height as usize,
                channels,
                pixel_format,
            )
            .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        }
    }
}

impl Camera for ZwoCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets {
            highest_dr: 0,
            hcg: 100,
            unity: 120,
            lowest_rn: self.info.max_gain,
            offset_highest_dr: 10,
            offset_hcg: 30,
            offset_unity: 20,
            offset_lowest_rn: 50,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        use cameraunit::{CameraInfo, CameraUnit};

        let temperature = catch_ffi_panic("ZWO::get_temperature", || self.camera.get_temperature())
            .map_err(CameraError::from)?
            .unwrap_or(0.0) as f64;

        let current_gain = catch_ffi_panic("ZWO::get_gain_raw", || self.camera.get_gain_raw())
            .map_err(CameraError::from)? as i32;

        let current_offset = catch_ffi_panic("ZWO::get_offset", || self.camera.get_offset())
            .map_err(CameraError::from)?;

        let current_exposure_us =
            catch_ffi_panic("ZWO::get_exposure", || self.camera.get_exposure())
                .map_err(CameraError::from)?
                .as_micros() as u64;

        let cooler_power = if self.info.has_cooler {
            catch_ffi_panic("ZWO::get_cooler_power", || self.camera.get_cooler_power())
                .map_err(CameraError::from)?
                .map(|p| p as f64)
        } else {
            None
        };

        let cooler_on = if self.info.has_cooler {
            catch_ffi_panic("ZWO::get_cooler", || self.camera.get_cooler())
                .map_err(CameraError::from)?
                .unwrap_or(false)
        } else {
            false
        };

        let is_exposing = catch_ffi_panic("ZWO::is_capturing", || self.camera.is_capturing())
            .map_err(CameraError::from)?;

        Ok(CameraStatus {
            temperature_c: temperature,
            cooler_power,
            cooler_on,
            is_exposing,
            current_gain,
            current_offset,
            current_exposure_us,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        use cameraunit::CameraInfo;
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("ZWO::set_temperature", || {
            self.camera.set_temperature(temp_c as f32)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))?;
        Ok(())
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        use cameraunit::CameraInfo;
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("ZWO::set_cooler", || self.camera.set_cooler(enabled))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        use cameraunit::CameraUnit;

        config.validate(&self.info)?;
        self.cancel_flag.store(false, Ordering::SeqCst);
        self.apply_config(config)?;

        let exposure_duration = Duration::from_micros(config.exposure_us);
        let total_timeout = config.timeout + exposure_duration;
        let start = Instant::now();

        catch_ffi_panic("ZWO::start_exposure", || self.camera.start_exposure())
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::ExposureFailed(format!("{:?}", e)))?;

        loop {
            if self.cancel_flag.load(Ordering::SeqCst) {
                use cameraunit::CameraInfo;
                let _ = catch_ffi_panic("ZWO::cancel_capture", || self.camera.cancel_capture());
                return Err(CameraError::Cancelled);
            }

            if start.elapsed() > total_timeout {
                use cameraunit::CameraInfo;
                let _ = catch_ffi_panic("ZWO::cancel_capture", || self.camera.cancel_capture());
                return Err(CameraError::ExposureTimeout(total_timeout));
            }

            let ready_result = catch_ffi_panic("ZWO::image_ready", || self.camera.image_ready())
                .map_err(CameraError::from)?;

            match ready_result {
                Ok(true) => break,
                Ok(false) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    use cameraunit::CameraInfo;
                    let _ = catch_ffi_panic("ZWO::cancel_capture", || self.camera.cancel_capture());
                    return Err(CameraError::ExposureFailed(format!("{:?}", e)));
                }
            }
        }

        let image = catch_ffi_panic("ZWO::download_image", || self.camera.download_image())
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::ImageReadFailed(format!("{:?}", e)))?;

        let (width, height) = self.get_capture_dimensions(config);
        self.buffer_to_frame(image, width, height, config)
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "ZWO"
    }
}

#[cfg(test)]
mod tests;
