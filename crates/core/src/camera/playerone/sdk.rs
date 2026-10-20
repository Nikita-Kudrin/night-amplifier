#![allow(non_snake_case)]
use dlopen2::wrapper::{Container, WrapperApi};
use std::os::raw::{c_char, c_int, c_long, c_uchar};
use std::sync::OnceLock;
use tracing::info;

use super::ffi_types::*;

#[derive(WrapperApi)]
pub struct PlayerOneSdkApi {
    POAGetCameraCount: unsafe extern "C" fn() -> c_int,
    POAGetCameraProperties:
        unsafe extern "C" fn(nIndex: c_int, pProp: *mut POACameraProperties) -> POAErrors,
    POAOpenCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POAInitCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POACloseCamera: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POASetConfig: unsafe extern "C" fn(
        nCameraID: c_int,
        confID: POAConfig,
        confValue: POAConfigValue,
        isAuto: POABool,
    ) -> POAErrors,
    POAGetConfig: unsafe extern "C" fn(
        nCameraID: c_int,
        confID: POAConfig,
        pConfValue: *mut POAConfigValue,
        pIsAuto: *mut POABool,
    ) -> POAErrors,
    POASetImageStartPos:
        unsafe extern "C" fn(nCameraID: c_int, startX: c_int, startY: c_int) -> POAErrors,
    POASetImageSize:
        unsafe extern "C" fn(nCameraID: c_int, width: c_int, height: c_int) -> POAErrors,
    POASetImageBin: unsafe extern "C" fn(nCameraID: c_int, bin: c_int) -> POAErrors,
    POASetImageFormat: unsafe extern "C" fn(nCameraID: c_int, imgFormat: POAImgFormat) -> POAErrors,
    POAStartExposure: unsafe extern "C" fn(nCameraID: c_int, bSingleFrame: POABool) -> POAErrors,
    POAStopExposure: unsafe extern "C" fn(nCameraID: c_int) -> POAErrors,
    POAImageReady: unsafe extern "C" fn(nCameraID: c_int, pIsReady: *mut POABool) -> POAErrors,
    POAGetImageData: unsafe extern "C" fn(
        nCameraID: c_int,
        pBuf: *mut c_uchar,
        lBufSize: c_long,
        nTimeoutms: c_int,
    ) -> POAErrors,
    POAGetSensorModeCount:
        unsafe extern "C" fn(nCameraID: c_int, pModeCount: *mut c_int) -> POAErrors,
    POAGetSensorModeInfo: unsafe extern "C" fn(
        nCameraID: c_int,
        modeIndex: c_int,
        pSenModeInfo: *mut POASensorModeInfo,
    ) -> POAErrors,
    POASetSensorMode: unsafe extern "C" fn(nCameraID: c_int, modeIndex: c_int) -> POAErrors,
    POAGetSensorMode: unsafe extern "C" fn(nCameraID: c_int, pModeIndex: *mut c_int) -> POAErrors,
}

/// `POAGetDroppedImagesCount`: frames dropped in video mode since the stream started.
pub type DroppedImagesCount =
    unsafe extern "C" fn(nCameraID: c_int, pDroppedCount: *mut c_int) -> POAErrors;

/// `POAGetConfigAttributesByConfigID`: a config's range and access flags.
pub type ConfigAttributesByConfigId = unsafe extern "C" fn(
    nCameraID: c_int,
    confID: POAConfig,
    pConfAttr: *mut POAConfigAttributes,
) -> POAErrors;

/// `POAGetSDKVersion`: the SDK's version string, for the load line.
pub type SdkVersion = unsafe extern "C" fn() -> *const c_char;

/// The SDK's own version, if it says. Field logs need it to tell an SDK fix from a bug.
fn sdk_version(lib_name: &str) -> Option<String> {
    // SAFETY: `SdkVersion` is the symbol's C signature, and the SDK stays loaded: this runs only
    // from `try_load`'s `Ok(api)` arm, whose `api` holds it until it moves into the static.
    let version = unsafe { crate::native_library::optional_symbol_lazy::<SdkVersion>(lib_name, "POAGetSDKVersion") }?;
    let text = crate::ffi_safety::catch_ffi_panic("PlayerOne::sdk_version", || {
        // SAFETY: no arguments, through the signature it was looked up with.
        let ptr = unsafe { version() };
        // SAFETY: non-null, and `POAGetSDKVersion` returns the SDK's own NUL-terminated string.
        (!ptr.is_null()).then(|| unsafe { std::ffi::CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
    });
    text.ok().flatten()
}

/// The optional symbols are looked up apart from `api`, so an SDK without them still loads.
pub struct PlayerOneSdk {
    pub api: Container<PlayerOneSdkApi>,
    pub dropped_images_count: Option<DroppedImagesCount>,
    pub config_attributes: Option<ConfigAttributesByConfigId>,
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

            // SAFETY: the `PlayerOneSdkApi` struct mirrors the SDK header's signatures, and loading
            // runs only the vendor library's initialisers.
            match unsafe { Container::load(lib_name) } {
                Ok(api) => {
                    let version = sdk_version(lib_name).unwrap_or_else(|| "unknown".to_string());
                    info!(version = %version, "PlayerOne SDK ({}) loaded successfully.", lib_name);
                    use crate::native_library::optional_symbol_lazy;
                    // SAFETY: both types are the symbols' C signatures, and the SDK stays loaded:
                    // `api` holds it here, then the static it moves into.
                    let (dropped_images_count, config_attributes) = unsafe {
                        (
                            optional_symbol_lazy::<DroppedImagesCount>(
                                lib_name,
                                "POAGetDroppedImagesCount",
                            ),
                            optional_symbol_lazy::<ConfigAttributesByConfigId>(
                                lib_name,
                                "POAGetConfigAttributesByConfigID",
                            ),
                        )
                    };
                    Some(PlayerOneSdk {
                        api,
                        dropped_images_count,
                        config_attributes,
                    })
                }
                Err(e) => {
                    info!("PlayerOne SDK ({}) not found or failed to load: {}. PlayerOne cameras disabled.", lib_name, e);
                    None
                }
            }
        }).as_ref()
    }
}
