use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets, RawFrame};

pub struct SvbonyProvider;

impl SvbonyProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SvbonyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for SvbonyProvider {
    fn name(&self) -> &'static str {
        "SVBony"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Ok(0)
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Ok(Vec::new())
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::ProviderNotFound(
            "SVBony feature not compiled".into(),
        ))
    }
}

pub struct SvbonyCamera;

impl Camera for SvbonyCamera {
    fn info(&self) -> &CameraInfo {
        unimplemented!()
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        unimplemented!()
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        unimplemented!()
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        unimplemented!()
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        unimplemented!()
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        unimplemented!()
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<RawFrame> {
        unimplemented!()
    }

    fn cancel(&self) {
        unimplemented!()
    }

    fn cancel_token(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        unimplemented!()
    }

    fn close(&mut self) -> CameraResult<()> {
        unimplemented!()
    }

    fn provider_name(&self) -> &'static str {
        "SVBony"
    }
}
