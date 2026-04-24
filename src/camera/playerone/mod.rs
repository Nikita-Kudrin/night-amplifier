//! Player One Astronomy camera implementation

pub mod ffi_types;
pub mod sdk;
pub mod shim;

use shim::{Camera as POACamera, CameraDescription};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};
use crate::ffi_safety::catch_ffi_panic;
use crate::Frame;

mod capture;
mod properties;
mod sensor_mode;

pub use properties::{camera_info_from_description, camera_info_from_properties};

/// Player One camera provider
pub struct PlayerOneProvider;

impl PlayerOneProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PlayerOneProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for PlayerOneProvider {
    fn name(&self) -> &'static str {
        "PlayerOne"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn camera_count(&self) -> CameraResult<usize> {
        catch_ffi_panic("PlayerOne::camera_count", || POACamera::all_cameras().len())
            .map_err(CameraError::from)
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        let descriptions = catch_ffi_panic("PlayerOne::list_cameras", POACamera::all_cameras)
            .map_err(CameraError::from)?;
        Ok(descriptions
            .iter()
            .map(camera_info_from_description)
            .collect())
    }

    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>> {
        let camera = PlayerOneCamera::open(index)?;
        Ok(Box::new(camera))
    }
}

/// Player One camera handle
pub struct PlayerOneCamera {
    camera: POACamera,
    info: CameraInfo,
    cancel_flag: Arc<AtomicBool>,
}

impl PlayerOneCamera {
    pub fn open(index: usize) -> CameraResult<Self> {
        let descriptions = catch_ffi_panic("PlayerOne::open::all_cameras", POACamera::all_cameras)
            .map_err(CameraError::from)?;
        if descriptions.is_empty() {
            return Err(CameraError::NoCamerasFound);
        }
        if index >= descriptions.len() {
            return Err(CameraError::InvalidCameraIndex {
                index,
                count: descriptions.len(),
            });
        }

        let description = descriptions.into_iter().nth(index).unwrap();
        let mut info = camera_info_from_description(&description);

        let camera = catch_ffi_panic("PlayerOne::open::description.open", || description.open())
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::OpenFailed(format!("{:?}", e)))?;

        info.sensor_modes = sensor_mode::list_sensor_modes(camera.id());
        info.has_dew_heater = camera.is_config_supported(ffi_types::POAConfig::POA_HEATER_POWER);

        Ok(Self {
            camera,
            info,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        })
    }
}

impl Camera for PlayerOneCamera {
    fn info(&self) -> &CameraInfo {
        &self.info
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Ok(GainPresets {
            highest_dr: 0,
            hcg: self.info.hcg_gain,
            unity: self.info.unity_gain,
            lowest_rn: self.info.max_gain,
            offset_highest_dr: 10,
            offset_hcg: 30,
            offset_unity: 20,
            offset_lowest_rn: 50,
        })
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        let temperature = catch_ffi_panic("PlayerOne::temperature", || self.camera.temperature())
            .map_err(CameraError::from)?
            .map(|t| t as f64 / 10.0)
            .unwrap_or(0.0);

        let current_gain = catch_ffi_panic("PlayerOne::gain", || self.camera.gain())
            .map_err(CameraError::from)?
            .map(|(v, _)| v as i32)
            .unwrap_or(0);

        let current_offset = catch_ffi_panic("PlayerOne::offset", || self.camera.offset())
            .map_err(CameraError::from)?
            .map(|v| v as i32)
            .unwrap_or(0);

        let current_exposure_us = catch_ffi_panic("PlayerOne::exposure", || self.camera.exposure())
            .map_err(CameraError::from)?
            .map(|(v, _)| v as u64)
            .unwrap_or(0);

        let cooler_power = if self.info.has_cooler {
            catch_ffi_panic("PlayerOne::cooler_power", || self.camera.cooler_power())
                .map_err(CameraError::from)?
                .ok()
                .map(|p| p as f64)
        } else {
            None
        };

        let cooler_on = if self.info.has_cooler {
            catch_ffi_panic("PlayerOne::cooler", || self.camera.cooler())
                .map_err(CameraError::from)?
                .unwrap_or(false)
        } else {
            false
        };

        let dew_heater_on = if self.info.has_dew_heater {
            catch_ffi_panic("PlayerOne::dew_heater_power", || {
                self.camera.dew_heater_power()
            })
            .map_err(CameraError::from)?
            .map(|p| p > 0)
            .unwrap_or(false)
        } else {
            false
        };

        Ok(CameraStatus {
            temperature_c: temperature,
            cooler_power,
            cooler_on,
            is_exposing: false,
            current_gain,
            current_offset,
            current_exposure_us,
            dew_heater_on,
        })
    }

    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("PlayerOne::set_target_temperature", || {
            self.camera.set_target_temperature(temp_c as i64)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))
    }

    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()> {
        if !self.info.has_cooler {
            return Err(CameraError::ParameterNotSupported("cooler".to_string()));
        }
        catch_ffi_panic("PlayerOne::set_cooler", || self.camera.set_cooler(enabled))
            .map_err(CameraError::from)?
            .map_err(|e| CameraError::CoolingFailed(format!("{:?}", e)))
    }

    fn set_dew_heater(&mut self, enabled: bool, power: i32) -> CameraResult<()> {
        if !self.info.has_dew_heater {
            return Err(CameraError::ParameterNotSupported("dew_heater".to_string()));
        }
        let power_val = if enabled { power as i64 } else { 0 };
        catch_ffi_panic("PlayerOne::set_dew_heater_power", || {
            self.camera.set_dew_heater_power(power_val)
        })
        .map_err(CameraError::from)?
        .map_err(|e| CameraError::ParameterNotSupported(format!("dew_heater: {:?}", e)))
    }

    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame> {
        config.validate(&self.info)?;
        capture::apply_config(&mut self.camera, config, &self.info)?;
        capture::run_capture(&mut self.camera, &self.info, config, &self.cancel_flag)
    }

    fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn close(&mut self) -> CameraResult<()> {
        Ok(())
    }

    fn provider_name(&self) -> &'static str {
        "PlayerOne"
    }
}
