#![allow(non_snake_case)]
use std::os::raw::{c_char, c_int, c_long, c_uchar};
use dlopen2::wrapper::{Container, WrapperApi};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct PlayerOneSdkApi {
    POAGetCameraCount: unsafe extern "C" fn() -> c_int,
    POAGetCameraProperties: unsafe extern "C" fn(nIndex: c_int, pProp: *mut POACameraProperties) -> POAErrors,
    POAOpenCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POAInitCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POACloseCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POASetConfig: unsafe extern "C" fn(nCameraID: c_int, confID: POAConfig, confValue: POAConfigValue, isAuto: POABool) -> POAErrors,
    POAGetConfig: unsafe extern "C" fn(nCameraID: c_int, confID: POAConfig, pConfValue: *mut POAConfigValue, pIsAuto: *mut POABool) -> POAErrors,
    POASetImageStartPos: unsafe extern "C" fn(nCameraID: c_int, startX: c_int, startY: c_int) -> POAErrors,
    POASetImageSize: unsafe extern "C" fn(nCameraID: c_int, width: c_int, height: c_int) -> POAErrors,
    POASetImageBin: unsafe extern "C" fn(nCameraID: c_int, bin: c_int) -> POAErrors,
    POASetImageFormat: unsafe extern "C" fn(nCameraID: c_int, imgFormat: POAImgFormat) -> POAErrors,
    POAStartExposure: unsafe extern "C" fn(nCameraID: c_int, bSingleFrame: POABool) -> POAErrors,
    POAStopExposure: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POAImageReady: unsafe extern "C" fn(nCameraID: c_int, pIsReady: *mut POABool) -> POAErrors,
    POAGetImageData: unsafe extern "C" fn(nCameraID: c_int, pBuf: *mut c_uchar, lBufSize: c_long, nTimeoutms: c_int) -> POAErrors,
    POAGetSensorModeCount: unsafe extern "C" fn(nCameraID: c_int, pModeCount: *mut c_int) -> POAErrors,
    POAGetSensorModeInfo: unsafe extern "C" fn(nCameraID: c_int, modeIndex: c_int, pSenModeInfo: *mut POASensorModeInfo) -> POAErrors,
    POASetSensorMode: unsafe extern "C" fn(nCameraID: c_int, modeIndex: c_int) -> POAErrors,
    POAGetSensorMode: unsafe extern "C" fn(nCameraID: c_int, pModeIndex: *mut c_int) -> POAErrors,
}

pub struct PlayerOneSdk {
    pub api: Container<PlayerOneSdkApi>,
}

static SDK: OnceLock<Option<PlayerOneSdk>> = OnceLock::new();

impl PlayerOneSdk {
    pub fn try_load() -> Option<&'static Self> {
        SDK.get_or_init(|| {
            let lib_name = if cfg!(windows) {
                "PlayerOneCamera.dll"
            } else if cfg!(target_os = "macos") {
                "libPlayerOneCamera.dylib"
            } else {
                "libPlayerOneCamera.so"
            };

            match unsafe { Container::load(lib_name) } {
                Ok(api) => {
                    info!("PlayerOne SDK ({}) loaded successfully.", lib_name);
                    Some(PlayerOneSdk { api })
                }
                Err(e) => {
                    info!("PlayerOne SDK ({}) not found or failed to load: {}. PlayerOne cameras disabled.", lib_name, e);
                    None
                }
            }
        }).as_ref()
    }
}
