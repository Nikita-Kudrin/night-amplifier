use std::ffi::CStr;
use std::os::raw::c_char;
use tracing::warn;

use super::ffi_types::*;
use super::sdk::QhySdk;

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
}

// SAFETY: QHY SDK handles are bound to a single device. All access to
// QhyHandle goes through AppState's StdMutex, which serializes calls.
unsafe impl Send for QhyHandle {}
unsafe impl Sync for QhyHandle {}

impl QhyHandle {
    pub fn open(id: &str) -> Result<Self, String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;

        let id_cstring = std::ffi::CString::new(id).map_err(|e| e.to_string())?;
        let handle = unsafe { sdk.api.OpenQHYCCD(id_cstring.as_ptr()) };

        if handle.is_null() {
            return Err(format!("Failed to open QHY camera {}", id));
        }

        let res = unsafe { sdk.api.InitQHYCCD(handle) };
        if res != QHYCCD_SUCCESS {
            unsafe { sdk.api.CloseQHYCCD(handle) };
            return Err(format!("InitQHYCCD failed with code {}", res));
        }

        Ok(Self { handle })
    }

    pub fn close(&self) -> Result<(), String> {
        let sdk = QhySdk::try_load().ok_or("QHY SDK not loaded")?;
        let res = unsafe { sdk.api.CloseQHYCCD(self.handle) };
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
            sdk.api.IsQHYCCDControlAvailable(self.handle, ControlId::CamColor as u32)
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
            sdk.api
                .GetQHYCCDParamMinMaxStep(self.handle, ctrl as u32, &mut min, &mut max, &mut step)
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

    let count = unsafe { sdk.api.ScanQHYCCD() };
    if count == 0 {
        return Some(Vec::new());
    }

    let mut cameras = Vec::new();
    for i in 0..count {
        let mut id_buf = [0i8; 64];
        let res = unsafe { sdk.api.GetQHYCCDId(i, id_buf.as_mut_ptr() as *mut c_char) };
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
}
