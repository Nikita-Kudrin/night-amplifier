use super::ffi_types::POAImgFormat;
use super::shim::Camera as POACamera;

pub struct ROI {
    pub start_x: u32,
    pub start_y: u32,
    pub width: u32,
    pub height: u32,
}
use std::time::Duration;
use tracing::{debug, warn};

use super::super::error::{CameraError, CameraResult};
use super::super::exposure::{Acquisition, Poll, Progress, SdkExposure};
use super::super::types::{CameraInfo, CaptureConfig, ImageFormat};
use super::sensor_mode;
use crate::ffi_safety::catch_ffi_panic;

pub fn apply_config(
    camera: &mut POACamera,
    config: &CaptureConfig,
    info: &CameraInfo,
) -> CameraResult<()> {
    // Set exposure time
    let exposure_us = config.exposure_us;
    catch_ffi_panic("PlayerOne::set_exposure", || {
        camera.set_exposure(exposure_us as i64, false)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set exposure: {:?}", e),
    })?;

    // Set gain
    let gain = config.gain;
    catch_ffi_panic("PlayerOne::set_gain", || {
        camera.set_gain(gain as i64, false)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set gain: {:?}", e),
    })?;

    // Set offset
    let offset = config.offset;
    catch_ffi_panic("PlayerOne::set_offset", || camera.set_offset(offset as i64))
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set offset: {:?}", e),
        })?;

    // Set binning
    let bin = config.bin;
    catch_ffi_panic("PlayerOne::set_bin", || camera.set_bin(bin as u32))
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set binning: {:?}", e),
        })?;

    // Set image format
    let format: POAImgFormat = match config.format {
        ImageFormat::Raw8 => POAImgFormat::POA_RAW8,
        ImageFormat::Raw16 => POAImgFormat::POA_RAW16,
        ImageFormat::Rgb24 => POAImgFormat::POA_RGB24,
    };
    catch_ffi_panic("PlayerOne::set_image_format", || {
        camera.set_image_format(format)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set image format: {:?}", e),
    })?;

    // Set ROI or full frame
    if let Some((x, y, w, h)) = config.roi {
        let roi = ROI {
            start_x: x,
            start_y: y,
            width: w,
            height: h,
        };
        catch_ffi_panic("PlayerOne::set_roi", || camera.set_roi(&roi))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set ROI: {:?}", e),
            })?;
    } else {
        let width = info.max_width / config.bin as u32;
        let height = info.max_height / config.bin as u32;
        let roi = ROI {
            start_x: 0,
            start_y: 0,
            width,
            height,
        };
        catch_ffi_panic("PlayerOne::set_roi", || camera.set_roi(&roi))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set image size: {:?}", e),
            })?;
    }

    if !info.sensor_modes.is_empty() {
        apply_sensor_mode(camera, config, info);
    }

    if info.has_cooler {
        apply_cooler_config(camera, config);
    }

    Ok(())
}

fn apply_sensor_mode(camera: &mut POACamera, config: &CaptureConfig, info: &CameraInfo) {
    let Some(desired) = config.sensor_mode else {
        return;
    };
    let camera_id = camera.id();
    match sensor_mode::resolve_mode_index(&info.sensor_modes, desired) {
        Some(index) => {
            if let Err(err) = sensor_mode::set_sensor_mode(camera_id, index) {
                warn!(?err, index, ?desired, "Failed to set sensor mode");
                return;
            }
            // `set_sensor_mode` reported success, but the SDK call is
            // documented as a no-op if issued at the wrong moment (e.g.
            // while an exposure is in progress) — read the mode back so a
            // silent no-take isn't just trusted and cached forever by
            // `should_reapply` (see `Camera::invalidate_config_cache` docs).
            match sensor_mode::current_sensor_mode(camera_id) {
                Some(actual) if actual == index => {
                    debug!(index, ?desired, "Sensor mode applied and verified");
                }
                Some(actual) => {
                    warn!(
                        requested = index,
                        actual,
                        ?desired,
                        "Sensor mode set call succeeded but camera reports a different mode — hardware did not take the change"
                    );
                }
                None => {
                    warn!(
                        index,
                        ?desired,
                        "Could not read back sensor mode to verify apply"
                    );
                }
            }
        }
        None => {
            warn!(?desired, modes = ?info.sensor_modes, "Desired sensor mode not found on camera")
        }
    }
}

fn apply_cooler_config(camera: &mut POACamera, config: &CaptureConfig) {
    if config.cooler_enabled {
        if let Some(temp) = config.target_temp_c {
            let result = catch_ffi_panic("PlayerOne::set_target_temperature", || {
                camera.set_target_temperature(temp as i64)
            });
            match result {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    warn!(error = ?e, target_temp_c = temp, "Failed to set target temperature")
                }
                Err(e) => warn!(error = %e, "Panic setting target temperature"),
            }
        }
    }
    let result = catch_ffi_panic("PlayerOne::set_cooler", || {
        camera.set_cooler(config.cooler_enabled)
    });
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            warn!(error = ?e, enabled = config.cooler_enabled, "Failed to set cooler state")
        }
        Err(e) => warn!(error = %e, "Panic setting cooler state"),
    }
}

/// `start` is when `capture()` was entered, config reapply included — see
/// [`CaptureConfig::stall_budget`].
/// Player One's calls for the shared [`ExposureLoop`](crate::camera::exposure::ExposureLoop).
pub(super) struct PlayerOneExposure<'a> {
    pub camera: &'a mut POACamera,
    pub info: &'a CameraInfo,
}

impl SdkExposure for PlayerOneExposure<'_> {
    fn info(&self) -> &CameraInfo {
        self.info
    }

    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        apply_config(self.camera, config, self.info)
    }

    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()> {
        let (single, context) = match acquisition {
            Acquisition::Stream => (false, "PlayerOne::start_exposure(false)"),
            Acquisition::Single => (true, "PlayerOne::start_exposure(true)"),
        };
        catch_ffi_panic(context, || self.camera.start_exposure(single))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::ExposureFailed(format!("{:?}", e)))
    }

    fn abort(&mut self, _acquisition: Acquisition) {
        let _ = catch_ffi_panic("PlayerOne::stop_exposure", || self.camera.stop_exposure());
    }

    /// Read before the stop, which resets it: whether the SDK was receiving frames and
    /// dropping them, or receiving nothing, separates two different faults.
    fn on_stall(&mut self, progress: &Progress) {
        if progress.acquisition != Acquisition::Stream {
            return;
        }
        match catch_ffi_panic("PlayerOne::dropped_images_count", || {
            self.camera.dropped_images_count()
        }) {
            Ok(Some(Ok(dropped))) => {
                warn!(dropped, budget = ?progress.budget, "Player One video stream stalled")
            }
            Ok(Some(Err(e))) => debug!(error = %e, "Could not read the dropped-frame count"),
            Ok(None) | Err(_) => {}
        }
    }

    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize> {
        let (width, height) = config.frame_dimensions(self.info);
        let bytes_per_pixel = match config.format {
            ImageFormat::Raw8 => 1,
            ImageFormat::Raw16 => 2,
            ImageFormat::Rgb24 => 3,
        };
        Ok((width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(bytes_per_pixel))
    }

    fn poll(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        let failed = |error, stream_ended| Poll::Failed {
            error,
            stream_ended,
        };
        let ready = match catch_ffi_panic("PlayerOne::is_image_ready", || {
            self.camera.is_image_ready()
        }) {
            Ok(ready) => ready,
            Err(e) => return failed(CameraError::from(e), false),
        };
        match ready {
            Ok(true) => {}
            Ok(false) => {
                std::thread::sleep(Duration::from_millis(5));
                return Poll::Pending;
            }
            Err(e) => {
                let _ = catch_ffi_panic("PlayerOne::stop_exposure", || self.camera.stop_exposure());
                let stream_ended = progress.acquisition == Acquisition::Stream;
                return failed(CameraError::ExposureFailed(format!("{:?}", e)), stream_ended);
            }
        }
        match catch_ffi_panic("PlayerOne::get_image_data", || {
            self.camera.get_image_data(buffer, Some(500))
        }) {
            Ok(Ok(())) => {
                let (width, height) = progress.config.frame_dimensions(self.info);
                Poll::Ready { width, height }
            }
            Ok(Err(e)) => failed(CameraError::ImageReadFailed(format!("{:?}", e)), false),
            Err(e) => failed(CameraError::from(e), false),
        }
    }

    fn finish_single(&mut self) -> CameraResult<()> {
        catch_ffi_panic("PlayerOne::stop_exposure", || self.camera.stop_exposure())
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::ExposureFailed(format!("{:?}", e)))
    }
}
