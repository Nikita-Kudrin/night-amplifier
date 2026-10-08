use std::ffi::CStr;
use std::os::raw::c_char;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tracing::warn;

use super::ffi_types::*;
use super::sdk::QhySdk;
use crate::camera::DeviceLease;

/// Provider key for [`DeviceLease`] slots. QHY closes by opaque pointer, so the lease is the
/// double-close guard (`close()` is followed by `Drop`, and `CloseQHYCCD` twice on one pointer
/// is a use-after-free) and it names the device id: one QHY device has at most one handle in
/// this process, since what a second `OpenQHYCCD` of an open id returns is undocumented.
pub(super) const PROVIDER: &str = "QHY";

/// How long `open` waits for another handle to let go of the device — discovery holds one
/// for a few seconds to read its capabilities. A device held longer is refused.
const HELD_WAIT: Duration = Duration::from_secs(5);

/// Serializes the SDK's device-table calls — scan, open, init, close — which discovery,
/// connect and recovery otherwise make from different threads at once. Calls on an open
/// handle during capture are left out.
fn device_table_calls() -> MutexGuard<'static, ()> {
    static CALLS: Mutex<()> = Mutex::new(());
    CALLS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Claim `id` for this process, then run `open`. The claim is atomic and taken before the SDK
/// sees the id, so discovery and a connect never open one device side by side; a failed open
/// gives it back.
fn claim_then_open<H>(
    id: &str,
    wait: Duration,
    open: impl FnOnce() -> Result<H, String>,
) -> Result<(H, DeviceLease), String> {
    let deadline = Instant::now() + wait;
    let lease = loop {
        if let Some(lease) = DeviceLease::try_acquire_unique_device(PROVIDER, id) {
            break lease;
        }
        if Instant::now() >= deadline {
            return Err(format!("QHY camera {id} is held by another handle"));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    match open() {
        Ok(handle) => Ok((handle, lease)),
        Err(e) => {
            lease.begin_close();
            Err(e)
        }
    }
}

pub struct ChipInfo {
    pub chip_w: f64,
    pub chip_h: f64,
    pub img_w: u32,
    pub img_h: u32,
    pub pixel_w: f64,
    pub pixel_h: f64,
    pub bpp: u32,
    pub bayer: String,
}

pub struct QhyHandle {
    handle: QhyccdHandle,
    /// Gates `CloseQHYCCD` to exactly one call per handle.
    lease: DeviceLease,
}

// SAFETY: QHY SDK handles are bound to a single device. All access to
// QhyHandle goes through AppState's StdMutex, which serializes calls.
unsafe impl Send for QhyHandle {}
unsafe impl Sync for QhyHandle {}

impl QhyHandle {
    /// Open `id`, waiting briefly for another handle of this process to let go of it.
    pub fn open(id: &str) -> Result<Self, String> {
        Self::open_claimed(id, HELD_WAIT)
    }

    /// Open `id` only if no other handle of this process holds it — for discovery, which must
    /// neither wait on a live camera nor open one alongside it.
    pub fn open_if_free(id: &str) -> Result<Self, String> {
        Self::open_claimed(id, Duration::ZERO)
    }

    fn open_claimed(id: &str, wait: Duration) -> Result<Self, String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let id_cstring = std::ffi::CString::new(id).map_err(|e| e.to_string())?;

        let (handle, lease) = claim_then_open(id, wait, || {
            let handle = {
                let _calls = device_table_calls();
                unsafe { sdk.api.OpenQHYCCD(id_cstring.as_ptr()) }
            };
            if handle.is_null() {
                Err(format!("Failed to open QHY camera {id}"))
            } else {
                Ok(handle)
            }
        })?;

        let camera = Self { handle, lease };
        // A failed init drops `camera`, which closes it through its lease.
        camera.init()?;
        Ok(camera)
    }

    pub fn init(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = {
            let _calls = device_table_calls();
            unsafe { sdk.api.InitQHYCCD(self.handle) }
        };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("InitQHYCCD failed: {}", res))
        }
    }

    pub fn close(&self) -> Result<(), String> {
        if !self.lease.begin_close() {
            return Ok(());
        }
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = {
            let _calls = device_table_calls();
            unsafe { sdk.api.CloseQHYCCD(self.handle) }
        };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("CloseQHYCCD failed: {}", res))
        }
    }

    pub fn chip_info(&self) -> Result<ChipInfo, String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let mut chip_w = 0.0;
        let mut chip_h = 0.0;
        let mut img_w = 0;
        let mut img_h = 0;
        let mut pixel_w = 0.0;
        let mut pixel_h = 0.0;
        let mut bpp = 0;

        let res = unsafe {
            sdk.api.GetQHYCCDChipInfo(
                self.handle,
                &mut chip_w,
                &mut chip_h,
                &mut img_w,
                &mut img_h,
                &mut pixel_w,
                &mut pixel_h,
                &mut bpp,
            )
        };

        if res != QHYCCD_SUCCESS {
            return Err(format!("GetQHYCCDChipInfo failed: {}", res));
        }

        // IsQHYCCDControlAvailable(CAM_COLOR) returns the Bayer pattern ID for color
        // cameras (1=BAYER_GB, 2=BAYER_GR, 3=BAYER_BG, 4=BAYER_RG) or QHYCCD_ERROR for mono.
        let cam_color_result = unsafe {
            sdk.api
                .IsQHYCCDControlAvailable(self.handle, ControlId::CamColor as u32)
        };

        let bayer = parse_bayer_id(cam_color_result);

        Ok(ChipInfo {
            chip_w,
            chip_h,
            img_w,
            img_h,
            pixel_w,
            pixel_h,
            bpp,
            bayer,
        })
    }

    pub fn is_control_available(&self, ctrl: ControlId) -> bool {
        if let Some(sdk) = QhySdk::try_load() {
            unsafe { sdk.api.IsQHYCCDControlAvailable(self.handle, ctrl as u32) == QHYCCD_SUCCESS }
        } else {
            false
        }
    }

    pub fn supported_bins(&self) -> Vec<u8> {
        get_supported_bins(|ctrl| self.is_control_available(ctrl))
    }

    pub fn set_param(&self, ctrl: ControlId, value: f64) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.SetQHYCCDParam(self.handle, ctrl as u32, value) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("SetQHYCCDParam {:?} failed: {}", ctrl, res))
        }
    }

    pub fn get_param(&self, ctrl: ControlId) -> Result<f64, String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let val = unsafe { sdk.api.GetQHYCCDParam(self.handle, ctrl as u32) };
        if val > 1_000_000_000.0 || val.is_nan() {
            Err(format!("GetQHYCCDParam returned invalid value: {}", val))
        } else {
            Ok(val)
        }
    }

    pub fn param_range(&self, ctrl: ControlId) -> Result<(f64, f64, f64), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let mut min = 0.0;
        let mut max = 0.0;
        let mut step = 0.0;
        let res = unsafe {
            sdk.api.GetQHYCCDParamMinMaxStep(
                self.handle,
                ctrl as u32,
                &mut min,
                &mut max,
                &mut step,
            )
        };
        if res == QHYCCD_SUCCESS {
            Ok((min, max, step))
        } else {
            Err(format!("GetQHYCCDParamMinMaxStep failed: {}", res))
        }
    }

    pub fn set_resolution(&self, x: u32, y: u32, w: u32, h: u32) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.SetQHYCCDResolution(self.handle, x, y, w, h) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("SetQHYCCDResolution failed: {}", res))
        }
    }

    pub fn set_bin(&self, bin: u32) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.SetQHYCCDBinMode(self.handle, bin, bin) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("SetQHYCCDBinMode failed: {}", res))
        }
    }

    pub fn set_bits(&self, bits: u32) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.SetQHYCCDBitsMode(self.handle, bits) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("SetQHYCCDBitsMode failed: {}", res))
        }
    }

    pub fn set_stream_mode(&self, mode: u8) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.SetQHYCCDStreamMode(self.handle, mode) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("SetQHYCCDStreamMode failed: {}", res))
        }
    }

    pub fn start_single_frame(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.ExpQHYCCDSingleFrame(self.handle) };
        if res == QHYCCD_SUCCESS || res == QHYCCD_READ_DIRECTLY {
            Ok(())
        } else {
            Err(format!("ExpQHYCCDSingleFrame failed: {}", res))
        }
    }

    pub fn get_single_frame(&self, buf: &mut [u8]) -> Result<(u32, u32), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let mut w = 0;
        let mut h = 0;
        let mut bpp = 0;
        let mut channels = 0;

        let res = unsafe {
            sdk.api.GetQHYCCDSingleFrame(
                self.handle,
                &mut w,
                &mut h,
                &mut bpp,
                &mut channels,
                buf.as_mut_ptr(),
            )
        };

        if res == QHYCCD_SUCCESS {
            Ok((w, h))
        } else if res == QHYCCD_READ_DIRECTLY {
            Err("QHYCCD_READ_DIRECTLY".to_string())
        } else if res == QHYCCD_ERROR {
            Err("QHYCCD_ERROR".to_string())
        } else {
            Err(format!("GetQHYCCDSingleFrame failed: {}", res))
        }
    }

    pub fn start_live(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.BeginQHYCCDLive(self.handle) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("BeginQHYCCDLive failed: {}", res))
        }
    }

    pub fn stop_live(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.StopQHYCCDLive(self.handle) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("StopQHYCCDLive failed: {}", res))
        }
    }

    pub fn get_live_frame(&self, buf: &mut [u8]) -> Result<(u32, u32), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let mut w = 0;
        let mut h = 0;
        let mut bpp = 0;
        let mut channels = 0;

        let res = unsafe {
            sdk.api.GetQHYCCDLiveFrame(
                self.handle,
                &mut w,
                &mut h,
                &mut bpp,
                &mut channels,
                buf.as_mut_ptr(),
            )
        };

        if res == QHYCCD_SUCCESS {
            Ok((w, h))
        } else if res == QHYCCD_READ_DIRECTLY || res == QHYCCD_ERROR {
            Err(res.to_string())
        } else {
            Err(format!("GetQHYCCDLiveFrame failed: {}", res))
        }
    }

    pub fn cancel(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        // CancelQHYCCDExposingAndReadout is safer as it stops both
        let res = unsafe { sdk.api.CancelQHYCCDExposingAndReadout(self.handle) };
        if res == QHYCCD_SUCCESS {
            Ok(())
        } else {
            Err(format!("CancelQHYCCDExposingAndReadout failed: {}", res))
        }
    }

    pub fn set_target_temperature(&self, temp_c: f64) -> Result<(), String> {
        self.set_param(ControlId::Cooler, temp_c)
    }

    pub fn current_temperature(&self) -> Result<f64, String> {
        self.get_param(ControlId::CurTemp)
    }

    pub fn cooler_power(&self) -> Result<f64, String> {
        self.get_param(ControlId::CurPWM)
    }
}

impl Drop for QhyHandle {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

pub fn scan_cameras() -> Option<Vec<String>> {
    let sdk = QhySdk::try_load()?;
    let _calls = device_table_calls();

    let count = unsafe { sdk.api.ScanQHYCCD() };
    if count == 0 {
        return Some(Vec::new());
    }

    let mut cameras = Vec::new();
    for i in 0..count {
        // `c_char`, not `i8`: it is `u8` on Linux ARM, the Raspberry Pi builds.
        let mut id_buf = [0 as c_char; 64];
        let res = unsafe { sdk.api.GetQHYCCDId(i, id_buf.as_mut_ptr()) };
        if res == QHYCCD_SUCCESS {
            let id = unsafe { CStr::from_ptr(id_buf.as_ptr()) }
                .to_string_lossy()
                .into_owned();
            cameras.push(id);
        }
    }

    Some(cameras)
}

pub fn parse_bayer_id(cam_color_result: u32) -> String {
    match cam_color_result {
        1 => "GBRG".to_string(),
        2 => "GRBG".to_string(),
        3 => "BGGR".to_string(),
        4 => "RGGB".to_string(),
        _ => "MONO".to_string(),
    }
}

pub fn get_supported_bins(is_available: impl Fn(ControlId) -> bool) -> Vec<u8> {
    let bin_controls = [
        (ControlId::CamBin1x1mode, 1u8),
        (ControlId::CamBin2x2mode, 2),
        (ControlId::CamBin3x3mode, 3),
        (ControlId::CamBin4x4mode, 4),
        (ControlId::CamBin6x6mode, 6),
        (ControlId::CamBin8x8mode, 8),
    ];

    let mut bins: Vec<u8> = bin_controls
        .iter()
        .filter(|(ctrl, _)| is_available(*ctrl))
        .map(|(_, bin)| *bin)
        .collect();

    if bins.is_empty() {
        bins.push(1);
    }
    bins
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bayer_id() {
        assert_eq!(parse_bayer_id(1), "GBRG");
        assert_eq!(parse_bayer_id(2), "GRBG");
        assert_eq!(parse_bayer_id(3), "BGGR");
        assert_eq!(parse_bayer_id(4), "RGGB");
        assert_eq!(parse_bayer_id(0), "MONO");
        assert_eq!(parse_bayer_id(999), "MONO");
    }

    #[test]
    fn test_get_supported_bins() {
        // Test all supported
        let bins = get_supported_bins(|_| true);
        assert_eq!(bins, vec![1, 2, 3, 4, 6, 8]);

        // Test none supported (should fallback to 1)
        let bins = get_supported_bins(|_| false);
        assert_eq!(bins, vec![1]);

        // Test only 1 and 2 supported
        let bins = get_supported_bins(|ctrl| {
            matches!(ctrl, ControlId::CamBin1x1mode | ControlId::CamBin2x2mode)
        });
        assert_eq!(bins, vec![1, 2]);
    }

    /// Discovery and a connect never open one device side by side: the claim is in place
    /// before the SDK sees the id.
    #[test]
    fn a_device_is_claimed_before_the_sdk_opens_it() {
        let id = "QHY-test-claimed-before-open";
        let (_, lease) = claim_then_open(id, Duration::ZERO, || {
            assert!(DeviceLease::is_device_open(PROVIDER, id));
            Ok::<_, String>(())
        })
        .unwrap();
        assert!(lease.begin_close());
        assert!(!DeviceLease::is_device_open(PROVIDER, id));
    }

    #[test]
    fn a_failed_open_gives_the_claim_back() {
        let id = "QHY-test-failed-open";
        let failed = claim_then_open(id, Duration::ZERO, || {
            Err::<(), _>("OpenQHYCCD returned null".to_string())
        });
        assert!(failed.is_err());
        assert!(!DeviceLease::is_device_open(PROVIDER, id));
    }

    /// A device another handle holds is never opened alongside it, however long the wait.
    #[test]
    fn a_held_device_is_never_opened_again() {
        let id = "QHY-test-held";
        let holder = DeviceLease::try_acquire_unique_device(PROVIDER, id).unwrap();
        let mut opened = false;
        let refused = claim_then_open(id, Duration::from_millis(60), || {
            opened = true;
            Ok::<_, String>(())
        });
        assert!(refused.is_err());
        assert!(!opened);
        assert!(holder.begin_close());
    }

    /// A connect waits out discovery's few seconds with the device.
    #[test]
    fn a_connect_waits_for_a_short_hold_to_end() {
        let id = "QHY-test-short-hold";
        let holder = DeviceLease::try_acquire_unique_device(PROVIDER, id).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            assert!(holder.begin_close());
        });
        let (_, lease) = claim_then_open(id, Duration::from_secs(2), || Ok::<_, String>(()))
            .expect("claimed once the holder let go");
        release.join().unwrap();
        assert!(lease.begin_close());
    }
}
