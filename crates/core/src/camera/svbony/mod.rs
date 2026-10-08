use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_long};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::warn;

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};
use shim::{
    enumerate_devices, get_camera_property, get_camera_property_ex, parse_fourcc_bayer,
    SvbonyHandle,
};

use super::device_lost::tolerate_unsupported;
use super::error::{CameraError, CameraResult};
use super::exposure::{Acquisition, ExposureLoop, Poll, Progress, SdkExposure};
use super::traits::{Camera, CameraProvider};
use super::types::{
    CameraInfo, CameraStatus, CaptureConfig, GainPresets, ImageFormat, RawFrame, SensorType,
};

use ffi_types::*;

pub struct SvbonyProvider;

impl SvbonyProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SvbonyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for SvbonyProvider {
    fn name(&self) -> &'static str {
        "SVBony"
    }

    fn is_available(&self) -> bool {
        sdk::SvbonySdk::try_load().is_some()
    }

    fn camera_count(&self) -> CameraResult<usize> {
        let devices = catch_ffi_panic("SVBony::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(devices.len())
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let devices = catch_ffi_panic("SVBony::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        let mut cameras = Vec::new();
        for (i, dev) in devices.iter().enumerate() {
            if let Some(info) = build_camera_info(dev, i as i32) {
                cameras.push(info);
            }
        }
        Ok(cameras)
    }

    /// Every enumerated device, including one whose property read failed: `open` indexes
    /// the raw enumeration, and `list_cameras` skipping a device would shift the rest.
    fn identities(&self) -> CameraResult<Vec<crate::camera::DeviceIdentity>> {
        let devices = catch_ffi_panic("SVBony::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();
        Ok(devices
            .iter()
            .map(|dev| {
                crate::camera::DeviceIdentity::new(
                    c_char_array_to_string(&dev.FriendlyName),
                    crate::camera::identity::normalize_serial(&c_char_array_to_string(
                        &dev.CameraSN,
                    )),
                )
            })
            .collect())
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = SvbonyCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

pub struct SvbonyCamera {
    handle: SvbonyHandle,
    info: CameraInfo,
    cooler_on: bool,
    /// The ROI actually reported by the SDK the last time it was set — the
    /// hardware may round the requested ROI to supported multiples, so a
    /// skipped (unchanged-config) frame must reuse this instead of
    /// re-deriving an unrounded value from `config`. Set by every config the
    /// exposure loop applies, and cleared whenever it forgets one.
    last_resolved_roi: Option<(c_int, c_int, c_int, c_int)>,
    exposure: ExposureLoop,
}

impl SvbonyCamera {
    pub fn open(index: usize) -> CameraResult<Self> {
        let devices = catch_ffi_panic("SVBony::enumerate", enumerate_devices)
            .map_err(CameraError::from)?
            .unwrap_or_default();

        if index >= devices.len() {
            return Err(CameraError::InvalidCameraIndex {
                index,
                count: devices.len(),
            });
        }

        let dev = &devices[index];
        let camera_id = dev.CameraID;

        let handle = catch_ffi_panic("SVBony::open", || SvbonyHandle::open(camera_id))
            .map_err(CameraError::from)?
            .map_err(CameraError::OpenFailed)?;

        let info = build_camera_info(dev, index as i32)
            .ok_or_else(|| CameraError::OpenFailed("Could not build camera info".to_string()))?;

        // Disable auto-exposure by default
        let _ = catch_ffi_panic("SVBony::set_auto_exp", || {
            let exposure = crate::ffi_safety::to_sdk_long("exposure", info.min_exposure_us)?;
            handle.set_control_value(SVB_EXPOSURE, exposure, false)
        });

        // Set max speed if available
        let _ = catch_ffi_panic("SVBony::set_speed", || {
            handle.set_control_value(SVB_FRAME_SPEED_MODE, 2, false) // 2 = High Speed
        });

        Ok(Self {
            handle,
            info,
            cooler_on: false,
            last_resolved_roi: None,
            exposure: ExposureLoop::new(),
        })
    }
}

impl Camera for SvbonyCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets {
            highest_dr: self.info.min_gain,
            hcg: self.info.min_gain + (self.info.max_gain - self.info.min_gain) / 3, // Approx
            unity: self.info.unity_gain,
            lowest_rn: self.info.max_gain,
            offset_highest_dr: 0,
            offset_hcg: 0,
            offset_unity: 0,
            offset_lowest_rn: 0,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        // Every read goes through `tolerate_unsupported`: a parameter this
        // model does not expose falls back, but a lost device propagates so the
        // fault detector can see it. See `camera::device_lost`.
        //
        // These used to discard the FFI-boundary result with `.ok()` as well,
        // which hid a panic caught at the boundary just as thoroughly as it hid
        // a dead handle.
        let current_exposure_us = tolerate_unsupported(
            catch_ffi_panic("SVBony::get_exp", || {
                self.handle.get_control_value(SVB_EXPOSURE)
            })
            .map_err(CameraError::from)?,
            (0, false),
        )?
        .0 as u64;

        let temperature_c = if self.info.has_cooler {
            tolerate_unsupported(
                catch_ffi_panic("SVBony::get_temp", || {
                    self.handle.get_control_value(SVB_CURRENT_TEMPERATURE)
                })
                .map_err(CameraError::from)?,
                (0, false),
            )?
            .0 as f64
                / 10.0
        } else {
            0.0
        };

        let current_gain = tolerate_unsupported(
            catch_ffi_panic("SVBony::get_gain", || {
                self.handle.get_control_value(SVB_GAIN)
            })
            .map_err(CameraError::from)?,
            (0, false),
        )?
        .0 as i32;

        let cooler_power = if self.info.has_cooler {
            Some(
                tolerate_unsupported(
                    catch_ffi_panic("SVBony::get_cooler_power", || {
                        self.handle.get_control_value(SVB_COOLER_POWER)
                    })
                    .map_err(CameraError::from)?,
                    (0, false),
                )?
                .0 as f64,
            )
        } else {
            None
        };

        Ok(CameraStatus {
            temperature_c,
            cooler_power,
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
        let temp_raw = (temp_c * 10.0) as c_long;
        catch_ffi_panic("SVBony::set_temp", || {
            self.handle
                .set_control_value(SVB_TARGET_TEMPERATURE, temp_raw, false)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::CoolingFailed)?;

        self.cooler_on = true;
        let _ = catch_ffi_panic("SVBony::enable_cooler", || {
            self.handle.set_control_value(SVB_COOLER_ENABLE, 1, false)
        });
        Ok(())
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        let val = if enabled { 1 } else { 0 };
        catch_ffi_panic("SVBony::set_cooler", || {
            self.handle.set_control_value(SVB_COOLER_ENABLE, val, false)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::SdkError {
            code: -1,
            message: e,
        })?;

        self.cooler_on = enabled;
        Ok(())
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::ParameterNotSupported("dew_heater".to_string()))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<RawFrame> {
        let Self {
            handle,
            info,
            last_resolved_roi,
            exposure,
            ..
        } = self;
        let mut sdk = SvbonyExposure {
            handle,
            info,
            resolved_roi: last_resolved_roi,
        };
        exposure.capture(&mut sdk, config)
    }

    fn invalidate_config_cache(&mut self) {
        self.exposure.invalidate();
        self.last_resolved_roi = None;
    }

    fn cancel(&self) {
        self.exposure.cancel();
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        self.exposure.cancel_token()
    }

    fn close(&mut self) -> CameraResult<()> {
        let _ = catch_ffi_panic("SVBony::close", || self.handle.close());
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "SVBony"
    }
}

/// The SDK's image type for `config`, and its bytes per pixel.
fn image_type(config: &CaptureConfig, info: &CameraInfo) -> (c_int, usize) {
    let is_color = info.sensor_type == SensorType::Color;
    match config.format {
        ImageFormat::Raw8 if is_color => (SVB_IMG_RAW8, 1),
        ImageFormat::Raw8 => (SVB_IMG_Y8, 1),
        ImageFormat::Raw16 if is_color => (SVB_IMG_RAW16, 2),
        ImageFormat::Raw16 => (SVB_IMG_Y16, 2),
        ImageFormat::Rgb24 => (SVB_IMG_RGB24, 3),
    }
}

/// SVBony's calls for the shared [`ExposureLoop`]. Video capture serves both acquisitions,
/// and every way a single exposure ends stops it.
struct SvbonyExposure<'a> {
    handle: &'a SvbonyHandle,
    info: &'a CameraInfo,
    resolved_roi: &'a mut Option<(c_int, c_int, c_int, c_int)>,
}

impl SdkExposure for SvbonyExposure<'_> {
    fn info(&self) -> &CameraInfo {
        self.info
    }

    fn apply(&mut self, config: &CaptureConfig) -> CameraResult<()> {
        let sdk_error = |message: String| CameraError::SdkError { code: -1, message };
        catch_ffi_panic("SVBony::set_exposure", || {
            let exposure = crate::ffi_safety::to_sdk_long("exposure", config.exposure_us)?;
            self.handle.set_control_value(SVB_EXPOSURE, exposure, false)
        })
        .map_err(CameraError::from)?
        .map_err(CameraError::ExposureFailed)?;

        catch_ffi_panic("SVBony::set_gain", || {
            self.handle
                .set_control_value(SVB_GAIN, config.gain as c_long, false)
        })
        .map_err(CameraError::from)?
        .map_err(sdk_error)?;

        let (svb_image_type, _) = image_type(config, self.info);
        catch_ffi_panic("SVBony::set_image_type", || {
            self.handle.set_output_image_type(svb_image_type)
        })
        .map_err(CameraError::from)?
        .map_err(sdk_error)?;

        let bin = config.bin as c_int;
        let (x, y, w, h) = match config.roi {
            Some((rx, ry, rw, rh)) => (rx as c_int, ry as c_int, rw as c_int, rh as c_int),
            None => (
                0,
                0,
                (self.info.max_width / bin as u32) as c_int,
                (self.info.max_height / bin as u32) as c_int,
            ),
        };
        catch_ffi_panic("SVBony::set_roi", || {
            self.handle.set_roi_format(x, y, w, h, bin)
        })
        .map_err(CameraError::from)?
        .map_err(|e| sdk_error(format!("Failed to set ROI: {}", e)))?;

        // Re-read actual ROI from SDK, as it might adjust to multiples
        let mut resolved = (x, y, w, h);
        if let Ok(Ok((rx, ry, rw, rh, _rbin))) =
            catch_ffi_panic("SVBony::get_roi", || self.handle.get_roi_format())
        {
            resolved = (rx, ry, rw, rh);
        }
        *self.resolved_roi = Some(resolved);
        Ok(())
    }

    fn start(&mut self, _acquisition: Acquisition) -> CameraResult<()> {
        catch_ffi_panic("SVBony::start_capture", || self.handle.start_video_capture())
            .map_err(CameraError::from)?
            .map_err(CameraError::ExposureFailed)
    }

    fn abort(&mut self, _acquisition: Acquisition) {
        self.stop();
    }

    fn frame_len(&mut self, config: &CaptureConfig) -> CameraResult<usize> {
        let (w, h) = self.frame_size();
        let (_, bytes_per_pixel) = image_type(config, self.info);
        Ok((w as usize) * (h as usize) * bytes_per_pixel)
    }

    fn poll(&mut self, progress: &Progress, buffer: &mut [u8]) -> Poll {
        // A short wait per call, so a cancel is seen between them.
        let budget_ms = progress.budget.as_millis().min(i32::MAX as u128) as c_int;
        let error = match catch_ffi_panic("SVBony::get_video_data", || {
            self.handle.get_video_data(buffer, 500.min(budget_ms))
        }) {
            Ok(Ok(())) => {
                let (w, h) = self.frame_size();
                return Poll::Ready {
                    width: w as u32,
                    height: h as u32,
                };
            }
            // The SDK's own timeout: retry until the stall budget runs out.
            Ok(Err(e)) if e.contains("SVBony SDK error 11") => return Poll::Pending,
            Ok(Err(e)) => CameraError::ExposureFailed(e),
            Err(e) => CameraError::ExposureFailed(e.to_string()),
        };
        self.stop();
        Poll::Failed {
            error,
            stream_ended: progress.acquisition == Acquisition::Stream,
        }
    }

    fn finish_single(&mut self) -> CameraResult<()> {
        self.stop();
        Ok(())
    }
}

impl SvbonyExposure<'_> {
    /// The frame's size as the SDK rounded it when the config was applied.
    fn frame_size(&self) -> (c_int, c_int) {
        let (_, _, w, h) = self
            .resolved_roi
            .expect("set by every config the exposure loop applies");
        (w, h)
    }

    fn stop(&self) {
        let _ = catch_ffi_panic("SVBony::stop_capture", || self.handle.stop_video_capture());
    }
}

fn c_char_array_to_string(arr: &[c_char]) -> String {
    let len = arr.iter().position(|&c| c == 0).unwrap_or(arr.len());
    let bytes: Vec<u8> = arr[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn build_camera_info(dev: &SVB_CAMERA_INFO, index: i32) -> Option<CameraInfo> {
    let prop = get_camera_property(dev.CameraID)?;
    let prop_ex = get_camera_property_ex(dev.CameraID);

    let name = c_char_array_to_string(&dev.FriendlyName);

    let is_color = prop.IsColorCam != 0;
    let bayer_pattern = if is_color {
        parse_fourcc_bayer(prop.BayerPattern)
    } else {
        None
    };

    let sensor_type = if is_color {
        SensorType::Color
    } else {
        SensorType::Mono
    };

    let mut supported_bins = Vec::new();
    for &bin in &prop.SupportedBins {
        if bin == 0 {
            break;
        }
        supported_bins.push(bin as u8);
    }
    if supported_bins.is_empty() {
        supported_bins.push(1);
    }

    let mut supported_formats = Vec::new();
    for &fmt in &prop.SupportedVideoFormat {
        if fmt == SVB_IMG_END {
            break;
        }
        match fmt {
            SVB_IMG_RAW8 | SVB_IMG_Y8 => supported_formats.push(ImageFormat::Raw8),
            SVB_IMG_RAW10 | SVB_IMG_RAW12 | SVB_IMG_RAW14 | SVB_IMG_RAW16 | SVB_IMG_Y10
            | SVB_IMG_Y12 | SVB_IMG_Y14 | SVB_IMG_Y16 => {
                if !supported_formats.contains(&ImageFormat::Raw16) {
                    supported_formats.push(ImageFormat::Raw16);
                }
            }
            SVB_IMG_RGB24 => supported_formats.push(ImageFormat::Rgb24),
            _ => {}
        }
    }

    let has_cooler = prop_ex
        .map(|ex| ex.bSupportControlTemp != 0)
        .unwrap_or(false);

    // Let's get control capabilities by opening a temporary handle if needed,
    // but the API allows `SVBGetControlCaps` on opened cameras only.
    // We'll use safe defaults for gain and exposure, then the UI updates them
    // when the camera is opened. SVBony typically has gain 0-500 or 0-1000.

    Some(CameraInfo {
        name,
        id: index,
        max_width: prop.MaxWidth as u32,
        max_height: prop.MaxHeight as u32,
        pixel_size_x_um: 0.0, // SVBGetSensorPixelSize needs open camera, leave 0
        pixel_size_y_um: 0.0,
        sensor_type,
        bayer_pattern,
        has_cooler,
        min_temp_c: if has_cooler { Some(-40.0) } else { None },
        max_temp_c: if has_cooler { Some(20.0) } else { None },
        has_shutter: false,
        is_usb3: true,
        bit_depth: prop.MaxBitDepth as u8,
        supported_bins,
        supported_formats,
        min_exposure_us: 10,
        max_exposure_us: 1_800_000_000,
        min_gain: 0,
        max_gain: 1000, // Safe default upper limit
        unity_gain: 100,
        hcg_gain: 100,
        sensor_modes: Vec::new(),
        has_dew_heater: false,
        serial: crate::camera::identity::normalize_serial(&c_char_array_to_string(&dev.CameraSN)),
    })
}
