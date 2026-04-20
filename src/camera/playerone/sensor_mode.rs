//! Player One sensor mode (Dual Sampling) support.
//!
//! The safe `playerone-sdk` crate does not expose sensor-mode APIs, so this
//! module wraps the raw `playerone-sdk-sys` bindings behind `catch_ffi_panic`.
//!
//! Cameras that support dual sampling (e.g., Uranus-C Pro) advertise multiple
//! modes, typically "Normal" (higher FPS) and "LRN" (Low Readout Noise).
//!
//! TODO: this whole module is a temporary workaround. Once
//! <https://github.com/Uriopass/playerone-sdk-rs> ships the sensor-mode API
//! in the safe wrapper (expected in `playerone-sdk >= 0.3.0`), delete this file,
//! drop the `playerone-sdk-sys` direct dependency from Cargo.toml, and replace
//! the callers in `playerone/mod.rs` and `playerone/capture.rs` with direct
//! calls on `playerone_sdk::Camera` (roughly: `camera.sensor_modes()`,
//! `camera.set_sensor_mode(index)`).

use std::os::raw::c_int;

use playerone_sdk_sys::{
    POAErrors, POAGetSensorMode, POAGetSensorModeCount, POAGetSensorModeInfo, POASensorModeInfo,
    POASetSensorMode,
};
use tracing::warn;

use crate::camera::types::{DualSamplingMode, SensorMode};
use crate::ffi_safety::catch_ffi_panic;

/// Enumerate all sensor modes advertised by the camera. Returns an empty vector
/// when the camera does not support mode selection or when any FFI call fails.
pub fn list_sensor_modes(camera_id: i32) -> Vec<SensorMode> {
    let count_result = catch_ffi_panic("PlayerOne::POAGetSensorModeCount", || {
        let mut count: c_int = 0;
        let err = unsafe { POAGetSensorModeCount(camera_id, &mut count) };
        (err, count)
    });

    let count = match count_result {
        Ok((POAErrors::POA_OK, n)) if n > 0 => n,
        Ok((POAErrors::POA_OK, _)) => return Vec::new(),
        Ok((POAErrors::POA_ERROR_ACCESS_DENIED, _)) => return Vec::new(),
        Ok((err, _)) => {
            warn!(?err, camera_id, "POAGetSensorModeCount returned error");
            return Vec::new();
        }
        Err(e) => {
            warn!(%e, camera_id, "Panic in POAGetSensorModeCount");
            return Vec::new();
        }
    };

    (0..count)
        .filter_map(|index| fetch_sensor_mode_info(camera_id, index))
        .collect()
}

fn fetch_sensor_mode_info(camera_id: i32, index: c_int) -> Option<SensorMode> {
    let info_result = catch_ffi_panic("PlayerOne::POAGetSensorModeInfo", || {
        let mut info = POASensorModeInfo::default();
        let err = unsafe { POAGetSensorModeInfo(camera_id, index, &mut info) };
        (err, info)
    });

    match info_result {
        Ok((POAErrors::POA_OK, info)) => Some(SensorMode {
            index: index as u32,
            name: cstr_from_bytes(&info.name),
            description: cstr_from_bytes(&info.desc),
        }),
        Ok((err, _)) => {
            warn!(?err, camera_id, index, "POAGetSensorModeInfo returned error");
            None
        }
        Err(e) => {
            warn!(%e, camera_id, index, "Panic in POAGetSensorModeInfo");
            None
        }
    }
}

/// Apply a sensor mode index. Must not be called while an exposure is in progress.
pub fn set_sensor_mode(camera_id: i32, index: u32) -> Result<(), POAErrors> {
    let result = catch_ffi_panic("PlayerOne::POASetSensorMode", || unsafe {
        POASetSensorMode(camera_id, index as c_int)
    });
    match result {
        Ok(POAErrors::POA_OK) => Ok(()),
        Ok(err) => Err(err),
        Err(e) => {
            warn!(%e, camera_id, index, "Panic in POASetSensorMode");
            Err(POAErrors::POA_ERROR_OPERATION_FAILED)
        }
    }
}

/// Read the currently active sensor mode index. Returns `None` when unsupported
/// or when the FFI call fails.
#[allow(dead_code)]
pub fn current_sensor_mode(camera_id: i32) -> Option<u32> {
    let result = catch_ffi_panic("PlayerOne::POAGetSensorMode", || {
        let mut idx: c_int = 0;
        let err = unsafe { POAGetSensorMode(camera_id, &mut idx) };
        (err, idx)
    });
    match result {
        Ok((POAErrors::POA_OK, idx)) if idx >= 0 => Some(idx as u32),
        _ => None,
    }
}

/// Resolve a desired `DualSamplingMode` to a concrete mode index by matching on
/// the camera's reported mode names. Returns `None` if no compatible mode is found.
pub fn resolve_mode_index(modes: &[SensorMode], desired: DualSamplingMode) -> Option<u32> {
    let keywords: &[&str] = match desired {
        DualSamplingMode::LowReadoutNoise => &["lrn", "low readout", "low read"],
        DualSamplingMode::Normal => &["normal"],
    };
    modes
        .iter()
        .find(|m| {
            let name = m.name.to_lowercase();
            keywords.iter().any(|k| name.contains(k))
        })
        .map(|m| m.index)
}

fn cstr_from_bytes(buf: &[std::os::raw::c_char]) -> String {
    let bytes: Vec<u8> = buf
        .iter()
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(index: u32, name: &str) -> SensorMode {
        SensorMode {
            index,
            name: name.to_string(),
            description: String::new(),
        }
    }

    #[test]
    fn resolves_lrn_by_acronym() {
        let modes = vec![mode(0, "Normal"), mode(1, "LRN")];
        assert_eq!(
            resolve_mode_index(&modes, DualSamplingMode::LowReadoutNoise),
            Some(1)
        );
    }

    #[test]
    fn resolves_lrn_case_insensitive() {
        let modes = vec![mode(0, "normal"), mode(1, "Low Readout Noise")];
        assert_eq!(
            resolve_mode_index(&modes, DualSamplingMode::LowReadoutNoise),
            Some(1)
        );
    }

    #[test]
    fn resolves_normal_mode() {
        let modes = vec![mode(0, "Normal"), mode(1, "LRN")];
        assert_eq!(
            resolve_mode_index(&modes, DualSamplingMode::Normal),
            Some(0)
        );
    }

    #[test]
    fn returns_none_when_no_match() {
        let modes = vec![mode(0, "HDR"), mode(1, "HighGain")];
        assert_eq!(
            resolve_mode_index(&modes, DualSamplingMode::LowReadoutNoise),
            None
        );
    }

    #[test]
    fn returns_none_on_empty_mode_list() {
        let modes: Vec<SensorMode> = Vec::new();
        assert_eq!(
            resolve_mode_index(&modes, DualSamplingMode::Normal),
            None
        );
    }
}
