use dlopen2::wrapper::{Container, WrapperApi};
use std::os::raw::{c_char, c_int, c_long, c_uchar};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct ZwoSdkApi {
    ASIGetNumOfConnectedCameras: unsafe extern "C" fn() -> c_int,
    ASIGetCameraProperty: unsafe extern "C" fn(
        pASICameraInfo: *mut ASI_CAMERA_INFO,
        iCameraIndex: c_int,
    ) -> ASI_ERROR_CODE,
    ASIOpenCamera: unsafe extern "C" fn(iCameraID: c_int) -> ASI_ERROR_CODE,
    ASIInitCamera: unsafe extern "C" fn(iCameraID: c_int) -> ASI_ERROR_CODE,
    ASICloseCamera: unsafe extern "C" fn(iCameraID: c_int) -> ASI_ERROR_CODE,
    ASIGetNumOfControls:
        unsafe extern "C" fn(iCameraID: c_int, piNumberOfControls: *mut c_int) -> ASI_ERROR_CODE,
    ASIGetControlCaps: unsafe extern "C" fn(
        iCameraID: c_int,
        iControlIndex: c_int,
        pControlCaps: *mut ASI_CONTROL_CAPS,
    ) -> ASI_ERROR_CODE,
    ASIGetControlValue: unsafe extern "C" fn(
        iCameraID: c_int,
        ControlType: c_int,
        plValue: *mut c_long,
        pbAuto: *mut ASI_BOOL,
    ) -> ASI_ERROR_CODE,
    ASISetControlValue: unsafe extern "C" fn(
        iCameraID: c_int,
        ControlType: c_int,
        lValue: c_long,
        bAuto: ASI_BOOL,
    ) -> ASI_ERROR_CODE,
    ASISetROIFormat: unsafe extern "C" fn(
        iCameraID: c_int,
        iWidth: c_int,
        iHeight: c_int,
        iBin: c_int,
        Img_type: ASI_IMG_TYPE,
    ) -> ASI_ERROR_CODE,
    ASIGetROIFormat: unsafe extern "C" fn(
        iCameraID: c_int,
        piWidth: *mut c_int,
        piHeight: *mut c_int,
        piBin: *mut c_int,
        pImg_type: *mut ASI_IMG_TYPE,
    ) -> ASI_ERROR_CODE,
    ASISetStartPos:
        unsafe extern "C" fn(iCameraID: c_int, iStartX: c_int, iStartY: c_int) -> ASI_ERROR_CODE,
    ASIGetStartPos: unsafe extern "C" fn(
        iCameraID: c_int,
        piStartX: *mut c_int,
        piStartY: *mut c_int,
    ) -> ASI_ERROR_CODE,
    ASIStartExposure: unsafe extern "C" fn(iCameraID: c_int, bIsDark: ASI_BOOL) -> ASI_ERROR_CODE,
    ASIStopExposure: unsafe extern "C" fn(iCameraID: c_int) -> ASI_ERROR_CODE,
    ASIGetExpStatus: unsafe extern "C" fn(
        iCameraID: c_int,
        pExpStatus: *mut ASI_EXPOSURE_STATUS,
    ) -> ASI_ERROR_CODE,
    ASIGetDataAfterExp: unsafe extern "C" fn(
        iCameraID: c_int,
        pBuffer: *mut c_uchar,
        lDataSize: c_long,
    ) -> ASI_ERROR_CODE,
    ASIGetID: unsafe extern "C" fn(iCameraID: c_int, pID: *mut ASI_ID) -> ASI_ERROR_CODE,
}

pub struct ZwoSdk {
    pub api: Container<ZwoSdkApi>,
}

static SDK: OnceLock<Option<ZwoSdk>> = OnceLock::new();

impl ZwoSdk {
    pub fn try_load() -> Option<&'static Self> {
        SDK.get_or_init(|| {
            let lib_name = if cfg!(windows) {
                "ASICamera2.dll"
            } else if cfg!(target_os = "macos") {
                "libASICamera2.dylib"
            } else {
                "libASICamera2.so"
            };

            match unsafe { Container::load(lib_name) } {
                Ok(api) => {
                    info!("ZWO SDK ({}) loaded successfully.", lib_name);
                    Some(ZwoSdk { api })
                }
                Err(e) => {
                    info!(
                        "ZWO SDK ({}) not found or failed to load: {}. ZWO cameras disabled.",
                        lib_name, e
                    );
                    None
                }
            }
        })
        .as_ref()
    }
}
