#![allow(non_snake_case)]
use std::os::raw::{c_char, c_float, c_int, c_long, c_uchar};
use std::sync::OnceLock;
use tracing::{info, warn};

use dlopen2::wrapper::{Container, WrapperApi};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct SvbonyApi {
    SVBGetNumOfConnectedCameras: unsafe extern "C" fn() -> c_int,
    SVBGetCameraInfo:
        unsafe extern "C" fn(pSVBCameraInfo: *mut SVB_CAMERA_INFO, iCameraIndex: c_int) -> c_int,
    SVBGetCameraProperty:
        unsafe extern "C" fn(iCameraID: c_int, pCameraProperty: *mut SVB_CAMERA_PROPERTY) -> c_int,
    SVBGetCameraPropertyEx: unsafe extern "C" fn(
        iCameraID: c_int,
        pCameraPropertyEx: *mut SVB_CAMERA_PROPERTY_EX,
    ) -> c_int,
    SVBOpenCamera: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBCloseCamera: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetNumOfControls:
        unsafe extern "C" fn(iCameraID: c_int, piNumberOfControls: *mut c_int) -> c_int,
    SVBGetControlCaps: unsafe extern "C" fn(
        iCameraID: c_int,
        iControlIndex: c_int,
        pControlCaps: *mut SVB_CONTROL_CAPS,
    ) -> c_int,
    SVBGetControlValue: unsafe extern "C" fn(
        iCameraID: c_int,
        ControlType: c_int,
        plValue: *mut c_long,
        pbAuto: *mut c_int,
    ) -> c_int,
    SVBSetControlValue: unsafe extern "C" fn(
        iCameraID: c_int,
        ControlType: c_int,
        lValue: c_long,
        bAuto: c_int,
    ) -> c_int,
    SVBGetOutputImageType: unsafe extern "C" fn(iCameraID: c_int, pImageType: *mut c_int) -> c_int,
    SVBSetOutputImageType: unsafe extern "C" fn(iCameraID: c_int, ImageType: c_int) -> c_int,
    SVBSetROIFormat: unsafe extern "C" fn(
        iCameraID: c_int,
        iStartX: c_int,
        iStartY: c_int,
        iWidth: c_int,
        iHeight: c_int,
        iBin: c_int,
    ) -> c_int,
    SVBSetROIFormatEx: unsafe extern "C" fn(
        iCameraID: c_int,
        iStartX: c_int,
        iStartY: c_int,
        iWidth: c_int,
        iHeight: c_int,
        iBin: c_int,
        iMode: c_int,
    ) -> c_int,
    SVBGetROIFormat: unsafe extern "C" fn(
        iCameraID: c_int,
        piStartX: *mut c_int,
        piStartY: *mut c_int,
        piWidth: *mut c_int,
        piHeight: *mut c_int,
        piBin: *mut c_int,
    ) -> c_int,
    SVBGetROIFormatEx: unsafe extern "C" fn(
        iCameraID: c_int,
        piStartX: *mut c_int,
        piStartY: *mut c_int,
        piWidth: *mut c_int,
        piHeight: *mut c_int,
        piBin: *mut c_int,
        piMode: *mut c_int,
    ) -> c_int,
    SVBGetDroppedFrames: unsafe extern "C" fn(iCameraID: c_int, piDropFrames: *mut c_int) -> c_int,
    SVBStartVideoCapture: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBStopVideoCapture: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetVideoData: unsafe extern "C" fn(
        iCameraID: c_int,
        pBuffer: *mut c_uchar,
        lBuffSize: c_long,
        iWaitms: c_int,
    ) -> c_int,
    SVBWhiteBalanceOnce: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetCameraFirmwareVersion:
        unsafe extern "C" fn(iCameraID: c_int, pCameraFirmwareVersion: *mut c_char) -> c_int,
    SVBGetSDKVersion: unsafe extern "C" fn() -> *const c_char,
    SVBGetCameraSupportMode:
        unsafe extern "C" fn(iCameraID: c_int, pSupportedMode: *mut SVB_SUPPORTED_MODE) -> c_int,
    SVBGetCameraMode: unsafe extern "C" fn(iCameraID: c_int, mode: *mut c_int) -> c_int,
    SVBSetCameraMode: unsafe extern "C" fn(iCameraID: c_int, mode: c_int) -> c_int,
    SVBSendSoftTrigger: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetSerialNumber: unsafe extern "C" fn(iCameraID: c_int, pSN: *mut SVB_ID) -> c_int,
    SVBSetTriggerOutputIOConf: unsafe extern "C" fn(
        iCameraID: c_int,
        pin: c_int,
        bPinHigh: c_int,
        lDelay: c_long,
        lDuration: c_long,
    ) -> c_int,
    SVBGetTriggerOutputIOConf: unsafe extern "C" fn(
        iCameraID: c_int,
        pin: c_int,
        bPinHigh: *mut c_int,
        lDelay: *mut c_long,
        lDuration: *mut c_long,
    ) -> c_int,
    SVBPulseGuide:
        unsafe extern "C" fn(iCameraID: c_int, direction: c_int, duration: c_int) -> c_int,
    SVBGetSensorPixelSize:
        unsafe extern "C" fn(iCameraID: c_int, fPixelSize: *mut c_float) -> c_int,
    SVBCanPulseGuide: unsafe extern "C" fn(iCameraID: c_int, pCanPulseGuide: *mut c_int) -> c_int,
    SVBSetAutoSaveParam: unsafe extern "C" fn(iCameraID: c_int, enable: c_int) -> c_int,
    SVBIsCameraNeedToUpgrade: unsafe extern "C" fn(
        iCameraID: c_int,
        pIsNeedToUpgrade: *mut c_int,
        pNeedToUpgradeMinVersion: *mut c_char,
    ) -> c_int,
    SVBRestoreDefaultParam: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
}

pub struct SvbonySdk {
    pub api: Container<SvbonyApi>,
}

// Ensure the Container can be shared safely across threads.
unsafe impl Send for SvbonySdk {}
unsafe impl Sync for SvbonySdk {}

static SDK: OnceLock<Option<SvbonySdk>> = OnceLock::new();

impl SvbonySdk {
    pub fn try_load() -> Option<&'static SvbonySdk> {
        SDK.get_or_init(|| {
            crate::camera::sdk_library::preload_libusb();
            let mut failures = Vec::new();
            for &name in library_candidates(std::env::consts::OS) {
                match unsafe { crate::camera::sdk_library::load_eagerly::<SvbonyApi>(name) } {
                    Ok(container) => {
                        info!("SVBony SDK ({}) loaded successfully.", name);
                        return Some(SvbonySdk { api: container });
                    }
                    Err(e) => failures.push((name, e)),
                }
            }

            if failures
                .iter()
                .all(|(_, e)| crate::camera::sdk_library::is_missing_file(e))
            {
                info!("SVBony SDK not installed. SVBony cameras disabled.");
            } else {
                let reasons: Vec<String> = failures
                    .iter()
                    .map(|(name, e)| format!("{name}: {e}"))
                    .collect();
                warn!(
                    "SVBony SDK found but failed to load ({}). SVBony cameras disabled.",
                    reasons.join("; ")
                );
            }
            None
        })
        .as_ref()
    }
}

/// SDK file names to try for `std::env::consts::OS`, the SDK's own name first. The Windows
/// SDK ships `SVBCameraSDK.dll`; `SVBony.dll` alone never loaded there.
fn library_candidates(os: &str) -> &'static [&'static str] {
    match os {
        "windows" => &["SVBCameraSDK.dll", "SVBony.dll"],
        "macos" => &["libSVBCameraSDK.dylib", "libSVBony.dylib"],
        _ => &["libSVBCameraSDK.so", "libSVBony.so"],
    }
}

#[cfg(test)]
mod tests {
    use super::library_candidates;

    #[test]
    fn every_platform_tries_the_sdk_file_name_first() {
        for (os, sdk_file) in [
            ("linux", "libSVBCameraSDK.so"),
            ("macos", "libSVBCameraSDK.dylib"),
            ("windows", "SVBCameraSDK.dll"),
        ] {
            assert_eq!(library_candidates(os)[0], sdk_file);
        }
    }
}

/// Symbolic name for an SVBony error code, so a log line names the fault
/// rather than numbering it.
fn svb_symbol(code: c_int) -> &'static str {
    match code {
        SVB_ERROR_INVALID_INDEX => "SVB_ERROR_INVALID_INDEX",
        SVB_ERROR_INVALID_ID => "SVB_ERROR_INVALID_ID",
        SVB_ERROR_INVALID_CONTROL_TYPE => "SVB_ERROR_INVALID_CONTROL_TYPE",
        SVB_ERROR_CAMERA_CLOSED => "SVB_ERROR_CAMERA_CLOSED",
        SVB_ERROR_CAMERA_REMOVED => "SVB_ERROR_CAMERA_REMOVED",
        SVB_ERROR_INVALID_PATH => "SVB_ERROR_INVALID_PATH",
        SVB_ERROR_INVALID_FILEFORMAT => "SVB_ERROR_INVALID_FILEFORMAT",
        SVB_ERROR_INVALID_SIZE => "SVB_ERROR_INVALID_SIZE",
        SVB_ERROR_INVALID_IMGTYPE => "SVB_ERROR_INVALID_IMGTYPE",
        SVB_ERROR_OUTOF_BOUNDARY => "SVB_ERROR_OUTOF_BOUNDARY",
        SVB_ERROR_TIMEOUT => "SVB_ERROR_TIMEOUT",
        SVB_ERROR_INVALID_SEQUENCE => "SVB_ERROR_INVALID_SEQUENCE",
        SVB_ERROR_BUFFER_TOO_SMALL => "SVB_ERROR_BUFFER_TOO_SMALL",
        SVB_ERROR_VIDEO_MODE_ACTIVE => "SVB_ERROR_VIDEO_MODE_ACTIVE",
        SVB_ERROR_EXPOSURE_IN_PROGRESS => "SVB_ERROR_EXPOSURE_IN_PROGRESS",
        SVB_ERROR_GENERAL_ERROR => "SVB_ERROR_GENERAL_ERROR",
        SVB_ERROR_INVALID_MODE => "SVB_ERROR_INVALID_MODE",
        SVB_ERROR_INVALID_DIRECTION => "SVB_ERROR_INVALID_DIRECTION",
        SVB_ERROR_UNKNOW_SENSOR_TYPE => "SVB_ERROR_UNKNOW_SENSOR_TYPE",
        _ => "SVB_ERROR_UNKNOWN",
    }
}

/// The single funnel every SVBony call passes through. Codes that mean the
/// device is gone are tagged for `CameraError::is_sdk_disconnected`.
pub fn check_error(code: c_int, context: &str) -> Result<(), String> {
    if code == SVB_SUCCESS {
        return Ok(());
    }
    let detail = format!("{} failed: {} ({})", context, svb_symbol(code), code);
    if matches!(
        code,
        SVB_ERROR_INVALID_ID | SVB_ERROR_CAMERA_CLOSED | SVB_ERROR_CAMERA_REMOVED
    ) {
        return Err(crate::camera::device_lost::mark(detail));
    }
    Err(detail)
}
