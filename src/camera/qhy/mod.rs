//! QHYCCD camera implementation

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};
use ffi_types::ControlId;
use shim::{scan_cameras, QhyHandle};

use super::device_lost::tolerate_unsupported;
use super::error::{CameraError, CameraResult};
use super::exposure::{Acquisition, ExposureLoop, Poll, Progress, SdkExposure};
use super::traits::{Camera, CameraProvider};
use super::types::{
    CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, RawFrame, SensorType,
};

/// QHY camera provider
pub struct QhyProvider;

impl QhyProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for QhyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for QhyProvider {
    fn name(&self) -> &'static str {
        "QHY"
    }

    fn is_available(&self) -> bool {
        sdk::QhySdk::try_load().is_some()
    }

    fn camera_count(&self) -> CameraResult<usize> {
        let cameras = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(cameras.len())
    }

    /// Every scanned device, in scan order so positions match `open(index)`.
    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let ids = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(ids
            .iter()
            .enumerate()
            .map(|(index, id)| describe(id, index as i32))
            .collect())
    }

    /// From the scan alone: `list_cameras` opens every device. The QHY id string is
    /// model plus serial, so it serves as both.
    fn identities(&self) -> CameraResult<Vec<crate::camera::DeviceIdentity>> {
        let ids = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(ids
            .into_iter()
            .map(|id| {
                let serial = crate::camera::identity::normalize_serial(&id);
                crate::camera::DeviceIdentity::new(id, serial)
            })
            .collect())
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = QhyCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

/// Capabilities need an open handle (QHY has no `ASIGetCameraProperty`), but a device another
/// handle holds is described from its id alone and never opened — a second open of an open
/// QHY id is undocumented, and closing it could close the live camera. One that fails to
/// open is described the same way: dropping it shifted every later device onto its
/// neighbour's position.
fn describe(id: &str, index: i32) -> CameraInfo {
    catch_ffi_panic("QHY::open_camera", || QhyHandle::open_if_free(id))
        .ok()
        .and_then(Result::ok)
        .and_then(|camera| build_camera_info(&camera, id, index).ok())
        .unwrap_or_else(|| camera_info_from_id(id, index))
}

/// What the scan alone says: the QHY id is model plus serial.
fn camera_info_from_id(id: &str, index: i32) -> CameraInfo {
    CameraInfo {
        name: id.to_string(),
        id: index,
        serial: crate::camera::identity::normalize_serial(id),
        ..Default::default()
    }
}

fn build_camera_info(camera: &QhyHandle, id: &str, index: i32) -> CameraResult<CameraInfo> {
    let chip = catch_ffi_panic("QHY::chip_info", || camera.chip_info())
        .map_err(CameraError::from)?
        .map_err(CameraError::OpenFailed)?;

    let mut info = CameraInfo {
        max_width: chip.img_w,
        max_height: chip.img_h,
        pixel_size_x_um: chip.pixel_w,
        pixel_size_y_um: chip.pixel_h,
        sensor_type: if chip.bayer == "MONO" {
            SensorType::Mono
        } else {
            SensorType::Color
        },
        ..camera_info_from_id(id, index)
    };
    if let Ok(Ok((_, max, _))) =
        catch_ffi_panic("QHY::gain_range", || camera.param_range(ControlId::Gain))
    {
        info.max_gain = max as i32;
    }

    info.has_cooler = camera.is_control_available(ControlId::Cooler);
    info.supported_bins = camera.supported_bins();
    info.bayer_pattern = match chip.bayer.as_str() {
        "GBRG" => Some(CfaPattern::Gbrg),
        "GRBG" => Some(CfaPattern::Grbg),
        "BGGR" => Some(CfaPattern::Bggr),
        "RGGB" => Some(CfaPattern::Rggb),
        _ => None,
    };
    info.bit_depth = chip.bpp as u8;
    info.supported_formats = if chip.bpp > 8 {
        vec![ImageFormat::Raw16, ImageFormat::Raw8]
    } else {
        vec![ImageFormat::Raw8]
    };
    Ok(info)
}

pub struct QhyCamera {
    camera: QhyHandle,
    info: CameraInfo,
    exposure: ExposureLoop,
}

impl QhyCamera {
    pub fn open(index: usize) -> CameraResult<Self> {
        let ids = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        if index >= ids.len() {
            return Err(CameraError::InvalidCameraIndex {
                index,
                count: ids.len(),
            });
        }

        let id = &ids[index];
        let camera = catch_ffi_panic("QHY::open_camera", || QhyHandle::open(id))
            .map_err(CameraError::from)?
            .map_err(CameraError::OpenFailed)?;

        let info = build_camera_info(&camera, id, index as i32)?;

        let slf = Self {
            camera,
            info,
            exposure: ExposureLoop::new(),
        };

        // Initialize defaults
        let _ = slf.camera.set_stream_mode(0); // Single frame mode
        let _ = slf.camera.set_param(
            ControlId::TransferBit,
            if slf.info.bit_depth > 8 { 16.0 } else { 8.0 },
        );
        let _ = slf
            .camera
            .set_resolution(0, 0, slf.info.max_width, slf.info.max_height);

        Ok(slf)
    }

    pub fn open_by_name(name: &str) -> CameraResult<Self> {
        let ids = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        for (i, id) in ids.iter().enumerate() {
            if id.contains(name) {
                return Self::open(i);
            }
        }
        Err(CameraError::OpenFailed(format!(
            "Camera '{}' not found",
            name
        )))
    }
}

impl Camera for QhyCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        // Note: These HCG/unity values are placeholder defaults.
        // QHY models vary widely in their actual thresholds and do not share a single scale.
        Ok(GainPresets {
            highest_dr: 0,
            hcg: 30,
            unity: 50,
            lowest_rn: self.info.max_gain,
            offset_highest_dr: 10,
            offset_hcg: 20,
            offset_unity: 30,
            offset_lowest_rn: 50,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        // Every read goes through `tolerate_unsupported`: a parameter this
        // model does not expose falls back, but a lost device propagates so the
        // fault detector can see it. See `camera::device_lost`.
        let temp = tolerate_unsupported(
            catch_ffi_panic("QHY::current_temp", || self.camera.current_temperature())
                .map_err(CameraError::from)?,
            0.0,
        )?;

        let pwm = tolerate_unsupported(
            catch_ffi_panic("QHY::cooler_power", || self.camera.cooler_power())
                .map_err(CameraError::from)?,
            0.0,
        )?;

        let current_gain = tolerate_unsupported(
            catch_ffi_panic("QHY::get_gain", || self.camera.get_param(ControlId::Gain))
                .map_err(CameraError::from)?,
            0.0,
        )? as i32;

        let current_offset = tolerate_unsupported(
            catch_ffi_panic("QHY::get_offset", || {
                self.camera.get_param(ControlId::Offset)
            })
            .map_err(CameraError::from)?,
            0.0,
        )? as i32;

        let current_exposure_us = tolerate_unsupported(
            catch_ffi_panic("QHY::get_exposure", || {
                self.camera.get_param(ControlId::Exposure)
            })
            .map_err(CameraError::from)?,
            0.0,
        )? as u64;

        let cooler_on = pwm > 0.0;

        Ok(CameraStatus {
            temperature_c: temp,
            cooler_power: Some(pwm),
            cooler_on,
            is_exposing: false,
            current_gain,
            current_offset,
            current_exposure_us,
            dew_heater_on: false,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("QHY::set_temp", || {
            self.camera.set_target_temperature(temp_c)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::CoolingFailed)
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        if !enabled {
            // Disable TEC by setting manual PWM to 0
            catch_ffi_panic("QHY::set_manual_pwm", || {
                self.camera.set_param(ControlId::ManualPWM, 0.0)
            })
            .map_err(CameraError::from)?
            .map_err(CameraError::CoolingFailed)?;
        }
        // When enabling, the actual target is set by set_target_temperature
        // which calls SetQHYCCDParam(Cooler, temp) and switches to auto mode.
        Ok(())
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::ParameterNotSupported("dew_heater".to_string()))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
        let Self { camera, info, exposure } = self;
        exposure.capture(&mut QhyExposure { camera, info }, config)
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
        let _ = catch_ffi_panic("QHY::close", || self.camera.close());
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "QHY"
    }
}


#[allow(clippy::too_many_arguments)]
fn apply_capture_config(
    camera: &QhyHandle,
    exposure_us: u64,
    gain: i32,
    offset: i32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    bin: u32,
    bits: u32,
) -> CameraResult<()> {
    catch_ffi_panic("QHY::set_exposure", || {
        camera
            .set_param(ControlId::Exposure, exposure_us as f64)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set exposure: {}", e),
    })?;

    catch_ffi_panic("QHY::set_gain", || {
        camera.set_param(ControlId::Gain, gain as f64)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set gain: {}", e),
    })?;

    catch_ffi_panic("QHY::set_offset", || {
        camera.set_param(ControlId::Offset, offset as f64)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set offset: {}", e),
    })?;

    catch_ffi_panic("QHY::set_resolution", || {
        camera.set_resolution(x, y, w, h)
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::SdkError {
        code: -1,
        message: format!("Failed to set resolution: {}", e),
    })?;

    catch_ffi_panic("QHY::set_bin", || camera.set_bin(bin))
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set bin mode: {}", e),
        })?;

    catch_ffi_panic("QHY::set_bits", || camera.set_bits(bits))
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set bit mode: {}", e),
        })?;

    Ok(())
}

/// The sensor window `config` reads out: its ROI, or the whole binned sensor.
fn frame_window(config: &CaptureConfig, info: &CameraInfo) -> (u32, u32, u32, u32) {
    let bin = config.bin as u32;
    config
        .roi
        .unwrap_or((0, 0, info.max_width / bin, info.max_height / bin))
}

fn bit_depth(config: &CaptureConfig) -> u32 {
    match config.format {
        ImageFormat::Raw8 | ImageFormat::Rgb24 => 8,
        ImageFormat::Raw16 => 16,
    }
}

/// QHY's calls for the shared [`ExposureLoop`].
struct QhyExposure<'a> {
    camera: &'a QhyHandle,
    info: &'a CameraInfo,
}

impl SdkExposure for QhyExposure<'_> {
    fn info(&self) -> &CameraInfo {
        self.info
    }

    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        let (x, y, w, h) = frame_window(config, self.info);
        apply_capture_config(
            self.camera,
            config.exposure_us,
            config.gain,
            config.offset,
            x,
            y,
            w,
            h,
            config.bin as u32,
            bit_depth(config),
        )
    }

    fn start(&mut self, acquisition: Acquisition) -> CameraResult<()> {
        let started = match acquisition {
            Acquisition::Stream => {
                let _ = catch_ffi_panic("QHY::set_stream_mode", || self.camera.set_stream_mode(1));
                let _ = catch_ffi_panic("QHY::init", || self.camera.init());
                catch_ffi_panic("QHY::start_live", || self.camera.start_live())
            }
            Acquisition::Single => {
                catch_ffi_panic("QHY::start_single", || self.camera.start_single_frame())
            }
        };
        started
            .map_err(CameraError::from)?
            .map_err(CameraError::ExposureFailed)
    }

    /// Back to single-frame mode, which a cancel or a stall does not need: those only stop
    /// the stream, and the next start re-enters live mode anyway.
    fn end_stream(&mut self) {
        let _ = catch_ffi_panic("QHY::stop_live", || self.camera.stop_live());
        let _ = catch_ffi_panic("QHY::set_stream_mode", || self.camera.set_stream_mode(0));
        let _ = catch_ffi_panic("QHY::init", || self.camera.init());
    }

    fn abort(&mut self, acquisition: Acquisition) {
        let _ = match acquisition {
            Acquisition::Stream => catch_ffi_panic("QHY::stop_live", || self.camera.stop_live()),
            Acquisition::Single => catch_ffi_panic("QHY::cancel", || self.camera.cancel()),
        };
    }

    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize> {
        let (_, _, w, h) = frame_window(config, self.info);
        let mut len = (w * h * (bit_depth(config) / 8)) as usize;
        if self.info.sensor_type == SensorType::Color && config.format == ImageFormat::Rgb24 {
            len *= 3;
        }
        Ok(len)
    }

    fn poll(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        let read = match progress.acquisition {
            Acquisition::Stream => {
                catch_ffi_panic("QHY::get_live", || self.camera.get_live_frame(buffer))
            }
            Acquisition::Single => {
                catch_ffi_panic("QHY::get_single", || self.camera.get_single_frame(buffer))
            }
        };
        let error = match read {
            Ok(Ok((width, height))) => return Poll::Ready { width, height },
            // GetQHYCCDSingleFrame usually answers READ_DIRECTLY or ERROR while not ready.
            Ok(Err(e)) if e == "QHYCCD_READ_DIRECTLY" || e == "QHYCCD_ERROR" || e == "4294967295" => {
                back_off(progress);
                return Poll::Pending;
            }
            Ok(Err(e)) => CameraError::ExposureFailed(e),
            Err(e) => CameraError::ExposureFailed(e.to_string()),
        };
        let stream_ended = progress.acquisition == Acquisition::Stream;
        if stream_ended {
            let _ = catch_ffi_panic("QHY::stop_live", || self.camera.stop_live());
        }
        Poll::Failed {
            error,
            stream_ended,
        }
    }
}

/// Sleeps until 50 ms before the exposure ends, at most 100 ms at a time, then polls every
/// 5 ms.
fn back_off(progress: &Progress) {
    let exposure = Duration::from_micros(progress.config.exposure_us);
    let polling_from = exposure.saturating_sub(Duration::from_millis(50));
    if progress.waited < polling_from {
        std::thread::sleep((polling_from - progress.waited).min(Duration::from_millis(100)));
    } else {
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests;
