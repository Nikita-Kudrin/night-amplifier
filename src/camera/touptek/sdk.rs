#![allow(non_snake_case)]
use dlopen2::wrapper::{Container, WrapperApi};
use std::os::raw::{c_char, c_int, c_short, c_uint, c_ushort, c_void};
use std::sync::OnceLock;
use tracing::{info, warn};

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct TouptekSdkApi {
    // Enumeration
    Toupcam_EnumV2: unsafe extern "C" fn(arr: *mut ToupcamDeviceV2) -> c_uint,

    // Lifecycle
    Toupcam_Open: unsafe extern "C" fn(camId: *const c_char) -> HToupcam,
    Toupcam_Close: unsafe extern "C" fn(h: HToupcam),

    // Pull mode
    Toupcam_StartPullModeWithCallback: unsafe extern "C" fn(
        h: HToupcam,
        funEvent: PTOUPCAM_EVENT_CALLBACK,
        ctxEvent: *mut c_void,
    ) -> c_int,
    Toupcam_Stop: unsafe extern "C" fn(h: HToupcam) -> c_int,
    Toupcam_PullImageV3: unsafe extern "C" fn(
        h: HToupcam,
        pImageData: *mut c_void,
        bStill: c_int,
        bits: c_int,
        rowPitch: c_int,
        pInfo: *mut ToupcamFrameInfoV3,
    ) -> c_int,
    Toupcam_WaitImageV3: unsafe extern "C" fn(
        h: HToupcam,
        nWaitMS: c_uint,
        pImageData: *mut c_void,
        bStill: c_int,
        bits: c_int,
        rowPitch: c_int,
        pInfo: *mut ToupcamFrameInfoV3,
    ) -> c_int,

    // Exposure & Gain
    Toupcam_put_ExpoTime: unsafe extern "C" fn(h: HToupcam, Time: c_uint) -> c_int,
    Toupcam_get_ExpoTime: unsafe extern "C" fn(h: HToupcam, Time: *mut c_uint) -> c_int,
    Toupcam_put_ExpoAGain: unsafe extern "C" fn(h: HToupcam, Gain: c_ushort) -> c_int,
    Toupcam_get_ExpoAGain: unsafe extern "C" fn(h: HToupcam, Gain: *mut c_ushort) -> c_int,
    Toupcam_get_ExpoAGainRange: unsafe extern "C" fn(
        h: HToupcam,
        nMin: *mut c_ushort,
        nMax: *mut c_ushort,
        nDef: *mut c_ushort,
    ) -> c_int,
    Toupcam_get_ExpTimeRange: unsafe extern "C" fn(
        h: HToupcam,
        nMin: *mut c_uint,
        nMax: *mut c_uint,
        nDef: *mut c_uint,
    ) -> c_int,

    // Auto exposure control
    Toupcam_put_AutoExpoEnable: unsafe extern "C" fn(h: HToupcam, mode: c_int) -> c_int,

    // Resolution & ROI
    Toupcam_put_Size: unsafe extern "C" fn(h: HToupcam, nWidth: c_int, nHeight: c_int) -> c_int,
    Toupcam_get_Size:
        unsafe extern "C" fn(h: HToupcam, pWidth: *mut c_int, pHeight: *mut c_int) -> c_int,
    Toupcam_put_eSize: unsafe extern "C" fn(h: HToupcam, nResolutionIndex: c_uint) -> c_int,
    Toupcam_put_Roi: unsafe extern "C" fn(
        h: HToupcam,
        xOffset: c_uint,
        yOffset: c_uint,
        xWidth: c_uint,
        yHeight: c_uint,
    ) -> c_int,

    // Options (RAW mode, binning, bit depth, etc.)
    Toupcam_put_Option: unsafe extern "C" fn(h: HToupcam, iOption: c_uint, iValue: c_int) -> c_int,
    Toupcam_get_Option:
        unsafe extern "C" fn(h: HToupcam, iOption: c_uint, piValue: *mut c_int) -> c_int,

    // Temperature & cooling (0.1°C units)
    Toupcam_get_Temperature: unsafe extern "C" fn(h: HToupcam, pTemperature: *mut c_short) -> c_int,
    Toupcam_put_Temperature: unsafe extern "C" fn(h: HToupcam, nTemperature: c_short) -> c_int,

    // Sensor info
    Toupcam_get_RawFormat: unsafe extern "C" fn(
        h: HToupcam,
        pFourCC: *mut c_uint,
        pBitsPerPixel: *mut c_uint,
    ) -> c_int,
    Toupcam_get_MonoMode: unsafe extern "C" fn(h: HToupcam) -> c_int,
    Toupcam_get_MaxBitDepth: unsafe extern "C" fn(h: HToupcam) -> c_int,
    Toupcam_get_SerialNumber: unsafe extern "C" fn(h: HToupcam, sn: *mut c_char) -> c_int,
    Toupcam_get_PixelSize: unsafe extern "C" fn(
        h: HToupcam,
        nResolutionIndex: c_uint,
        x: *mut f32,
        y: *mut f32,
    ) -> c_int,

    // Speed
    Toupcam_get_MaxSpeed: unsafe extern "C" fn(h: HToupcam) -> c_int,
    Toupcam_put_Speed: unsafe extern "C" fn(h: HToupcam, nSpeed: c_ushort) -> c_int,

    // Resolution count
    Toupcam_get_ResolutionNumber: unsafe extern "C" fn(h: HToupcam) -> c_int,
    Toupcam_get_Resolution: unsafe extern "C" fn(
        h: HToupcam,
        nResolutionIndex: c_uint,
        pWidth: *mut c_int,
        pHeight: *mut c_int,
    ) -> c_int,
}

pub struct TouptekSdk {
    pub api: Container<TouptekSdkApi>,
}

static SDK: OnceLock<Option<TouptekSdk>> = OnceLock::new();

impl TouptekSdk {
    pub fn try_load() -> Option<&'static Self> {
        SDK.get_or_init(|| {
            let lib_name = if cfg!(windows) {
                "toupcam.dll"
            } else if cfg!(target_os = "macos") {
                "libtoupcam.dylib"
            } else {
                "libtoupcam.so"
            };

            match unsafe { Container::<TouptekSdkApi>::load(lib_name) } {
                Ok(api) => {
                    info!("ToupTek SDK ({}) loaded successfully.", lib_name);
                    Some(TouptekSdk { api })
                }
                Err(e) => {
                    info!(
                        "ToupTek SDK ({}) not found or failed to load: {}. ToupTek cameras disabled.",
                        lib_name, e
                    );
                    None
                }
            }
        })
        .as_ref()
    }
}

/// ToupTek uses Windows-style HRESULT: >= 0 is success, < 0 is failure.
pub fn check_hresult(hr: i32, context: &str) -> Result<(), String> {
    if hr >= 0 {
        Ok(())
    } else {
        Err(format!("{} failed: HRESULT 0x{:08X}", context, hr as u32))
    }
}
