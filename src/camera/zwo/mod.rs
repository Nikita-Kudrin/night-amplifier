//! ZWO ASI camera implementation
//!
//! Uses the `cameraunit_asi` crate for safe Rust bindings to the ZWO ASI SDK.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};
use shim::{get_camera_ids, num_cameras, Camera as ZwoShimCamera, CameraInfoASI};

use super::device_lost::tolerate_unsupported;
use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{
    BufferPool, CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, RawFrame,
    SensorType,
};

mod props;

use props::build_camera_info;

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
                        catch_ffi_panic("ZWO::open_camera", || ZwoShimCamera::open(id))
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
    camera: ZwoShimCamera,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
    last_applied_config: Option<CaptureConfig>,
    buffer_pool: BufferPool,
    stream_running: bool,
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
                        catch_ffi_panic("ZWO::open_camera", || ZwoShimCamera::open(id))
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
            catch_ffi_panic("ZWO::open_camera", || ZwoShimCamera::open(camera_id))
                .map_err(CameraError::from)?
                .map_err(CameraError::OpenFailed)?;

        let mut info = build_camera_info(&camera, &camera_info_handle, camera_id);
        info.has_dew_heater =
            camera.is_control_supported(ffi_types::ASI_CONTROL_TYPE_ASI_ANTI_DEW_HEATER);

        Ok(Self {
            camera,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            last_applied_config: None,
            buffer_pool: BufferPool::new(),
            stream_running: false,
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
                    catch_ffi_panic("ZWO::open_camera", || ZwoShimCamera::open(*id))
                        .map_err(CameraError::from)?
                        .map_err(CameraError::OpenFailed)?;

                let info = build_camera_info(&camera, &camera_info_handle, *id);

                return Ok(Self {
                    camera,
                    info,
                    cancel_flag: Arc::new(AtomicBool::new(false)),
                    last_applied_config: None,
                    buffer_pool: BufferPool::new(),
                    stream_running: false,
                });
            }
        }

        Err(CameraError::OpenFailed(format!(
            "Camera '{}' not found",
            name
        )))
    }

    fn apply_config(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        let exposure = config.exposure_us as i64;
        catch_ffi_panic("ZWO::set_exposure", || self.camera.set_exposure(exposure))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set exposure: {}", e),
            })?;

        let gain = config.gain as i64;
        catch_ffi_panic("ZWO::set_gain_raw", || self.camera.set_gain_raw(gain))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set gain: {}", e),
            })?;

        let format = match config.format {
            ImageFormat::Raw8 => ffi_types::ASI_IMG_TYPE_ASI_IMG_RAW8,
            ImageFormat::Raw16 => ffi_types::ASI_IMG_TYPE_ASI_IMG_RAW16,
            ImageFormat::Rgb24 => ffi_types::ASI_IMG_TYPE_ASI_IMG_RGB24,
        };
        catch_ffi_panic("ZWO::set_image_fmt", || self.camera.set_image_fmt(format))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set image format: {}", e),
            })?;

        let (x, y, w, h) = if let Some((x, y, w, h)) = config.roi {
            (x as i32, y as i32, w as i32, h as i32)
        } else {
            let width = (self.info.max_width / config.bin as u32) as i32;
            let height = (self.info.max_height / config.bin as u32) as i32;
            (0, 0, width, height)
        };

        catch_ffi_panic("ZWO::set_roi", || {
            self.camera.set_roi(x, y, w, h, config.bin as i32)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set ROI: {}", e),
        })?;

        if self.info.has_cooler {
            if config.cooler_enabled {
                if let Some(temp) = config.target_temp_c {
                    let result = catch_ffi_panic("ZWO::set_temperature", || {
                        self.camera.set_temperature(temp as f32)
                    });
                    match result {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            tracing::warn!(error = ?e, target_temp_c = temp, "Failed to set target temperature")
                        }
                        Err(e) => tracing::warn!(error = %e, "Panic setting target temperature"),
                    }
                }
            }
            let result = catch_ffi_panic("ZWO::set_cooler", || {
                self.camera.set_cooler(config.cooler_enabled)
            });
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    tracing::warn!(error = ?e, enabled = config.cooler_enabled, "Failed to set cooler state")
                }
                Err(e) => tracing::warn!(error = %e, "Panic setting cooler state"),
            }
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
        buffer: &[u8],
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

        if self.info.sensor_type == SensorType::Color && channels == 1 {
            let pattern = self.info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
            Frame::from_bayer(
                buffer,
                width as usize,
                height as usize,
                pixel_format,
                pattern,
            )
            .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        } else {
            Frame::from_raw(
                buffer,
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
        // Every read goes through `tolerate_unsupported`: a parameter this
        // model does not expose falls back, but a lost device propagates so the
        // fault detector can see it. See `camera::device_lost`.
        let temperature = tolerate_unsupported(
            catch_ffi_panic("ZWO::get_temperature", || self.camera.get_temperature())
                .map_err(CameraError::from)?,
            0.0,
        )? as f64;

        let current_gain = tolerate_unsupported(
            catch_ffi_panic("ZWO::get_gain_raw", || self.camera.get_gain_raw())
                .map_err(CameraError::from)?,
            0,
        )? as i32;

        let current_offset = tolerate_unsupported(
            catch_ffi_panic("ZWO::get_offset_raw", || self.camera.get_offset_raw())
                .map_err(CameraError::from)?,
            0,
        )? as i32;

        let current_exposure_us = tolerate_unsupported(
            catch_ffi_panic("ZWO::get_exposure", || self.camera.get_exposure())
                .map_err(CameraError::from)?,
            0,
        )? as u64;

        let cooler_on = if self.info.has_cooler {
            tolerate_unsupported(
                catch_ffi_panic("ZWO::get_cooler", || self.camera.get_cooler())
                    .map_err(CameraError::from)?,
                false,
            )?
        } else {
            false
        };

        let dew_heater_on = if self.info.has_dew_heater {
            tolerate_unsupported(
                catch_ffi_panic("ZWO::get_anti_dew_heater", || {
                    self.camera.get_anti_dew_heater()
                })
                .map_err(CameraError::from)?,
                false,
            )?
        } else {
            false
        };

        Ok(CameraStatus {
            temperature_c: temperature,
            cooler_power: None,
            cooler_on,
            is_exposing: false,
            current_gain,
            current_offset,
            current_exposure_us,
            dew_heater_on,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("ZWO::set_temperature", || {
            self.camera.set_temperature(temp_c as f32)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::CoolingFailed)?;
        Ok(())
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("ZWO::set_cooler", || self.camera.set_cooler(enabled))
            .map_err(CameraError::from)?
            .map_err(CameraError::CoolingFailed)
    }

    fn set_dew_heater(&mut self, enabled: bool, _power: i32) -> CameraResult<()> {
        if !self.info.has_dew_heater {
            return Err(CameraError::ParameterNotSupported("dew_heater".to_string()));
        }
        catch_ffi_panic("ZWO::set_anti_dew_heater", || {
            self.camera.set_anti_dew_heater(enabled)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::ParameterNotSupported(format!("{:?}", e)))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
        config.validate(&self.info)?;
        self.cancel_flag.store(false, Ordering::SeqCst);
        let exposure_duration = Duration::from_micros(config.exposure_us);
        let total_timeout = config.timeout + exposure_duration;
        let start = Instant::now();
        let is_continuous = config.is_continuous();

        let (width, height) = self.get_capture_dimensions(config);
        let channels = match config.format {
            ImageFormat::Raw8 | ImageFormat::Raw16 => 1,
            ImageFormat::Rgb24 => 3,
        };
        let bytes_per_channel = match config.format {
            ImageFormat::Raw8 | ImageFormat::Rgb24 => 1,
            ImageFormat::Raw16 => 2,
        };

        let required_size = (width * height * channels * bytes_per_channel) as usize;
        let mut buffer = self.buffer_pool.get(required_size);

        if config.should_reapply(self.last_applied_config.as_ref()) {
            if self.stream_running {
                let _ = catch_ffi_panic("ZWO::stop_video_capture", || {
                    self.camera.stop_video_capture()
                });
                self.stream_running = false;
            }
            self.apply_config(config)?;
            self.last_applied_config = Some(config.clone());
        }

        if is_continuous {
            if !self.stream_running {
                catch_ffi_panic("ZWO::start_video_capture", || {
                    self.camera.start_video_capture()
                })
                .map_err(CameraError::from)?
                .map_err(CameraError::ExposureFailed)?;
                self.stream_running = true;
            }

            // Loop just to allow cancellation while waiting for blocking ASIGetVideoData
            // Actually, ASIGetVideoData is a single blocking call. To allow cancellation,
            // we'd need to either use a shorter timeout and loop, or wait in a thread.
            // ZWO SDK video capture timeout is in milliseconds.
            // We can pass a shorter timeout (e.g. 100ms) and loop, checking cancel_flag.
            let mut got_frame = false;
            while start.elapsed() <= total_timeout {
                if self.cancel_flag.load(Ordering::SeqCst) {
                    let _ = catch_ffi_panic("ZWO::stop_video_capture", || {
                        self.camera.stop_video_capture()
                    });
                    self.stream_running = false;
                    return Err(CameraError::Cancelled);
                }

                let wait_ms =
                    100.min(total_timeout.saturating_sub(start.elapsed()).as_millis() as i32);

                match catch_ffi_panic("ZWO::get_video_data", || {
                    self.camera.get_video_data(&mut buffer, wait_ms.max(10))
                }) {
                    Ok(Ok(())) => {
                        got_frame = true;
                        break;
                    }
                    Ok(Err(_)) => {
                        // Timeout or error, loop and retry if time remains. Avoid 100% CPU spin-loop
                        // if the SDK returns immediately on error.
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => {
                        let _ = catch_ffi_panic("ZWO::stop_video_capture", || {
                            self.camera.stop_video_capture()
                        });
                        self.stream_running = false;
                        return Err(CameraError::ImageReadFailed(e.to_string()));
                    }
                }
            }
            if !got_frame {
                let _ = catch_ffi_panic("ZWO::stop_video_capture", || {
                    self.camera.stop_video_capture()
                });
                self.stream_running = false;
                return Err(CameraError::ExposureTimeout(total_timeout));
            }
        } else {
            if self.stream_running {
                let _ = catch_ffi_panic("ZWO::stop_video_capture", || {
                    self.camera.stop_video_capture()
                });
                self.stream_running = false;
            }

            catch_ffi_panic("ZWO::start_exposure", || self.camera.start_capture())
                .map_err(CameraError::from)?
                .map_err(CameraError::ExposureFailed)?;

            loop {
                if self.cancel_flag.load(Ordering::SeqCst) {
                    let _ = catch_ffi_panic("ZWO::cancel_capture", || self.camera.stop_capture());
                    return Err(CameraError::Cancelled);
                }

                if start.elapsed() > total_timeout {
                    let _ = catch_ffi_panic("ZWO::cancel_capture", || self.camera.stop_capture());
                    return Err(CameraError::ExposureTimeout(total_timeout));
                }

                let ready_result =
                    catch_ffi_panic("ZWO::image_ready", || self.camera.is_image_ready())
                        .map_err(CameraError::from)?;

                match ready_result {
                    Ok(true) => break,
                    Ok(false) => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => {
                        let _ =
                            catch_ffi_panic("ZWO::cancel_capture", || self.camera.stop_capture());
                        return Err(CameraError::ExposureFailed(e));
                    }
                }
            }

            catch_ffi_panic("ZWO::download_image", || {
                self.camera.get_image_data(&mut buffer)
            })
            .map_err(CameraError::from)?
            .map_err(CameraError::ImageReadFailed)?;
        }

        Ok(RawFrame {
            data: buffer,
            width,
            height,
            format: config.format,
        })
    }

    fn invalidate_config_cache(&mut self) {
        self.last_applied_config = None;
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
