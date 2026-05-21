#![allow(non_snake_case)]
use std::os::raw::{c_char, c_float, c_int, c_long, c_uchar};
use std::sync::OnceLock;
use tracing::{debug, warn};

use dlopen2::wrapper::{Container, WrapperApi};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct SvbonyApi {
    SVBGetNumOfConnectedCameras: unsafe extern "C" fn() -> c_int,
    SVBGetCameraInfo: unsafe extern "C" fn(pSVBCameraInfo: *mut SVB_CAMERA_INFO, iCameraIndex: c_int) -> c_int,
    SVBGetCameraProperty: unsafe extern "C" fn(iCameraID: c_int, pCameraProperty: *mut SVB_CAMERA_PROPERTY) -> c_int,
    SVBGetCameraPropertyEx: unsafe extern "C" fn(iCameraID: c_int, pCameraPropertyEx: *mut SVB_CAMERA_PROPERTY_EX) -> c_int,
    SVBOpenCamera: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBCloseCamera: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetNumOfControls: unsafe extern "C" fn(iCameraID: c_int, piNumberOfControls: *mut c_int) -> c_int,
    SVBGetControlCaps: unsafe extern "C" fn(iCameraID: c_int, iControlIndex: c_int, pControlCaps: *mut SVB_CONTROL_CAPS) -> c_int,
    SVBGetControlValue: unsafe extern "C" fn(iCameraID: c_int, ControlType: c_int, plValue: *mut c_long, pbAuto: *mut c_int) -> c_int,
    SVBSetControlValue: unsafe extern "C" fn(iCameraID: c_int, ControlType: c_int, lValue: c_long, bAuto: c_int) -> c_int,
    SVBGetOutputImageType: unsafe extern "C" fn(iCameraID: c_int, pImageType: *mut c_int) -> c_int,
    SVBSetOutputImageType: unsafe extern "C" fn(iCameraID: c_int, ImageType: c_int) -> c_int,
    SVBSetROIFormat: unsafe extern "C" fn(iCameraID: c_int, iStartX: c_int, iStartY: c_int, iWidth: c_int, iHeight: c_int, iBin: c_int) -> c_int,
    SVBSetROIFormatEx: unsafe extern "C" fn(iCameraID: c_int, iStartX: c_int, iStartY: c_int, iWidth: c_int, iHeight: c_int, iBin: c_int, iMode: c_int) -> c_int,
    SVBGetROIFormat: unsafe extern "C" fn(iCameraID: c_int, piStartX: *mut c_int, piStartY: *mut c_int, piWidth: *mut c_int, piHeight: *mut c_int, piBin: *mut c_int) -> c_int,
    SVBGetROIFormatEx: unsafe extern "C" fn(iCameraID: c_int, piStartX: *mut c_int, piStartY: *mut c_int, piWidth: *mut c_int, piHeight: *mut c_int, piBin: *mut c_int, piMode: *mut c_int) -> c_int,
    SVBGetDroppedFrames: unsafe extern "C" fn(iCameraID: c_int, piDropFrames: *mut c_int) -> c_int,
    SVBStartVideoCapture: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBStopVideoCapture: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetVideoData: unsafe extern "C" fn(iCameraID: c_int, pBuffer: *mut c_uchar, lBuffSize: c_long, iWaitms: c_int) -> c_int,
    SVBWhiteBalanceOnce: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetCameraFirmwareVersion: unsafe extern "C" fn(iCameraID: c_int, pCameraFirmwareVersion: *mut c_char) -> c_int,
    SVBGetSDKVersion: unsafe extern "C" fn() -> *const c_char,
    SVBGetCameraSupportMode: unsafe extern "C" fn(iCameraID: c_int, pSupportedMode: *mut SVB_SUPPORTED_MODE) -> c_int,
    SVBGetCameraMode: unsafe extern "C" fn(iCameraID: c_int, mode: *mut c_int) -> c_int,
    SVBSetCameraMode: unsafe extern "C" fn(iCameraID: c_int, mode: c_int) -> c_int,
    SVBSendSoftTrigger: unsafe extern "C" fn(iCameraID: c_int) -> c_int,
    SVBGetSerialNumber: unsafe extern "C" fn(iCameraID: c_int, pSN: *mut SVB_ID) -> c_int,
    SVBSetTriggerOutputIOConf: unsafe extern "C" fn(iCameraID: c_int, pin: c_int, bPinHigh: c_int, lDelay: c_long, lDuration: c_long) -> c_int,
    SVBGetTriggerOutputIOConf: unsafe extern "C" fn(iCameraID: c_int, pin: c_int, bPinHigh: *mut c_int, lDelay: *mut c_long, lDuration: *mut c_long) -> c_int,
    SVBPulseGuide: unsafe extern "C" fn(iCameraID: c_int, direction: c_int, duration: c_int) -> c_int,
    SVBGetSensorPixelSize: unsafe extern "C" fn(iCameraID: c_int, fPixelSize: *mut c_float) -> c_int,
    SVBCanPulseGuide: unsafe extern "C" fn(iCameraID: c_int, pCanPulseGuide: *mut c_int) -> c_int,
    SVBSetAutoSaveParam: unsafe extern "C" fn(iCameraID: c_int, enable: c_int) -> c_int,
    SVBIsCameraNeedToUpgrade: unsafe extern "C" fn(iCameraID: c_int, pIsNeedToUpgrade: *mut c_int, pNeedToUpgradeMinVersion: *mut c_char) -> c_int,
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
            let lib_names = if cfg!(target_os = "windows") {
                vec!["SVBony.dll"]
            } else if cfg!(target_os = "macos") {
                vec!["libSVBony.dylib", "libSVBCameraSDK.dylib"]
            } else {
                vec!["libSVBony.so", "libSVBCameraSDK.so"]
            };

            for name in lib_names {
                match unsafe { Container::load(name) } {
                    Ok(container) => {
                        debug!("Successfully loaded SVBony SDK from {}", name);
                        return Some(SvbonySdk { api: container });
                    }
                    Err(e) => {
                        debug!("Failed to load SVBony SDK {}: {}", name, e);
                    }
                }
            }

            warn!("SVBONY SDK not found. SVBony provider will be disabled.");
            None
        })
        .as_ref()
    }
}

pub fn check_error(code: c_int, context: &str) -> Result<(), String> {
    if code == SVB_SUCCESS {
        Ok(())
    } else {
        Err(format!("SVBony SDK error {} in {}", code, context))
    }
}
