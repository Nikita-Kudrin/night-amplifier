//! ToupTek camera implementation

use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};
use shim::{enumerate_devices, parse_fourcc_bayer, TouptekHandle};

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, SensorType};

use ffi_types::*;

/// ToupTek camera provider
pub struct TouptekProvider;

impl TouptekProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TouptekProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for TouptekProvider {
    fn name(&self) -> &'static str {
        "ToupTek"
    }

    fn is_available(&self) -> bool {
        sdk::TouptekSdk::try_load().is_some()
    }

    fn camera_count(&self) -> CameraResult<usize> {
        let devices = catch_ffi_panic("ToupTek::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(devices.len())
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let devices = catch_ffi_panic("ToupTek::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        let mut cameras = Vec::new();
        for (i, dev) in devices.iter().enumerate() {
            if let Some(info) = build_camera_info_from_device(dev, i as i32) {
                cameras.push(info);
            }
        }
        Ok(cameras)
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = TouptekCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

pub struct TouptekCamera {
    handle: TouptekHandle,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
    cooler_on: bool,
}

impl TouptekCamera {
    pub fn open(index: usize) -> CameraResult<Self> {
        let devices = catch_ffi_panic("ToupTek::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        if index >= devices.len() {
            return Err(CameraError::InvalidCameraIndex {
                index,
                count: devices.len(),
            });
        }

        let dev = &devices[index];
        let cam_id = c_char_array_to_string(&dev.id);

        let handle = catch_ffi_panic("ToupTek::open", || TouptekHandle::open(&cam_id))
            .map_err(CameraError::from)?
            .map_err(CameraError::OpenFailed)?;

        let info = build_camera_info_from_handle(&handle, dev, index as i32)?;

        // Configure for astronomy: RAW mode, auto-exposure off, max speed
        catch_ffi_panic("ToupTek::set_raw", || handle.set_raw_mode(true))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set RAW mode: {}", e),
            })?;

        let _ = catch_ffi_panic("ToupTek::set_autoexpo", || handle.set_auto_exposure(false));
        let _ = catch_ffi_panic("ToupTek::set_speed", || handle.set_max_speed());

        // Enable high bit depth if camera supports it
        let max_bpp = catch_ffi_panic("ToupTek::max_bpp", || handle.max_bit_depth())
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(8);
        if max_bpp > 8 {
            let _ = catch_ffi_panic("ToupTek::set_bitdepth", || handle.set_bit_depth(true));
        }

        Ok(Self {
            handle,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cooler_on: false,
        })
    }
}

impl Camera for TouptekCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        // ToupTek gain is in percent (100 = 1x). Generic defaults for the
        // gain scale — camera-specific tuning may be needed for exact values.
        Ok(GainPresets {
            highest_dr: self.info.min_gain,
            hcg: self.info.min_gain,
            unity: 100,
            lowest_rn: self.info.max_gain,
            offset_highest_dr: 0,
            offset_hcg: 0,
            offset_unity: 0,
            offset_lowest_rn: 0,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        // If getting exposure fails, the camera is disconnected
        let current_exposure_us =
            catch_ffi_panic("ToupTek::get_expo", || self.handle.get_exposure_us())
                .map_err(CameraError::from)?
                .map_err(|_e| CameraError::Disconnected)? as u64;

        let temperature_c = if self.info.has_cooler {
            catch_ffi_panic("ToupTek::get_temp", || self.handle.get_temperature_c())
                .map_err(CameraError::from)?
                .unwrap_or(0.0)
        } else {
            0.0
        };

        let current_gain = catch_ffi_panic("ToupTek::get_gain", || self.handle.get_gain())
            .map_err(CameraError::from)?
            .unwrap_or(100) as i32;

        Ok(CameraStatus {
            temperature_c,
            cooler_power: None,
            cooler_on: self.cooler_on,
            is_exposing: false,
            current_gain,
            current_offset: 0,
            current_exposure_us,
            dew_heater_on: false,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("ToupTek::set_temp", || {
            self.handle.set_target_temperature_c(temp_c)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::CoolingFailed)?;
        self.cooler_on = true;
        Ok(())
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        // ToupTek TEC is controlled implicitly via put_Temperature, but we track
        // the intended state so the UI reflects it.
        self.cooler_on = enabled;
        Ok(())
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::ParameterNotSupported(
            "dew_heater".to_string(),
        ))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        config.validate(&self.info)?;
        self.cancel_flag.store(false, Ordering::SeqCst);

        let bin = config.bin;

        // Set exposure
        catch_ffi_panic("ToupTek::set_expo", || {
            self.handle.set_exposure_us(config.exposure_us as u32)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::ExposureFailed)?;

        // Set gain (Camera trait uses i32, ToupTek SDK uses u16 percent)
        catch_ffi_panic("ToupTek::set_gain", || {
            self.handle.set_gain(config.gain as u16)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set gain: {}", e),
        })?;

        // Set binning
        if bin > 1 {
            catch_ffi_panic("ToupTek::set_bin", || self.handle.set_binning(bin))
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::SdkError {
                    code: -1,
                    message: format!("Failed to set binning: {}", e),
                })?;
        }

        // Set ROI or full resolution
        if let Some((x, y, w, h)) = config.roi {
            catch_ffi_panic("ToupTek::set_roi", || self.handle.set_roi(x, y, w, h))
                .map_err(CameraError::from)?
                .map_err(|e| CameraError::SdkError {
                    code: -1,
                    message: format!("Failed to set ROI: {}", e),
                })?;
        } else {
            // Full frame at index 0 (highest resolution)
            catch_ffi_panic("ToupTek::set_esize", || {
                self.handle.set_resolution_index(0)
            })
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set resolution: {}", e),
            })?;
        }

        // Start pull mode
        catch_ffi_panic("ToupTek::start_pull", || self.handle.start_pull_mode())
            .map_err(CameraError::from)?
            .map_err(CameraError::ExposureFailed)?;

        // Calculate buffer size
        let (w, h) = catch_ffi_panic("ToupTek::get_size", || self.handle.get_resolution())
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to get resolution: {}", e),
            })?;
        let w = w as u32;
        let h = h as u32;

        let bytes_per_pixel = match config.format {
            ImageFormat::Raw16 => 2usize,
            ImageFormat::Raw8 => 1,
            ImageFormat::Rgb24 => 3,
        };
        let buf_size = (w as usize) * (h as usize) * bytes_per_pixel;
        let mut buffer = vec![0u8; buf_size];

        // Calculate timeout: exposure + margin
        let exposure_duration = Duration::from_micros(config.exposure_us);
        let total_timeout = config.timeout + exposure_duration;
        let wait_ms = total_timeout.as_millis().min(u32::MAX as u128) as u32;

        let start = Instant::now();

        let fatal_error = self.handle.get_fatal_error_flag();

        // Wait for the image
        let frame_info = loop {
            if self.cancel_flag.load(Ordering::SeqCst) {
                let _ = catch_ffi_panic("ToupTek::stop", || self.handle.stop());
                return Err(CameraError::Cancelled);
            }

            if start.elapsed() > total_timeout {
                let _ = catch_ffi_panic("ToupTek::stop", || self.handle.stop());
                return Err(CameraError::ExposureTimeout(total_timeout));
            }

            if fatal_error.load(Ordering::SeqCst) {
                let _ = catch_ffi_panic("ToupTek::stop", || self.handle.stop());
                return Err(CameraError::ExposureFailed("Camera reported a hardware error or disconnect during capture".to_string()));
            }

            // Try to pull with a short wait — allows cancel checks
            let chunk_wait = 500u32.min(wait_ms);
            match catch_ffi_panic("ToupTek::wait_image", || {
                self.handle.wait_image_raw(chunk_wait, &mut buffer)
            }) {
                Ok(Ok(info)) => break info,
                Ok(Err(_)) => continue, // Not ready yet
                Err(e) => {
                    let _ = catch_ffi_panic("ToupTek::stop", || self.handle.stop());
                    return Err(CameraError::ExposureFailed(e.to_string()));
                }
            }
        };

        // Stop pull mode
        let _ = catch_ffi_panic("ToupTek::stop", || self.handle.stop());

        // Build Frame
        let actual_w = frame_info.width as usize;
        let actual_h = frame_info.height as usize;

        let is_color = self.info.sensor_type == SensorType::Color;
        let pixel_format = match config.format {
            ImageFormat::Raw8 => {
                if is_color {
                    PixelFormat::Bayer8
                } else {
                    PixelFormat::Rgb8
                }
            }
            ImageFormat::Raw16 => {
                if is_color {
                    PixelFormat::Bayer16
                } else {
                    PixelFormat::Rgb16
                }
            }
            ImageFormat::Rgb24 => PixelFormat::Rgb8,
        };

        if is_color {
            let pattern = self.info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
            Frame::from_bayer(&buffer, actual_w, actual_h, pixel_format, pattern)
                .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        } else {
            Frame::from_raw(&buffer, actual_w, actual_h, 1, pixel_format)
                .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        let _ = catch_ffi_panic("ToupTek::close", || self.handle.close());
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "ToupTek"
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn c_char_array_to_string(arr: &[i8]) -> String {
    let ptr = arr.as_ptr();
    unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Build CameraInfo from enumeration data only (no handle needed).
fn build_camera_info_from_device(dev: &ToupcamDeviceV2, id: i32) -> Option<CameraInfo> {
    if dev.model.is_null() {
        return None;
    }

    let model = unsafe { &*dev.model };
    let name = c_char_array_to_string(&dev.displayname);
    let flag = model.flag;

    let max_width = if model.preview > 0 {
        model.res[0].width
    } else {
        0
    };
    let max_height = if model.preview > 0 {
        model.res[0].height
    } else {
        0
    };

    let is_mono = (flag & TOUPCAM_FLAG_MONO as u64) != 0;
    let has_cooler = (flag & TOUPCAM_FLAG_TEC_ONOFF as u64) != 0;

    // Determine supported formats from flags
    let mut supported_formats = Vec::new();
    if (flag & TOUPCAM_FLAG_RAW16 as u64) != 0
        || (flag & TOUPCAM_FLAG_RAW14 as u64) != 0
        || (flag & TOUPCAM_FLAG_RAW12 as u64) != 0
        || (flag & TOUPCAM_FLAG_RAW10 as u64) != 0
    {
        supported_formats.push(ImageFormat::Raw16);
    }
    if (flag & TOUPCAM_FLAG_RAW8 as u64) != 0 || supported_formats.is_empty() {
        supported_formats.push(ImageFormat::Raw8);
    }

    // Determine bit depth from flags
    let bit_depth = if (flag & TOUPCAM_FLAG_RAW16 as u64) != 0 {
        16
    } else if (flag & TOUPCAM_FLAG_RAW14 as u64) != 0 {
        14
    } else if (flag & TOUPCAM_FLAG_RAW12 as u64) != 0 {
        12
    } else if (flag & TOUPCAM_FLAG_RAW10 as u64) != 0 {
        10
    } else {
        8
    };

    // Binning support: ToupTek uses digital binning via TOUPCAM_OPTION_BINNING.
    // Almost all cameras support 1x1 and 2x2; 3x3/4x4 depend on the model.
    let supported_bins = if (flag & TOUPCAM_FLAG_BINSKIP_SUPPORTED as u64) != 0 {
        vec![1, 2, 3, 4]
    } else {
        vec![1, 2]
    };

    Some(CameraInfo {
        name,
        id,
        max_width,
        max_height,
        pixel_size_x_um: model.xpixsz as f64,
        pixel_size_y_um: model.ypixsz as f64,
        sensor_type: if is_mono {
            SensorType::Mono
        } else {
            SensorType::Color
        },
        bayer_pattern: None, // Set later when handle is opened
        has_cooler,
        min_temp_c: if has_cooler { Some(-40.0) } else { None },
        max_temp_c: if has_cooler { Some(20.0) } else { None },
        has_shutter: false,
        is_usb3: (flag & TOUPCAM_FLAG_USB30 as u64) != 0,
        bit_depth,
        supported_bins,
        supported_formats,
        min_exposure_us: 1,
        max_exposure_us: 2_000_000_000,
        min_gain: 100,  // 1x in ToupTek percent
        max_gain: 1500, // 15x, will be refined when handle is opened
        unity_gain: 100,
        hcg_gain: 100,
        sensor_modes: Vec::new(),
        has_dew_heater: false,
    })
}

/// Build CameraInfo with a live handle (more accurate than enumeration alone).
fn build_camera_info_from_handle(
    handle: &TouptekHandle,
    dev: &ToupcamDeviceV2,
    id: i32,
) -> CameraResult<CameraInfo> {
    let mut info =
        build_camera_info_from_device(dev, id).ok_or(CameraError::OpenFailed(
            "ToupTek device model pointer is null".to_string(),
        ))?;

    // Refine gain range from the SDK
    if let Ok(Ok((min, max, _def))) =
        catch_ffi_panic("ToupTek::gain_range", || handle.gain_range())
    {
        info.min_gain = min as i32;
        info.max_gain = max as i32;
    }

    // Refine exposure range from the SDK
    if let Ok(Ok((min, max, _def))) =
        catch_ffi_panic("ToupTek::expo_range", || handle.exposure_range())
    {
        info.min_exposure_us = min as u64;
        info.max_exposure_us = max as u64;
    }

    // Refine pixel size from the SDK
    if let Ok(Ok((px, py))) =
        catch_ffi_panic("ToupTek::pixel_size", || handle.pixel_size(0))
    {
        info.pixel_size_x_um = px as f64;
        info.pixel_size_y_um = py as f64;
    }

    // Detect bayer pattern from raw format
    if let Ok(Ok((fourcc, _bpp))) =
        catch_ffi_panic("ToupTek::raw_format", || handle.raw_format())
    {
        info.bayer_pattern = parse_fourcc_bayer(fourcc);
        if info.bayer_pattern.is_some() {
            info.sensor_type = SensorType::Color;
        }
    }

    // Detect mono mode
    if let Ok(Ok(is_mono)) = catch_ffi_panic("ToupTek::mono_mode", || handle.is_mono()) {
        if is_mono {
            info.sensor_type = SensorType::Mono;
            info.bayer_pattern = None;
        }
    }

    // Refine bit depth from SDK
    if let Ok(Ok(bpp)) = catch_ffi_panic("ToupTek::max_bpp", || handle.max_bit_depth()) {
        info.bit_depth = bpp as u8;
    }

    Ok(info)
}

#[cfg(test)]
mod tests;
