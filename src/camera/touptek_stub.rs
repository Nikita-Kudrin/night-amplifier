//! Stub implementation when ToupTek SDK is not available

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::Frame;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};

/// ToupTek camera provider (stub)
pub struct TouptekProvider;

impl TouptekProvider {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TouptekProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for TouptekProvider {
    fn name(&self) -> &'static str {
        "ToupTek"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }
}

/// ToupTek camera handle (stub)
pub struct TouptekCamera {
    _private: (),
}

impl TouptekCamera {
    pub fn camera_count() -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    pub fn open(_index: usize) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    pub fn open_by_name(_name: &str) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }
}

impl Camera for TouptekCamera {
    fn info(&self) -> &CameraInfo {
        unreachable!("TouptekCamera stub should never be instantiated")
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn cancel(&self) {}

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn close(&mut self) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ToupTek".to_string()))
    }

    fn provider_name(&self) -> &'static str {
        "ToupTek"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_not_available() {
        let provider = TouptekProvider::new();
        assert!(!provider.is_available());
        assert_eq!(provider.name(), "ToupTek");
    }

    #[test]
    fn test_sdk_not_available_errors() {
        assert!(matches!(
            TouptekCamera::camera_count(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            TouptekCamera::list_cameras(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            TouptekCamera::open(0),
            Err(CameraError::SdkNotAvailable(_))
        ));
    }
}
