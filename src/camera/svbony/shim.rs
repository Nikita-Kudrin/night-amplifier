use std::ffi::CStr;
use std::os::raw::{c_char, c_int, c_long, c_uchar};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, warn};

use super::ffi_types::*;
use super::sdk::{check_error, SvbonySdk};
use crate::CfaPattern;

pub struct SvbonyHandle {
    camera_id: c_int,
    video_mode_active: AtomicBool,
}

// SAFETY: SDK is thread-safe per camera handle.
unsafe impl Send for SvbonyHandle {}
unsafe impl Sync for SvbonyHandle {}

impl SvbonyHandle {
    pub fn open(camera_id: c_int) -> Result<Self, String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        
        let hr = unsafe { sdk.api.SVBOpenCamera(camera_id) };
        check_error(hr, "SVBOpenCamera")?;

        Ok(Self {
            camera_id,
            video_mode_active: AtomicBool::new(false),
        })
    }

    pub fn close(&self) {
        let sdk = match SvbonySdk::try_load() {
            Some(s) => s,
            None => return,
        };

        if self.video_mode_active.load(Ordering::SeqCst) {
            let _ = unsafe { sdk.api.SVBStopVideoCapture(self.camera_id) };
            self.video_mode_active.store(false, Ordering::SeqCst);
        }

        unsafe { sdk.api.SVBCloseCamera(self.camera_id) };
    }

    pub fn set_control_value(&self, control_type: c_int, value: c_long, auto: bool) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let auto_val = if auto { SVB_TRUE } else { SVB_FALSE };
        let hr = unsafe { sdk.api.SVBSetControlValue(self.camera_id, control_type, value, auto_val) };
        check_error(hr, &format!("SVBSetControlValue({})", control_type))
    }

    pub fn get_control_value(&self, control_type: c_int) -> Result<(c_long, bool), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let mut value: c_long = 0;
        let mut auto: c_int = 0;
        let hr = unsafe { sdk.api.SVBGetControlValue(self.camera_id, control_type, &mut value, &mut auto) };
        check_error(hr, &format!("SVBGetControlValue({})", control_type))?;
        Ok((value, auto == SVB_TRUE))
    }

    pub fn set_output_image_type(&self, image_type: c_int) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let hr = unsafe { sdk.api.SVBSetOutputImageType(self.camera_id, image_type) };
        check_error(hr, "SVBSetOutputImageType")
    }

    pub fn set_roi_format(&self, x: c_int, y: c_int, w: c_int, h: c_int, bin: c_int) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let hr = unsafe { sdk.api.SVBSetROIFormat(self.camera_id, x, y, w, h, bin) };
        check_error(hr, "SVBSetROIFormat")
    }

    pub fn get_roi_format(&self) -> Result<(c_int, c_int, c_int, c_int, c_int), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let mut x = 0;
        let mut y = 0;
        let mut w = 0;
        let mut h = 0;
        let mut bin = 0;
        let hr = unsafe { sdk.api.SVBGetROIFormat(self.camera_id, &mut x, &mut y, &mut w, &mut h, &mut bin) };
        check_error(hr, "SVBGetROIFormat")?;
        Ok((x, y, w, h, bin))
    }

    pub fn start_video_capture(&self) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let hr = unsafe { sdk.api.SVBStartVideoCapture(self.camera_id) };
        check_error(hr, "SVBStartVideoCapture")?;
        self.video_mode_active.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn stop_video_capture(&self) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let hr = unsafe { sdk.api.SVBStopVideoCapture(self.camera_id) };
        self.video_mode_active.store(false, Ordering::SeqCst);
        check_error(hr, "SVBStopVideoCapture")
    }

    pub fn get_video_data(&self, buffer: &mut [u8], wait_ms: c_int) -> Result<(), String> {
        let sdk = SvbonySdk::try_load().ok_or("SVBONY SDK not loaded")?;
        let hr = unsafe { sdk.api.SVBGetVideoData(self.camera_id, buffer.as_mut_ptr(), buffer.len() as c_long, wait_ms) };
        check_error(hr, "SVBGetVideoData")
    }
}

impl Drop for SvbonyHandle {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn enumerate_devices() -> Option<Vec<SVB_CAMERA_INFO>> {
    let sdk = SvbonySdk::try_load()?;
    let count = unsafe { sdk.api.SVBGetNumOfConnectedCameras() };
    if count <= 0 {
        return Some(Vec::new());
    }

    let mut cameras = Vec::with_capacity(count as usize);
    for i in 0..count {
        let mut info = std::mem::MaybeUninit::<SVB_CAMERA_INFO>::zeroed();
        let hr = unsafe { sdk.api.SVBGetCameraInfo(info.as_mut_ptr(), i) };
        if hr == SVB_SUCCESS {
            cameras.push(unsafe { info.assume_init() });
        }
    }
    Some(cameras)
}

pub fn get_camera_property(camera_id: c_int) -> Option<SVB_CAMERA_PROPERTY> {
    let sdk = SvbonySdk::try_load()?;
    let mut prop = std::mem::MaybeUninit::<SVB_CAMERA_PROPERTY>::zeroed();
    let hr = unsafe { sdk.api.SVBGetCameraProperty(camera_id, prop.as_mut_ptr()) };
    if hr == SVB_SUCCESS {
        Some(unsafe { prop.assume_init() })
    } else {
        None
    }
}

pub fn get_camera_property_ex(camera_id: c_int) -> Option<SVB_CAMERA_PROPERTY_EX> {
    let sdk = SvbonySdk::try_load()?;
    let mut prop = std::mem::MaybeUninit::<SVB_CAMERA_PROPERTY_EX>::zeroed();
    let hr = unsafe { sdk.api.SVBGetCameraPropertyEx(camera_id, prop.as_mut_ptr()) };
    if hr == SVB_SUCCESS {
        Some(unsafe { prop.assume_init() })
    } else {
        None
    }
}

pub fn parse_fourcc_bayer(bayer_pattern: c_int) -> Option<CfaPattern> {
    match bayer_pattern {
        SVB_BAYER_RG => Some(CfaPattern::Rggb),
        SVB_BAYER_BG => Some(CfaPattern::Bggr),
        SVB_BAYER_GR => Some(CfaPattern::Grbg),
        SVB_BAYER_GB => Some(CfaPattern::Gbrg),
        _ => None,
    }
}
