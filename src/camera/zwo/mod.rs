//! ZWO ASI camera implementation
//!
//! Uses the `cameraunit_asi` crate for safe Rust bindings to the ZWO ASI SDK.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use shim::{
    get_camera_ids, get_camera_properties, num_cameras, Camera as ZwoShimCamera, CameraInfoASI,
};

use super::device_lease::DeviceLease;
use super::device_lost::tolerate_unsupported;
use super::error::{CameraError, CameraResult};
use super::exposure::{Acquisition, ExposureLoop, Poll, Progress, SdkExposure};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, RawFrame};

mod props;

use props::{build_camera_info, camera_info_from_properties};

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
        ZwoCamera::list_cameras()
    }

    /// From the property table alone: `list_cameras` opens every device, which would
    /// take the lease of a camera the other role is exposing with. Sorted by camera id,
    /// the order `ZwoCamera::open` indexes. The SDK exposes no serial before open.
    fn identities(&self) -> CameraResult<Vec<crate::camera::DeviceIdentity>> {
        let ids =
            catch_ffi_panic("ZWO::get_camera_ids", get_camera_ids).map_err(CameraError::from)?;
        Ok(ids
            .unwrap_or_default()
            .into_iter()
            .map(|(id, name)| crate::camera::DeviceIdentity::new(name, None).with_device_id(id))
            .collect())
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
    exposure: ExposureLoop,
}

impl ZwoCamera {
    /// Get the number of connected ZWO cameras
    pub fn camera_count() -> CameraResult<usize> {
        let count = catch_ffi_panic("ZWO::num_cameras", num_cameras).map_err(CameraError::from)?;
        Ok(count.max(0) as usize)
    }

    /// List all connected cameras, one entry per position [`Self::open`] indexes.
    ///
    /// The capabilities need an open handle, but a device some handle still holds is
    /// described from its properties alone: opening it took a lease over the live handle's
    /// and closing it again closed the device underneath a running capture. A device that
    /// fails to open is described the same way — dropping it shifted every later entry
    /// onto its neighbour's index.
    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        let properties = catch_ffi_panic("ZWO::get_camera_properties", get_camera_properties)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(properties
            .iter()
            .map(|(&id, properties)| Self::describe(id, properties))
            .collect())
    }

    fn describe(id: i32, properties: &CameraInfoASI) -> CameraInfo {
        if DeviceLease::is_open(shim::PROVIDER, id) {
            return camera_info_from_properties(properties, id);
        }
        match catch_ffi_panic("ZWO::open_camera", || ZwoShimCamera::open(id)) {
            Ok(Ok((camera, opened))) => build_camera_info(&camera, &opened, id),
            _ => camera_info_from_properties(properties, id),
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
            exposure: ExposureLoop::new(),
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
                    exposure: ExposureLoop::new(),
                });
            }
        }

        Err(CameraError::OpenFailed(format!(
            "Camera '{}' not found",
            name
        )))
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
        let Self { camera, info, exposure } = self;
        exposure.capture(&mut ZwoExposure { camera, info }, config)
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
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "ZWO"
    }
}

/// ZWO's calls for the shared [`ExposureLoop`].
struct ZwoExposure<'a> {
    camera: &'a ZwoShimCamera,
    info: &'a CameraInfo,
}

impl SdkExposure for ZwoExposure<'_> {
    fn info(&self) -> &CameraInfo {
        self.info
    }

    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()> {
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

    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()> {
        let started = match acquisition {
            Acquisition::Stream => {
                catch_ffi_panic("ZWO::start_video_capture", || self.camera.start_video_capture())
            }
            Acquisition::Single => {
                catch_ffi_panic("ZWO::start_exposure", || self.camera.start_capture())
            }
        };
        started
            .map_err(CameraError::from)?
            .map_err(CameraError::ExposureFailed)
    }

    fn abort(&mut self, acquisition: Acquisition) {
        let _ = match acquisition {
            Acquisition::Stream => {
                catch_ffi_panic("ZWO::stop_video_capture", || self.camera.stop_video_capture())
            }
            Acquisition::Single => {
                catch_ffi_panic("ZWO::cancel_capture", || self.camera.stop_capture())
            }
        };
    }

    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize> {
        let (width, height) = config.frame_dimensions(self.info);
        let (channels, bytes_per_channel) = match config.format {
            ImageFormat::Raw8 => (1, 1),
            ImageFormat::Raw16 => (1, 2),
            ImageFormat::Rgb24 => (3, 1),
        };
        Ok((width * height * channels * bytes_per_channel) as usize)
    }

    fn poll(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        match progress.acquisition {
            Acquisition::Stream => self.poll_stream(progress, buffer),
            Acquisition::Single => self.poll_single(progress, buffer),
        }
    }
}

impl ZwoExposure<'_> {
    /// A short wait per call, so a cancel is seen between them.
    fn poll_stream(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        let wait_ms = 100.min(progress.left().as_millis() as i32).max(10);
        match catch_ffi_panic("ZWO::get_video_data", || {
            self.camera.get_video_data(buffer, wait_ms)
        }) {
            Ok(Ok(())) => {
                let (width, height) = progress.config.frame_dimensions(self.info);
                Poll::Ready { width, height }
            }
            // A lost device has no stream left to stop.
            Ok(Err(e)) if crate::camera::device_lost::is_marked(&e) => Poll::Failed {
                error: CameraError::ImageReadFailed(e),
                stream_ended: true,
            },
            Ok(Err(_)) => {
                std::thread::sleep(Duration::from_millis(5));
                Poll::Pending
            }
            Err(e) => {
                self.abort(Acquisition::Stream);
                Poll::Failed {
                    error: CameraError::ImageReadFailed(e.to_string()),
                    stream_ended: true,
                }
            }
        }
    }

    fn poll_single(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        let failed = |error| Poll::Failed {
            error,
            stream_ended: false,
        };
        match catch_ffi_panic("ZWO::image_ready", || self.camera.is_image_ready()) {
            Err(e) => return failed(CameraError::from(e)),
            Ok(Ok(true)) => {}
            Ok(Ok(false)) => {
                std::thread::sleep(Duration::from_millis(5));
                return Poll::Pending;
            }
            Ok(Err(e)) => {
                self.abort(Acquisition::Single);
                return failed(CameraError::ExposureFailed(e));
            }
        }
        match catch_ffi_panic("ZWO::download_image", || self.camera.get_image_data(buffer)) {
            Ok(Ok(())) => {
                let (width, height) = progress.config.frame_dimensions(self.info);
                Poll::Ready { width, height }
            }
            Ok(Err(e)) => failed(CameraError::ImageReadFailed(e)),
            Err(e) => failed(CameraError::from(e)),
        }
    }
}

#[cfg(test)]
mod tests;
