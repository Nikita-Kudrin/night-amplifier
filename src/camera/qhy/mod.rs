//! QHYCCD camera implementation

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};
use shim::{scan_cameras, QhyHandle};
use ffi_types::ControlId;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, SensorType};

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

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let ids = catch_ffi_panic("QHY::scan_cameras", scan_cameras)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        let mut cameras = Vec::new();
        // Unlike ZWO (ASIGetCameraProperty), QHY requires opening each camera to
        // query chip info. This open/init/close cycle per camera is unavoidable
        // and may be slow on systems with many QHY devices.
        for (i, id) in ids.iter().enumerate() {
            if let Ok(Ok(cam)) = catch_ffi_panic("QHY::open_camera", || QhyHandle::open(id)) {
                if let Ok(Ok(chip)) = catch_ffi_panic("QHY::chip_info", || cam.chip_info()) {
                    let mut info = CameraInfo {
                        name: id.clone(),
                        id: i as i32,
                        max_width: chip.img_w,
                        max_height: chip.img_h,
                        pixel_size_x_um: chip.pixel_w,
                        pixel_size_y_um: chip.pixel_h,
                        sensor_type: if chip.bayer == "MONO" {
                            SensorType::Mono
                        } else {
                            SensorType::Color
                        },
                        ..Default::default()
                    };
                    if let Ok(Ok((_, max, _))) = catch_ffi_panic("QHY::gain_range", || cam.param_range(ControlId::Gain)) {
                        info.max_gain = max as i32;
                    }
                    

                    info.has_cooler = cam.is_control_available(ControlId::Cooler);
                    info.supported_bins = cam.supported_bins();
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
                    cameras.push(info);
                }
            }
        }
        Ok(cameras)
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = QhyCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

pub struct QhyCamera {
    camera: QhyHandle,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
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

        let chip = catch_ffi_panic("QHY::chip_info", || camera.chip_info())
            .map_err(CameraError::from)?
            .map_err(CameraError::OpenFailed)?;

        let mut info = CameraInfo {
            name: id.clone(),
            id: index as i32,
            max_width: chip.img_w,
            max_height: chip.img_h,
            pixel_size_x_um: chip.pixel_w,
            pixel_size_y_um: chip.pixel_h,
            sensor_type: if chip.bayer == "MONO" {
                SensorType::Mono
            } else {
                SensorType::Color
            },
            ..Default::default()
        };
        if let Ok(Ok((_, max, _))) = catch_ffi_panic("QHY::gain_range", || camera.param_range(ControlId::Gain)) {
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
        
        let slf = Self {
            camera,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        };

        // Initialize defaults
        let _ = slf.camera.set_stream_mode(0); // Single frame mode
        let _ = slf.camera.set_param(ControlId::TransferBit, if chip.bpp > 8 { 16.0 } else { 8.0 });
        let _ = slf.camera.set_resolution(0, 0, chip.img_w, chip.img_h);
        
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
        Err(CameraError::OpenFailed(format!("Camera '{}' not found", name)))
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_capture_config(
        &self,
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
            self.camera.set_param(ControlId::Exposure, exposure_us as f64)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set exposure: {}", e),
        })?;

        catch_ffi_panic("QHY::set_gain", || {
            self.camera.set_param(ControlId::Gain, gain as f64)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set gain: {}", e),
        })?;

        catch_ffi_panic("QHY::set_offset", || {
            self.camera.set_param(ControlId::Offset, offset as f64)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set offset: {}", e),
        })?;

        catch_ffi_panic("QHY::set_resolution", || {
            self.camera.set_resolution(x, y, w, h)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: format!("Failed to set resolution: {}", e),
        })?;

        catch_ffi_panic("QHY::set_bin", || self.camera.set_bin(bin))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set bin mode: {}", e),
            })?;

        catch_ffi_panic("QHY::set_bits", || self.camera.set_bits(bits))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::SdkError {
                code: -1,
                message: format!("Failed to set bit mode: {}", e),
            })?;

        Ok(())
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
        let temp = catch_ffi_panic("QHY::current_temp", || self.camera.current_temperature())
            .map_err(CameraError::from)?
            .unwrap_or(0.0);

        let pwm = catch_ffi_panic("QHY::cooler_power", || self.camera.cooler_power())
            .map_err(CameraError::from)?
            .unwrap_or(0.0);

        let current_gain = catch_ffi_panic("QHY::get_gain", || {
            self.camera.get_param(ControlId::Gain)
        })
        .map_err(CameraError::from)?
        .unwrap_or(0.0) as i32;

        let current_offset = catch_ffi_panic("QHY::get_offset", || {
            self.camera.get_param(ControlId::Offset)
        })
        .map_err(CameraError::from)?
        .unwrap_or(0.0) as i32;

        let current_exposure_us = catch_ffi_panic("QHY::get_exposure", || {
            self.camera.get_param(ControlId::Exposure)
        })
        .map_err(CameraError::from)?
        .unwrap_or(0.0) as u64;

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
        catch_ffi_panic("QHY::set_temp", || self.camera.set_target_temperature(temp_c))
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

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        config.validate(&self.info)?;
        self.cancel_flag.store(false, Ordering::SeqCst);

        let bin = config.bin as u32;
        let bits = match config.format {
            ImageFormat::Raw8 | ImageFormat::Rgb24 => 8,
            ImageFormat::Raw16 => 16,
        };

        let (x, y, w, h) = if let Some((x, y, w, h)) = config.roi {
            (x, y, w, h)
        } else {
            (0, 0, self.info.max_width / bin, self.info.max_height / bin)
        };

        self.apply_capture_config(config.exposure_us, config.gain, config.offset, x, y, w, h, bin, bits)?;

        let exposure_duration = Duration::from_micros(config.exposure_us);
        let total_timeout = config.timeout + exposure_duration;
        let start = Instant::now();

        catch_ffi_panic("QHY::start_single", || self.camera.start_single_frame())
            .map_err(CameraError::from)?
            .map_err(CameraError::ExposureFailed)?;

        let mut buf_len = (w * h * (bits / 8)) as usize;
        if self.info.sensor_type == SensorType::Color && config.format == ImageFormat::Rgb24 {
            buf_len *= 3;
        }
        let mut buffer = vec![0u8; buf_len];

        loop {
            if self.cancel_flag.load(Ordering::SeqCst) {
                let _ = catch_ffi_panic("QHY::cancel", || self.camera.cancel());
                return Err(CameraError::Cancelled);
            }

            if start.elapsed() > total_timeout {
                let _ = catch_ffi_panic("QHY::cancel", || self.camera.cancel());
                return Err(CameraError::ExposureTimeout(total_timeout));
            }

            let ready = catch_ffi_panic("QHY::get_single", || self.camera.get_single_frame(&mut buffer));
            match ready {
                Ok(Ok((bw, bh))) => {
                    let channels = if self.info.sensor_type == SensorType::Color && config.format == ImageFormat::Rgb24 { 3 } else { 1 };
                    let pixel_format = match config.format {
                        ImageFormat::Raw8 => if channels == 1 && self.info.sensor_type == SensorType::Color { PixelFormat::Bayer8 } else { PixelFormat::Rgb8 },
                        ImageFormat::Raw16 => if channels == 1 && self.info.sensor_type == SensorType::Color { PixelFormat::Bayer16 } else { PixelFormat::Rgb16 },
                        ImageFormat::Rgb24 => PixelFormat::Rgb8,
                    };

                    if self.info.sensor_type == SensorType::Color && channels == 1 {
                        let pattern = self.info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
                        return Frame::from_bayer(&buffer, bw as usize, bh as usize, pixel_format, pattern)
                            .map_err(|e| CameraError::ImageReadFailed(e.to_string()));
                    } else {
                        return Frame::from_raw(&buffer, bw as usize, bh as usize, channels, pixel_format)
                            .map_err(|e| CameraError::ImageReadFailed(e.to_string()));
                    }
                }
                Ok(Err(e)) => {
                    // QHY GetQHYCCDSingleFrame usually returns READ_DIRECTLY or ERROR if it's not ready yet.
                    if e == "QHYCCD_READ_DIRECTLY" || e == "QHYCCD_ERROR" {
                        let elapsed = start.elapsed();
                        if elapsed < exposure_duration.saturating_sub(Duration::from_millis(50)) {
                            // Initial backoff: sleep until 50ms before exposure ends, max 100ms at a time
                            let remaining = exposure_duration - elapsed - Duration::from_millis(50);
                            std::thread::sleep(remaining.min(Duration::from_millis(100)));
                        } else {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        continue;
                    } else {
                        return Err(CameraError::ExposureFailed(e));
                    }
                }
                Err(e) => {
                    return Err(CameraError::ExposureFailed(e.to_string()));
                }
            }
        }
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        let _ = catch_ffi_panic("QHY::close", || self.camera.close());
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "QHY"
    }
}

#[cfg(test)]
mod tests;
