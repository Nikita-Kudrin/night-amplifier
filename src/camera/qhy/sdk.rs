#![allow(non_snake_case)]
use std::os::raw::c_char;
use dlopen2::wrapper::{Container, WrapperApi};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct QhySdkApi {
    InitQHYCCDResource: unsafe extern "C" fn() -> u32,
    ReleaseQHYCCDResource: unsafe extern "C" fn() -> u32,
    ScanQHYCCD: unsafe extern "C" fn() -> u32,
    GetQHYCCDId: unsafe extern "C" fn(index: u32, id: *mut c_char) -> u32,
    OpenQHYCCD: unsafe extern "C" fn(id: *const c_char) -> QhyccdHandle,
    CloseQHYCCD: unsafe extern "C" fn(handle: QhyccdHandle) -> u32,
    SetQHYCCDStreamMode: unsafe extern "C" fn(handle: QhyccdHandle, mode: u8) -> u32,
    InitQHYCCD: unsafe extern "C" fn(handle: QhyccdHandle) -> u32,
    IsQHYCCDControlAvailable: unsafe extern "C" fn(handle: QhyccdHandle, ctrl: u32) -> u32,
    SetQHYCCDParam: unsafe extern "C" fn(handle: QhyccdHandle, ctrl: u32, value: f64) -> u32,
    GetQHYCCDParam: unsafe extern "C" fn(handle: QhyccdHandle, ctrl: u32) -> f64,
    GetQHYCCDParamMinMaxStep: unsafe extern "C" fn(
        handle: QhyccdHandle,
        ctrl: u32,
        min: *mut f64,
        max: *mut f64,
        step: *mut f64,
    ) -> u32,
    SetQHYCCDResolution: unsafe extern "C" fn(
        handle: QhyccdHandle,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> u32,
    SetQHYCCDBinMode: unsafe extern "C" fn(handle: QhyccdHandle, wbin: u32, hbin: u32) -> u32,
    SetQHYCCDBitsMode: unsafe extern "C" fn(handle: QhyccdHandle, bits: u32) -> u32,
    ExpQHYCCDSingleFrame: unsafe extern "C" fn(handle: QhyccdHandle) -> u32,
    GetQHYCCDSingleFrame: unsafe extern "C" fn(
        handle: QhyccdHandle,
        w: *mut u32,
        h: *mut u32,
        bpp: *mut u32,
        channels: *mut u32,
        img_data: *mut u8,
    ) -> u32,
    CancelQHYCCDExposingAndReadout: unsafe extern "C" fn(handle: QhyccdHandle) -> u32,
    GetQHYCCDChipInfo: unsafe extern "C" fn(
        handle: QhyccdHandle,
        chip_w: *mut f64,
        chip_h: *mut f64,
        img_w: *mut u32,
        img_h: *mut u32,
        pixel_w: *mut f64,
        pixel_h: *mut f64,
        bpp: *mut u32,
    ) -> u32,
    IsQHYCCDCFWPlugged: unsafe extern "C" fn(handle: QhyccdHandle) -> u32,
}

pub struct QhySdk {
    pub api: Container<QhySdkApi>,
}

static SDK: OnceLock<Option<QhySdk>> = OnceLock::new();

impl QhySdk {
    pub fn try_load() -> Option<&'static Self> {
        SDK.get_or_init(|| {
            let lib_name = if cfg!(windows) {
                "qhyccd.dll"
            } else if cfg!(target_os = "macos") {
                "libqhyccd.dylib"
            } else {
                "libqhyccd.so"
            };

            match unsafe { Container::<QhySdkApi>::load(lib_name) } {
                Ok(api) => {
                    info!("QHY SDK ({}) loaded successfully.", lib_name);
                    // Initialize the QHY resource (required by QHY SDK)
                    let res = unsafe { api.InitQHYCCDResource() };
                    if res != QHYCCD_SUCCESS {
                        warn!("QHY SDK loaded but InitQHYCCDResource failed with code {}", res);
                        // Still returning it, but it might not work. Often it's fine.
                    }
                    Some(QhySdk { api })
                }
                Err(e) => {
                    info!("QHY SDK ({}) not found or failed to load: {}. QHY cameras disabled.", lib_name, e);
                    None
                }
            }
        }).as_ref()
    }
}
