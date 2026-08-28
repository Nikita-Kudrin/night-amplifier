use std::collections::BTreeMap;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_long, c_uchar};
use tracing::warn;

use super::ffi_types::*;
use super::sdk::ZwoSdk;
use crate::camera::device_lost;
use crate::camera::DeviceLease;

/// Provider key for [`DeviceLease`] slots. `ASICloseCamera` takes a device
/// index, so a stale close would land on whoever holds that index now.
pub(super) const PROVIDER: &str = "ZWO";

/// Symbolic name for an `ASI_ERROR_CODE`, so a log line says
/// `ASI_ERROR_CAMERA_REMOVED` rather than `5`.
fn asi_symbol(err: ASI_ERROR_CODE) -> &'static str {
    match err {
        ASI_ERROR_CODE_ASI_ERROR_INVALID_INDEX => "ASI_ERROR_INVALID_INDEX",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_ID => "ASI_ERROR_INVALID_ID",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_CONTROL_TYPE => "ASI_ERROR_INVALID_CONTROL_TYPE",
        ASI_ERROR_CODE_ASI_ERROR_CAMERA_CLOSED => "ASI_ERROR_CAMERA_CLOSED",
        ASI_ERROR_CODE_ASI_ERROR_CAMERA_REMOVED => "ASI_ERROR_CAMERA_REMOVED",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_PATH => "ASI_ERROR_INVALID_PATH",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_FILEFORMAT => "ASI_ERROR_INVALID_FILEFORMAT",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_SIZE => "ASI_ERROR_INVALID_SIZE",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_IMGTYPE => "ASI_ERROR_INVALID_IMGTYPE",
        ASI_ERROR_CODE_ASI_ERROR_OUTOF_BOUNDARY => "ASI_ERROR_OUTOF_BOUNDARY",
        ASI_ERROR_CODE_ASI_ERROR_TIMEOUT => "ASI_ERROR_TIMEOUT",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_SEQUENCE => "ASI_ERROR_INVALID_SEQUENCE",
        ASI_ERROR_CODE_ASI_ERROR_BUFFER_TOO_SMALL => "ASI_ERROR_BUFFER_TOO_SMALL",
        ASI_ERROR_CODE_ASI_ERROR_VIDEO_MODE_ACTIVE => "ASI_ERROR_VIDEO_MODE_ACTIVE",
        ASI_ERROR_CODE_ASI_ERROR_EXPOSURE_IN_PROGRESS => "ASI_ERROR_EXPOSURE_IN_PROGRESS",
        ASI_ERROR_CODE_ASI_ERROR_GENERAL_ERROR => "ASI_ERROR_GENERAL_ERROR",
        ASI_ERROR_CODE_ASI_ERROR_INVALID_MODE => "ASI_ERROR_INVALID_MODE",
        ASI_ERROR_CODE_ASI_ERROR_GPS_NOT_SUPPORTED => "ASI_ERROR_GPS_NOT_SUPPORTED",
        ASI_ERROR_CODE_ASI_ERROR_GPS_VER_ERR => "ASI_ERROR_GPS_VER_ERR",
        _ => "ASI_ERROR_UNKNOWN",
    }
}

/// Render a ZWO failure symbolically, tagging the codes that mean the device
/// is gone. Before this the message was a bare integer, which is why device
/// loss on ZWO was never classified at all.
fn asi_err(op: &str, err: ASI_ERROR_CODE) -> String {
    let detail = format!("{} failed: {} ({})", op, asi_symbol(err), err);
    if matches!(
        err,
        ASI_ERROR_CODE_ASI_ERROR_INVALID_ID
            | ASI_ERROR_CODE_ASI_ERROR_CAMERA_CLOSED
            | ASI_ERROR_CODE_ASI_ERROR_CAMERA_REMOVED
    ) {
        return device_lost::mark(detail);
    }
    detail
}

#[derive(Debug)]
pub struct CameraInfoASI {
    pub name: String,
    pub camera_id: i32,
    pub max_height: i32,
    pub max_width: i32,
    pub is_color_cam: bool,
    pub bayer_pattern: ASI_BAYER_PATTERN,
    pub supported_bins: Vec<i32>,
    pub supported_video_format: Vec<ASI_IMG_TYPE>,
    pub pixel_size: f64,
    pub mechanical_shutter: bool,
    pub is_cooler_cam: bool,
    pub is_usb3_host: bool,
    pub is_usb3_camera: bool,
    pub elec_per_adu: f32,
    pub bit_depth: i32,
    pub is_trigger_cam: bool,
}

pub struct Camera {
    camera_id: c_int,
    /// Proof this handle still owns `camera_id`. Gates every `ASICloseCamera`.
    lease: DeviceLease,
}

impl Camera {
    pub fn open(camera_id: i32) -> Result<(Self, CameraInfoASI), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASIOpenCamera(camera_id) };
        if err != ASI_ERROR_CODE_ASI_SUCCESS {
            return Err(asi_err("ASIOpenCamera", err));
        }
        let err = unsafe { sdk.api.ASIInitCamera(camera_id) };
        if err != ASI_ERROR_CODE_ASI_SUCCESS {
            return Err(asi_err("ASIInitCamera", err));
        }

        let mut info: ASI_CAMERA_INFO = unsafe { std::mem::zeroed() };
        let err = unsafe { sdk.api.ASIGetCameraProperty(&mut info, camera_id) };
        if err != ASI_ERROR_CODE_ASI_SUCCESS {
            return Err(asi_err("ASIGetCameraProperty", err));
        }

        let name = unsafe { CStr::from_ptr(info.Name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        let supported_bins = info
            .SupportedBins
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b)
            .collect();
        let supported_video_format = info
            .SupportedVideoFormat
            .iter()
            .take_while(|&&f| f != ASI_IMG_TYPE_ASI_IMG_END)
            .copied()
            .collect();

        let info_asi = CameraInfoASI {
            name,
            camera_id: info.CameraID as i32,
            max_height: info.MaxHeight as i32,
            max_width: info.MaxWidth as i32,
            is_color_cam: info.IsColorCam == ASI_BOOL_ASI_TRUE,
            bayer_pattern: info.BayerPattern,
            supported_bins,
            supported_video_format,
            pixel_size: info.PixelSize as f64,
            mechanical_shutter: info.MechanicalShutter == ASI_BOOL_ASI_TRUE,
            is_cooler_cam: info.IsCoolerCam == ASI_BOOL_ASI_TRUE,
            is_usb3_host: info.IsUSB3Host == ASI_BOOL_ASI_TRUE,
            is_usb3_camera: info.IsUSB3Camera == ASI_BOOL_ASI_TRUE,
            elec_per_adu: info.ElecPerADU as f32,
            bit_depth: info.BitDepth as i32,
            is_trigger_cam: info.IsTriggerCam == ASI_BOOL_ASI_TRUE,
        };

        Ok((
            Self {
                camera_id,
                lease: DeviceLease::acquire(PROVIDER, camera_id),
            },
            info_asi,
        ))
    }

    pub fn close(&self) -> Result<(), String> {
        // Refused for a handle a later open has superseded, and for a second
        // close on the same handle (`Drop` follows an explicit `close()`).
        if !self.lease.begin_close() {
            return Ok(());
        }
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASICloseCamera(self.camera_id) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASICloseCamera", err))
        }
    }

    pub fn set_control_value(
        &self,
        control_type: ASI_CONTROL_TYPE,
        value: c_long,
        auto: ASI_BOOL,
    ) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe {
            sdk.api
                .ASISetControlValue(self.camera_id, control_type as c_int, value, auto)
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASISetControlValue", err))
        }
    }

    pub fn get_control_value(
        &self,
        control_type: ASI_CONTROL_TYPE,
    ) -> Result<(c_long, ASI_BOOL), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let mut value: c_long = 0;
        let mut auto: ASI_BOOL = ASI_BOOL_ASI_FALSE;
        let err = unsafe {
            sdk.api
                .ASIGetControlValue(self.camera_id, control_type as c_int, &mut value, &mut auto)
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok((value, auto))
        } else {
            Err(asi_err("ASIGetControlValue", err))
        }
    }

    pub fn set_exposure(&self, us: i64) -> Result<(), String> {
        self.set_control_value(
            ASI_CONTROL_TYPE_ASI_EXPOSURE,
            us as c_long,
            ASI_BOOL_ASI_FALSE,
        )
    }

    pub fn set_gain_raw(&self, gain: i64) -> Result<(), String> {
        self.set_control_value(
            ASI_CONTROL_TYPE_ASI_GAIN,
            gain as c_long,
            ASI_BOOL_ASI_FALSE,
        )
    }

    pub fn set_temperature(&self, temp_c: f32) -> Result<(), String> {
        self.set_control_value(
            ASI_CONTROL_TYPE_ASI_TARGET_TEMP,
            temp_c as c_long,
            ASI_BOOL_ASI_FALSE,
        )
    }

    pub fn set_cooler(&self, enabled: bool) -> Result<(), String> {
        self.set_control_value(
            ASI_CONTROL_TYPE_ASI_COOLER_ON,
            if enabled { 1 } else { 0 },
            ASI_BOOL_ASI_FALSE,
        )
    }

    pub fn set_anti_dew_heater(&self, enabled: bool) -> Result<(), String> {
        self.set_control_value(
            ASI_CONTROL_TYPE_ASI_ANTI_DEW_HEATER,
            if enabled { 1 } else { 0 },
            ASI_BOOL_ASI_FALSE,
        )
    }

    pub fn get_temperature(&self) -> Result<f32, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_TEMPERATURE)
            .map(|(v, _)| v as f32 / 10.0)
    }

    pub fn get_gain_raw(&self) -> Result<i64, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_GAIN)
            .map(|(v, _)| v)
    }

    pub fn get_offset_raw(&self) -> Result<i64, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_OFFSET)
            .map(|(v, _)| v)
    }

    pub fn get_exposure(&self) -> Result<i64, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_EXPOSURE)
            .map(|(v, _)| v)
    }

    pub fn get_cooler(&self) -> Result<bool, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_COOLER_ON)
            .map(|(v, _)| v != 0)
    }

    pub fn get_anti_dew_heater(&self) -> Result<bool, String> {
        self.get_control_value(ASI_CONTROL_TYPE_ASI_ANTI_DEW_HEATER)
            .map(|(v, _)| v != 0)
    }

    pub fn is_control_supported(&self, control_type: ASI_CONTROL_TYPE) -> bool {
        self.get_control_value(control_type).is_ok()
    }

    pub fn set_image_fmt(&self, format: ASI_IMG_TYPE) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let mut width = 0;
        let mut height = 0;
        let mut bin = 1;
        let mut old_format = ASI_IMG_TYPE_ASI_IMG_RAW8;
        unsafe {
            sdk.api.ASIGetROIFormat(
                self.camera_id,
                &mut width,
                &mut height,
                &mut bin,
                &mut old_format,
            )
        };
        let err = unsafe {
            sdk.api
                .ASISetROIFormat(self.camera_id, width, height, bin, format)
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASISetROIFormat", err))
        }
    }

    pub fn set_roi(&self, x: i32, y: i32, width: i32, height: i32, bin: i32) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let mut old_width = 0;
        let mut old_height = 0;
        let mut old_bin = 1;
        let mut format = ASI_IMG_TYPE_ASI_IMG_RAW8;
        unsafe {
            sdk.api.ASIGetROIFormat(
                self.camera_id,
                &mut old_width,
                &mut old_height,
                &mut old_bin,
                &mut format,
            )
        };

        let err = unsafe { sdk.api.ASISetStartPos(self.camera_id, x, y) };
        if err != ASI_ERROR_CODE_ASI_SUCCESS {
            return Err(asi_err("ASISetStartPos", err));
        }

        let err = unsafe {
            sdk.api
                .ASISetROIFormat(self.camera_id, width, height, bin, format)
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASISetROIFormat", err))
        }
    }

    pub fn start_capture(&self) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASIStartExposure(self.camera_id, ASI_BOOL_ASI_FALSE) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIStartExposure", err))
        }
    }

    pub fn stop_capture(&self) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASIStopExposure(self.camera_id) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIStopExposure", err))
        }
    }

    pub fn is_image_ready(&self) -> Result<bool, String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let mut status = ASI_EXPOSURE_STATUS_ASI_EXP_IDLE;
        let err = unsafe { sdk.api.ASIGetExpStatus(self.camera_id, &mut status) };
        if err != ASI_ERROR_CODE_ASI_SUCCESS {
            return Err(asi_err("ASIGetExpStatus", err));
        }
        if status == ASI_EXPOSURE_STATUS_ASI_EXP_SUCCESS {
            Ok(true)
        } else if status == ASI_EXPOSURE_STATUS_ASI_EXP_FAILED {
            Err("Exposure failed".to_string())
        } else {
            Ok(false)
        }
    }

    pub fn get_image_data(&self, buffer: &mut [u8]) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe {
            sdk.api.ASIGetDataAfterExp(
                self.camera_id,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as c_long,
            )
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIGetDataAfterExp", err))
        }
    }

    pub fn start_video_capture(&self) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASIStartVideoCapture(self.camera_id) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIStartVideoCapture", err))
        }
    }

    pub fn stop_video_capture(&self) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe { sdk.api.ASIStopVideoCapture(self.camera_id) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIStopVideoCapture", err))
        }
    }

    pub fn get_video_data(&self, buffer: &mut [u8], wait_ms: i32) -> Result<(), String> {
        let sdk = ZwoSdk::try_load().ok_or("ZWO SDK not loaded")?;
        let err = unsafe {
            sdk.api.ASIGetVideoData(
                self.camera_id,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as c_long,
                wait_ms,
            )
        };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            Ok(())
        } else {
            Err(asi_err("ASIGetVideoData", err))
        }
    }
}

impl Drop for Camera {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub fn get_camera_ids() -> Option<BTreeMap<i32, String>> {
    let sdk = ZwoSdk::try_load()?;
    let count = unsafe { sdk.api.ASIGetNumOfConnectedCameras() };
    if count <= 0 {
        return Some(BTreeMap::new());
    }

    let mut map = BTreeMap::new();
    for i in 0..count {
        let mut info: ASI_CAMERA_INFO = unsafe { std::mem::zeroed() };
        let err = unsafe { sdk.api.ASIGetCameraProperty(&mut info, i) };
        if err == ASI_ERROR_CODE_ASI_SUCCESS {
            let name = unsafe { CStr::from_ptr(info.Name.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            map.insert(info.CameraID as i32, name);
        }
    }
    Some(map)
}

pub fn num_cameras() -> i32 {
    if let Some(sdk) = ZwoSdk::try_load() {
        unsafe { sdk.api.ASIGetNumOfConnectedCameras() }
    } else {
        0
    }
}
