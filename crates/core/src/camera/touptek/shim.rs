use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::warn;

use super::ffi_types::*;
use super::sdk::{check_hresult, TouptekSdk};
use crate::camera::DeviceLease;
use crate::CfaPattern;

/// Provider key for [`DeviceLease`] slots. ToupTek closes by opaque pointer,
/// so reopening cannot alias a live handle; the lease is here for the
/// double-close guard — `close()` is followed by `Drop`, and `Toupcam_Close`
/// twice on one pointer is a use-after-free.
pub(super) const PROVIDER: &str = "ToupTek";

pub struct TouptekHandle {
    handle: HToupcam,
    pull_mode_active: AtomicBool,
    fatal_error_flag: Arc<AtomicBool>,
    /// Gates `Toupcam_Close` to exactly one call per handle.
    lease: DeviceLease,
}

// SAFETY: ToupTek SDK handles are bound to a single device. All access to
// TouptekHandle goes through AppState's StdMutex, which serializes calls.
unsafe impl Send for TouptekHandle {}

impl TouptekHandle {
    /// `cam_id` is the enumerated `ToupcamDeviceV2::id`, passed on unconverted: it is UTF-16
    /// on Windows.
    pub fn open(cam_id: &[TChar; 64]) -> Result<Self, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;

        // The SDK reads the id up to its terminator; make sure there is one.
        let mut id = *cam_id;
        id[id.len() - 1] = 0;
        // SAFETY: `id` is a local copy, NUL-terminated just above.
        let handle = unsafe { sdk.api.Toupcam_Open(id.as_ptr()) };

        if handle.is_null() {
            return Err(format!("Failed to open ToupTek camera {}", tchar_to_string(&id)));
        }

        Ok(Self {
            handle,
            pull_mode_active: AtomicBool::new(false),
            fatal_error_flag: Arc::new(AtomicBool::new(false)),
            lease: DeviceLease::acquire_unique(PROVIDER),
        })
    }

    pub fn get_fatal_error_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.fatal_error_flag)
    }

    /// The SDK and this handle, refused once `close` has run: the handle is the pointer that
    /// close freed, so any later call would be a use-after-free. `close` and every call take
    /// `&self` of a `!Sync` handle, so none runs while another does.
    fn open_sdk(&self) -> Result<(&'static TouptekSdk, HToupcam), String> {
        if self.lease.is_closed() {
            return Err("ToupTek camera handle is closed".to_string());
        }
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        Ok((sdk, self.handle))
    }

    pub fn close(&self) {
        if !self.lease.begin_close() {
            return;
        }
        let sdk = match TouptekSdk::try_load() {
            Some(s) => s,
            None => return,
        };

        // SAFETY (both blocks): the lease authorizes one close per handle, and `open_sdk` refuses
        // every call after it, so `self.handle` is still open here and never used again.
        if self.pull_mode_active.load(Ordering::SeqCst) {
            // SAFETY: as above.
            let _ = unsafe { sdk.api.Toupcam_Stop(self.handle) };
            self.pull_mode_active.store(false, Ordering::SeqCst);
        }

        // SAFETY: as above.
        unsafe { sdk.api.Toupcam_Close(self.handle) };
    }

    // ── RAW mode & bit depth ────────────────────────────────────────────

    pub fn set_raw_mode(&self, enable: bool) -> Result<(), String> {
        self.put_option(TOUPCAM_OPTION_RAW, if enable { 1 } else { 0 })
    }

    pub fn set_bit_depth(&self, high: bool) -> Result<(), String> {
        self.put_option(TOUPCAM_OPTION_BITDEPTH, if high { 1 } else { 0 })
    }

    // ── Exposure ────────────────────────────────────────────────────────

    pub fn set_auto_exposure(&self, enable: bool) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe {
            sdk.api
                .Toupcam_put_AutoExpoEnable(handle, if enable { 1 } else { 0 })
        };
        check_hresult(hr, "Toupcam_put_AutoExpoEnable")
    }

    pub fn set_exposure_us(&self, time_us: u32) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_ExpoTime(handle, time_us) };
        check_hresult(hr, "Toupcam_put_ExpoTime")
    }

    pub fn get_exposure_us(&self) -> Result<u32, String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut time: u32 = 0;
        // SAFETY: `time` is a live local the SDK writes during the call.
        let hr = unsafe { sdk.api.Toupcam_get_ExpoTime(handle, &mut time) };
        check_hresult(hr, "Toupcam_get_ExpoTime")?;
        Ok(time)
    }

    pub fn exposure_range(&self) -> Result<(u32, u32, u32), String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut min = 0u32;
        let mut max = 0u32;
        let mut def = 0u32;
        // SAFETY: `min`, `max`, `def` are live locals the SDK writes during the call.
        let hr = unsafe {
            sdk.api
                .Toupcam_get_ExpTimeRange(handle, &mut min, &mut max, &mut def)
        };
        check_hresult(hr, "Toupcam_get_ExpTimeRange")?;
        Ok((min, max, def))
    }

    // ── Gain (percent, e.g. 100 = 1x, 300 = 3x) ───────────────────────

    pub fn set_gain(&self, gain_pct: u16) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_ExpoAGain(handle, gain_pct) };
        check_hresult(hr, "Toupcam_put_ExpoAGain")
    }

    pub fn get_gain(&self) -> Result<u16, String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut gain: u16 = 0;
        // SAFETY: `gain` is a live local the SDK writes during the call.
        let hr = unsafe { sdk.api.Toupcam_get_ExpoAGain(handle, &mut gain) };
        check_hresult(hr, "Toupcam_get_ExpoAGain")?;
        Ok(gain)
    }

    pub fn gain_range(&self) -> Result<(u16, u16, u16), String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut min = 0u16;
        let mut max = 0u16;
        let mut def = 0u16;
        // SAFETY: `min`, `max`, `def` are live locals the SDK writes during the call.
        let hr = unsafe {
            sdk.api
                .Toupcam_get_ExpoAGainRange(handle, &mut min, &mut max, &mut def)
        };
        check_hresult(hr, "Toupcam_get_ExpoAGainRange")?;
        Ok((min, max, def))
    }

    // ── Resolution & binning ────────────────────────────────────────────

    pub fn set_resolution(&self, width: i32, height: i32) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_Size(handle, width, height) };
        check_hresult(hr, "Toupcam_put_Size")
    }

    pub fn get_resolution(&self) -> Result<(i32, i32), String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut w = 0i32;
        let mut h = 0i32;
        // SAFETY: `w`, `h` are live locals the SDK writes during the call.
        let hr = unsafe { sdk.api.Toupcam_get_Size(handle, &mut w, &mut h) };
        check_hresult(hr, "Toupcam_get_Size")?;
        Ok((w, h))
    }

    pub fn set_resolution_index(&self, index: u32) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_eSize(handle, index) };
        check_hresult(hr, "Toupcam_put_eSize")
    }

    pub fn set_binning(&self, bin: u8) -> Result<(), String> {
        self.put_option(TOUPCAM_OPTION_BINNING, bin as i32)
    }

    pub fn set_roi(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_Roi(handle, x, y, w, h) };
        check_hresult(hr, "Toupcam_put_Roi")
    }

    // ── Temperature (0.1°C units internally) ────────────────────────────

    pub fn get_temperature_c(&self) -> Result<f64, String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut temp: i16 = 0;
        // SAFETY: `temp` is a live local the SDK writes during the call.
        let hr = unsafe { sdk.api.Toupcam_get_Temperature(handle, &mut temp) };
        check_hresult(hr, "Toupcam_get_Temperature")?;
        Ok(temp as f64 / 10.0)
    }

    pub fn set_target_temperature_c(&self, temp_c: f64) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        let temp_raw = (temp_c * 10.0) as i16;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_Temperature(handle, temp_raw) };
        check_hresult(hr, "Toupcam_put_Temperature")
    }

    // ── Sensor info ─────────────────────────────────────────────────────

    pub fn raw_format(&self) -> Result<(u32, u32), String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut fourcc = 0u32;
        let mut bpp = 0u32;
        // SAFETY: `fourcc`, `bpp` are live locals the SDK writes during the call.
        let hr = unsafe {
            sdk.api
                .Toupcam_get_RawFormat(handle, &mut fourcc, &mut bpp)
        };
        check_hresult(hr, "Toupcam_get_RawFormat")?;
        Ok((fourcc, bpp))
    }

    pub fn is_mono(&self) -> Result<bool, String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`).
        let hr = unsafe { sdk.api.Toupcam_get_MonoMode(handle) };
        // S_OK (0) = mono, S_FALSE (1) = color
        Ok(hr == S_OK as i32)
    }

    pub fn max_bit_depth(&self) -> Result<u32, String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`).
        let hr = unsafe { sdk.api.Toupcam_get_MaxBitDepth(handle) };
        if hr < 0 {
            return Err(format!(
                "Toupcam_get_MaxBitDepth failed: HRESULT 0x{:08X}",
                hr as u32
            ));
        }
        Ok(hr as u32)
    }

    pub fn serial_number(&self) -> Result<String, String> {
        let (sdk, handle) = self.open_sdk()?;
        // `c_char`, not `i8`: it is `u8` on Linux ARM, the Raspberry Pi builds.
        let mut sn = [0 as std::os::raw::c_char; 32];
        // SAFETY: `sn` holds the 32 bytes `toupcam.h` gives a serial number.
        let hr = unsafe {
            sdk.api
                .Toupcam_get_SerialNumber(handle, sn.as_mut_ptr())
        };
        check_hresult(hr, "Toupcam_get_SerialNumber")?;
        Ok(crate::ffi_safety::fixed_c_str(&sn))
    }

    pub fn pixel_size(&self, res_index: u32) -> Result<(f32, f32), String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        // SAFETY: `x`, `y` are live locals the SDK writes during the call.
        let hr = unsafe {
            sdk.api
                .Toupcam_get_PixelSize(handle, res_index, &mut x, &mut y)
        };
        check_hresult(hr, "Toupcam_get_PixelSize")?;
        Ok((x, y))
    }

    // ── Speed ───────────────────────────────────────────────────────────

    pub fn set_max_speed(&self) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`).
        let max_speed = unsafe { sdk.api.Toupcam_get_MaxSpeed(handle) };
        if max_speed < 0 {
            return Err("Toupcam_get_MaxSpeed failed".to_string());
        }
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_Speed(handle, max_speed as u16) };
        check_hresult(hr, "Toupcam_put_Speed")
    }

    // ── Pull mode capture ───────────────────────────────────────────────

    /// Start pull mode. The event callback is minimal — just for the SDK's
    /// internal state machine. We use WaitImageV3 to actually pull frames.
    pub fn start_pull_mode(&self) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;

        self.fatal_error_flag.store(false, Ordering::SeqCst);

        unsafe extern "C" fn event_callback(event: u32, ctx: *mut c_void) {
            if (event == TOUPCAM_EVENT_ERROR || event == TOUPCAM_EVENT_DISCONNECTED)
                && !ctx.is_null()
            {
                let flag = &*(ctx as *const AtomicBool);
                flag.store(true, Ordering::SeqCst);
            }
        }

        let ctx_ptr = Arc::as_ptr(&self.fatal_error_flag) as *mut c_void;

        // SAFETY: `ctx_ptr` points into `fatal_error_flag`, which outlives pull mode: `close`, run
        // by `Drop` at the latest, stops it first. `event_callback` has the SDK's callback type.
        let hr = unsafe {
            sdk.api
                .Toupcam_StartPullModeWithCallback(handle, Some(event_callback), ctx_ptr)
        };
        check_hresult(hr, "Toupcam_StartPullModeWithCallback")?;
        self.pull_mode_active.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`).
        let hr = unsafe { sdk.api.Toupcam_Stop(handle) };
        self.pull_mode_active.store(false, Ordering::SeqCst);
        check_hresult(hr, "Toupcam_Stop")
    }

    /// Wait up to `timeout_ms` for one RAW frame, pulled into `buf`. A `buf` smaller than the
    /// most the SDK may write is refused before the call, which takes no length.
    pub fn wait_image_raw(&self, timeout_ms: u32, buf: &mut [u8]) -> Result<ToupcamFrameInfoV3, WaitError> {
        let (sdk, handle) = self.open_sdk().map_err(WaitError::Refused)?;
        let need = self.raw_frame_len().map_err(WaitError::NotReady)?;
        check_raw_buffer(buf.len(), need)?;
        let mut info = std::mem::MaybeUninit::<ToupcamFrameInfoV3>::zeroed();

        // SAFETY: `handle` is open (`open_sdk`); `buf` holds the most a RAW frame takes at the
        // SDK's own resolution and bit depth (`check_raw_buffer`); `info` is a live local.
        let hr = unsafe {
            sdk.api.Toupcam_WaitImageV3(
                handle,
                timeout_ms,
                buf.as_mut_ptr() as *mut c_void,
                0, // bStill = false (video/preview frame)
                0, // bits = 0 (use default for RAW mode: native bit depth)
                0, // rowPitch = 0 (let SDK calculate stride automatically)
                info.as_mut_ptr(),
            )
        };
        check_hresult(hr, "Toupcam_WaitImageV3").map_err(WaitError::NotReady)?;
        // SAFETY: zeroed above, and every field is an integer, so any bytes are valid.
        Ok(unsafe { info.assume_init() })
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Bytes a RAW frame takes at most: the current resolution (an ROI is within it), 16 bits a
    /// sample with the high-bit-depth option on, else 8.
    fn raw_frame_len(&self) -> Result<usize, String> {
        let (w, h) = self.get_resolution()?;
        let bytes = if self.get_option(TOUPCAM_OPTION_BITDEPTH)? != 0 { 2 } else { 1 };
        Ok(w.max(0) as usize * h.max(0) as usize * bytes)
    }

    fn put_option(&self, option: u32, value: i32) -> Result<(), String> {
        let (sdk, handle) = self.open_sdk()?;
        // SAFETY: `handle` is open (`open_sdk`); the rest are plain values.
        let hr = unsafe { sdk.api.Toupcam_put_Option(handle, option, value) };
        check_hresult(hr, &format!("Toupcam_put_Option(0x{:02X})", option))
    }

    fn get_option(&self, option: u32) -> Result<i32, String> {
        let (sdk, handle) = self.open_sdk()?;
        let mut value = 0i32;
        // SAFETY: `value` is a live local the SDK writes during the call.
        let hr = unsafe { sdk.api.Toupcam_get_Option(handle, option, &mut value) };
        check_hresult(hr, &format!("Toupcam_get_Option(0x{:02X})", option))?;
        Ok(value)
    }
}

impl Drop for TouptekHandle {
    fn drop(&mut self) {
        self.close();
    }
}

/// Why [`TouptekHandle::wait_image_raw`] returned no frame.
#[derive(Debug, PartialEq)]
pub enum WaitError {
    /// None yet, or the SDK refused: the next wait may deliver.
    NotReady(String),
    /// No wait can deliver: the handle is closed, the SDK gone, or the buffer smaller than
    /// the frame the SDK would write.
    Refused(String),
}

fn check_raw_buffer(have: usize, need: usize) -> Result<(), WaitError> {
    if have < need {
        let message = format!("frame buffer of {have} bytes is smaller than the {need} the camera writes");
        return Err(WaitError::Refused(message));
    }
    Ok(())
}

/// Parse the ToupTek FourCC into a CfaPattern.
pub fn parse_fourcc_bayer(fourcc: u32) -> Option<CfaPattern> {
    match fourcc {
        FOURCC_RGGB => Some(CfaPattern::Rggb),
        FOURCC_BGGR => Some(CfaPattern::Bggr),
        FOURCC_GRBG => Some(CfaPattern::Grbg),
        FOURCC_GBRG => Some(CfaPattern::Gbrg),
        FOURCC_YYYY => None, // mono
        _ => {
            warn!("Unknown ToupTek FourCC: 0x{:08X}, treating as mono", fourcc);
            None
        }
    }
}

/// Enumerate connected ToupTek cameras. Returns the raw device list.
pub fn enumerate_devices() -> Option<Vec<ToupcamDeviceV2>> {
    let sdk = TouptekSdk::try_load()?;

    let mut arr = vec![
        ToupcamDeviceV2 {
            displayname: [0; 64],
            id: [0; 64],
            model: std::ptr::null(),
        };
        TOUPCAM_MAX as usize
    ];

    // SAFETY: `arr` holds `TOUPCAM_MAX` entries, the most `Toupcam_EnumV2` writes.
    let count = unsafe { sdk.api.Toupcam_EnumV2(arr.as_mut_ptr()) };
    arr.truncate(count as usize);
    Some(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_buffer_smaller_than_the_raw_frame_is_refused() {
        assert!(matches!(check_raw_buffer(7, 8), Err(WaitError::Refused(_))));
        assert_eq!(check_raw_buffer(8, 8), Ok(()));
    }

    /// After `close` the handle is the pointer the SDK freed: nothing may reach the SDK with it.
    #[test]
    fn a_closed_handle_refuses_every_call_without_the_sdk() {
        let camera = TouptekHandle {
            handle: std::ptr::null_mut(),
            pull_mode_active: AtomicBool::new(false),
            fatal_error_flag: Arc::new(AtomicBool::new(false)),
            lease: DeviceLease::acquire_unique(PROVIDER),
        };
        assert!(camera.lease.begin_close(), "closed as `close` would, short of the SDK call");
        assert!(camera.get_gain().is_err());
        assert!(camera.set_exposure_us(1000).is_err());
        assert!(matches!(camera.wait_image_raw(0, &mut [0; 4]), Err(WaitError::Refused(_))));
        camera.close();
    }
}
