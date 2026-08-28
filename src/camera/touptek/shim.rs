use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
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
unsafe impl Sync for TouptekHandle {}

impl TouptekHandle {
    pub fn open(cam_id: &str) -> Result<Self, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;

        let id_cstring = std::ffi::CString::new(cam_id).map_err(|e| e.to_string())?;
        let handle = unsafe { sdk.api.Toupcam_Open(id_cstring.as_ptr()) };

        if handle.is_null() {
            return Err(format!("Failed to open ToupTek camera {}", cam_id));
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

    pub fn close(&self) {
        if !self.lease.begin_close() {
            return;
        }
        let sdk = match TouptekSdk::try_load() {
            Some(s) => s,
            None => return,
        };

        if self.pull_mode_active.load(Ordering::SeqCst) {
            let _ = unsafe { sdk.api.Toupcam_Stop(self.handle) };
            self.pull_mode_active.store(false, Ordering::SeqCst);
        }

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
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe {
            sdk.api
                .Toupcam_put_AutoExpoEnable(self.handle, if enable { 1 } else { 0 })
        };
        check_hresult(hr, "Toupcam_put_AutoExpoEnable")
    }

    pub fn set_exposure_us(&self, time_us: u32) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_ExpoTime(self.handle, time_us) };
        check_hresult(hr, "Toupcam_put_ExpoTime")
    }

    pub fn get_exposure_us(&self) -> Result<u32, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut time: u32 = 0;
        let hr = unsafe { sdk.api.Toupcam_get_ExpoTime(self.handle, &mut time) };
        check_hresult(hr, "Toupcam_get_ExpoTime")?;
        Ok(time)
    }

    pub fn exposure_range(&self) -> Result<(u32, u32, u32), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut min = 0u32;
        let mut max = 0u32;
        let mut def = 0u32;
        let hr = unsafe {
            sdk.api
                .Toupcam_get_ExpTimeRange(self.handle, &mut min, &mut max, &mut def)
        };
        check_hresult(hr, "Toupcam_get_ExpTimeRange")?;
        Ok((min, max, def))
    }

    // ── Gain (percent, e.g. 100 = 1x, 300 = 3x) ───────────────────────

    pub fn set_gain(&self, gain_pct: u16) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_ExpoAGain(self.handle, gain_pct) };
        check_hresult(hr, "Toupcam_put_ExpoAGain")
    }

    pub fn get_gain(&self) -> Result<u16, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut gain: u16 = 0;
        let hr = unsafe { sdk.api.Toupcam_get_ExpoAGain(self.handle, &mut gain) };
        check_hresult(hr, "Toupcam_get_ExpoAGain")?;
        Ok(gain)
    }

    pub fn gain_range(&self) -> Result<(u16, u16, u16), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut min = 0u16;
        let mut max = 0u16;
        let mut def = 0u16;
        let hr = unsafe {
            sdk.api
                .Toupcam_get_ExpoAGainRange(self.handle, &mut min, &mut max, &mut def)
        };
        check_hresult(hr, "Toupcam_get_ExpoAGainRange")?;
        Ok((min, max, def))
    }

    // ── Resolution & binning ────────────────────────────────────────────

    pub fn set_resolution(&self, width: i32, height: i32) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_Size(self.handle, width, height) };
        check_hresult(hr, "Toupcam_put_Size")
    }

    pub fn get_resolution(&self) -> Result<(i32, i32), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut w = 0i32;
        let mut h = 0i32;
        let hr = unsafe { sdk.api.Toupcam_get_Size(self.handle, &mut w, &mut h) };
        check_hresult(hr, "Toupcam_get_Size")?;
        Ok((w, h))
    }

    pub fn set_resolution_index(&self, index: u32) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_eSize(self.handle, index) };
        check_hresult(hr, "Toupcam_put_eSize")
    }

    pub fn set_binning(&self, bin: u8) -> Result<(), String> {
        self.put_option(TOUPCAM_OPTION_BINNING, bin as i32)
    }

    pub fn set_roi(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_Roi(self.handle, x, y, w, h) };
        check_hresult(hr, "Toupcam_put_Roi")
    }

    // ── Temperature (0.1°C units internally) ────────────────────────────

    pub fn get_temperature_c(&self) -> Result<f64, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut temp: i16 = 0;
        let hr = unsafe { sdk.api.Toupcam_get_Temperature(self.handle, &mut temp) };
        check_hresult(hr, "Toupcam_get_Temperature")?;
        Ok(temp as f64 / 10.0)
    }

    pub fn set_target_temperature_c(&self, temp_c: f64) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let temp_raw = (temp_c * 10.0) as i16;
        let hr = unsafe { sdk.api.Toupcam_put_Temperature(self.handle, temp_raw) };
        check_hresult(hr, "Toupcam_put_Temperature")
    }

    // ── Sensor info ─────────────────────────────────────────────────────

    pub fn raw_format(&self) -> Result<(u32, u32), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut fourcc = 0u32;
        let mut bpp = 0u32;
        let hr = unsafe {
            sdk.api
                .Toupcam_get_RawFormat(self.handle, &mut fourcc, &mut bpp)
        };
        check_hresult(hr, "Toupcam_get_RawFormat")?;
        Ok((fourcc, bpp))
    }

    pub fn is_mono(&self) -> Result<bool, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_get_MonoMode(self.handle) };
        // S_OK (0) = mono, S_FALSE (1) = color
        Ok(hr == S_OK as i32)
    }

    pub fn max_bit_depth(&self) -> Result<u32, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_get_MaxBitDepth(self.handle) };
        if hr < 0 {
            return Err(format!(
                "Toupcam_get_MaxBitDepth failed: HRESULT 0x{:08X}",
                hr as u32
            ));
        }
        Ok(hr as u32)
    }

    pub fn serial_number(&self) -> Result<String, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut sn = [0i8; 32];
        let hr = unsafe {
            sdk.api
                .Toupcam_get_SerialNumber(self.handle, sn.as_mut_ptr())
        };
        check_hresult(hr, "Toupcam_get_SerialNumber")?;
        let s = unsafe { CStr::from_ptr(sn.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Ok(s)
    }

    pub fn pixel_size(&self, res_index: u32) -> Result<(f32, f32), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut x = 0.0f32;
        let mut y = 0.0f32;
        let hr = unsafe {
            sdk.api
                .Toupcam_get_PixelSize(self.handle, res_index, &mut x, &mut y)
        };
        check_hresult(hr, "Toupcam_get_PixelSize")?;
        Ok((x, y))
    }

    // ── Speed ───────────────────────────────────────────────────────────

    pub fn set_max_speed(&self) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let max_speed = unsafe { sdk.api.Toupcam_get_MaxSpeed(self.handle) };
        if max_speed < 0 {
            return Err("Toupcam_get_MaxSpeed failed".to_string());
        }
        let hr = unsafe { sdk.api.Toupcam_put_Speed(self.handle, max_speed as u16) };
        check_hresult(hr, "Toupcam_put_Speed")
    }

    // ── Pull mode capture ───────────────────────────────────────────────

    /// Start pull mode. The event callback is minimal — just for the SDK's
    /// internal state machine. We use WaitImageV3 to actually pull frames.
    pub fn start_pull_mode(&self) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;

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

        let hr = unsafe {
            sdk.api
                .Toupcam_StartPullModeWithCallback(self.handle, Some(event_callback), ctx_ptr)
        };
        check_hresult(hr, "Toupcam_StartPullModeWithCallback")?;
        self.pull_mode_active.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_Stop(self.handle) };
        self.pull_mode_active.store(false, Ordering::SeqCst);
        check_hresult(hr, "Toupcam_Stop")
    }

    /// Wait for and pull a single RAW image frame.
    ///
    /// `timeout_ms`: how long to block waiting for the frame.
    /// `buf`: pre-allocated output buffer (caller must size it correctly).
    /// Returns frame info on success.
    pub fn wait_image_raw(
        &self,
        timeout_ms: u32,
        buf: &mut [u8],
    ) -> Result<ToupcamFrameInfoV3, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut info = std::mem::MaybeUninit::<ToupcamFrameInfoV3>::zeroed();

        let hr = unsafe {
            sdk.api.Toupcam_WaitImageV3(
                self.handle,
                timeout_ms,
                buf.as_mut_ptr() as *mut c_void,
                0, // bStill = false (video/preview frame)
                0, // bits = 0 (use default for RAW mode: native bit depth)
                0, // rowPitch = 0 (let SDK calculate stride automatically)
                info.as_mut_ptr(),
            )
        };
        check_hresult(hr, "Toupcam_WaitImageV3")?;
        Ok(unsafe { info.assume_init() })
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    fn put_option(&self, option: u32, value: i32) -> Result<(), String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let hr = unsafe { sdk.api.Toupcam_put_Option(self.handle, option, value) };
        check_hresult(hr, &format!("Toupcam_put_Option(0x{:02X})", option))
    }

    fn get_option(&self, option: u32) -> Result<i32, String> {
        let sdk = TouptekSdk::try_load().ok_or("ToupTek SDK not loaded")?;
        let mut value = 0i32;
        let hr = unsafe { sdk.api.Toupcam_get_Option(self.handle, option, &mut value) };
        check_hresult(hr, &format!("Toupcam_get_Option(0x{:02X})", option))?;
        Ok(value)
    }
}

impl Drop for TouptekHandle {
    fn drop(&mut self) {
        self.close();
    }
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

    let count = unsafe { sdk.api.Toupcam_EnumV2(arr.as_mut_ptr()) };
    arr.truncate(count as usize);
    Some(arr)
}
