use super::ffi_types::POAImgFormat;
use super::shim::Camera as POACamera;

pub struct ROI {
    pub start_x: u32,
    pub start_y: u32,
    pub width: u32,
    pub height: u32,
}
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

use super::super::error::{CameraError, CameraResult};
use super::super::types::{CameraInfo, CaptureConfig, ImageFormat, SensorType};
use super::sensor_mode;
use crate::ffi_safety::catch_ffi_panic;
use crate::{CfaPattern, Frame, PixelFormat};

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
            }
        }
        None => warn!(?desired, modes = ?info.sensor_modes, "Desired sensor mode not found on camera"),
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
                Ok(Err(e)) => warn!(error = ?e, target_temp_c = temp, "Failed to set target temperature"),
                Err(e) => warn!(error = %e, "Panic setting target temperature"),
            }
        }
    }
    let result = catch_ffi_panic("PlayerOne::set_cooler", || {
        camera.set_cooler(config.cooler_enabled)
    });
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => warn!(error = ?e, enabled = config.cooler_enabled, "Failed to set cooler state"),
        Err(e) => warn!(error = %e, "Panic setting cooler state"),
    }
}

pub fn get_capture_dimensions(info: &CameraInfo, config: &CaptureConfig) -> (u32, u32) {
    if let Some((_, _, w, h)) = config.roi {
        (w, h)
    } else {
        (
            info.max_width / config.bin as u32,
            info.max_height / config.bin as u32,
        )
    }
}

pub fn buffer_to_frame(
    info: &CameraInfo,
    buffer: &[u8],
    width: u32,
    height: u32,
    config: &CaptureConfig,
) -> CameraResult<Frame> {
    let (pixel_format, channels) = match config.format {
        ImageFormat::Raw8 => {
            if info.sensor_type == SensorType::Color {
                (PixelFormat::Bayer8, 1)
            } else {
                (PixelFormat::Rgb8, 1)
            }
        }
        ImageFormat::Raw16 => {
            if info.sensor_type == SensorType::Color {
                (PixelFormat::Bayer16, 1)
            } else {
                (PixelFormat::Rgb16, 1)
            }
        }
        ImageFormat::Rgb24 => (PixelFormat::Rgb8, 3),
    };

    if info.sensor_type == SensorType::Color && channels == 1 {
        let pattern = info.bayer_pattern.unwrap_or(CfaPattern::Rggb);
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

pub fn run_capture(
    camera: &mut POACamera,
    info: &CameraInfo,
    config: &CaptureConfig,
    cancel_flag: &AtomicBool,
) -> CameraResult<Frame> {
    // Reset cancel flag
    cancel_flag.store(false, Ordering::SeqCst);

    // Create buffer
    let mut buffer = catch_ffi_panic("PlayerOne::create_image_buffer", || {
        camera.create_image_buffer()
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::ImageReadFailed(e))?;

    // Calculate timeout
    let exposure_duration = Duration::from_micros(config.exposure_us);
    let total_timeout = config.timeout + exposure_duration;
    let start = Instant::now();

    // Start exposure
    catch_ffi_panic("PlayerOne::start_exposure", || camera.start_exposure())
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::ExposureFailed(format!("{:?}", e)))?;

    // Wait for image to be ready
    loop {
        if cancel_flag.load(Ordering::SeqCst) {
            let _ = catch_ffi_panic("PlayerOne::stop_exposure", || camera.stop_exposure());
            return Err(CameraError::Cancelled);
        }

        if start.elapsed() > total_timeout {
            let _ = catch_ffi_panic("PlayerOne::stop_exposure", || camera.stop_exposure());
            return Err(CameraError::ExposureTimeout(total_timeout));
        }

        let ready_result = catch_ffi_panic("PlayerOne::is_image_ready", || camera.is_image_ready())
            .map_err(CameraError::from)?;

        match ready_result {
            Ok(true) => break,
            Ok(false) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = catch_ffi_panic("PlayerOne::stop_exposure", || camera.stop_exposure());
                return Err(CameraError::ExposureFailed(format!("{:?}", e)));
            }
        }
    }

    // Get image data
    catch_ffi_panic("PlayerOne::get_image_data", || {
        camera.get_image_data(&mut buffer, Some(500))
    })
    .map_err(CameraError::from)?
    .map_err(|e| CameraError::ImageReadFailed(format!("{:?}", e)))?;

    // Stop exposure
    catch_ffi_panic("PlayerOne::stop_exposure", || camera.stop_exposure())
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::ExposureFailed(format!("{:?}", e)))?;

    // Convert to Frame
    let (width, height) = get_capture_dimensions(info, config);
    buffer_to_frame(info, &buffer, width, height, config)
}
