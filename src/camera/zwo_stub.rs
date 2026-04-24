//! Stub implementation when ZWO ASI SDK is not available

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::Frame;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};

/// ZWO camera provider (stub)
pub struct ZwoProvider;

impl ZwoProvider {
    /// Create a new ZWO provider
    pub fn new() -> Self {
        Self
    }
}

impl Default for ZwoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for ZwoProvider {
    fn name(&self) -> &'static str {
        "ZWO"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }
}

/// ZWO camera handle (stub)
pub struct ZwoCamera {
    _private: (),
}

impl ZwoCamera {
    /// Get the number of connected ZWO cameras
    pub fn camera_count() -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    /// List all connected cameras
    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    /// Open a camera by index
    pub fn open(_index: usize) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    /// Open a camera by name
    pub fn open_by_name(_name: &str) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }
}

impl Camera for ZwoCamera {
    fn info(&self) -> &CameraInfo {
        unreachable!("ZwoCamera stub should never be instantiated")
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn cancel(&self) {}

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn close(&mut self) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("ZWO".to_string()))
    }

    fn provider_name(&self) -> &'static str {
        "ZWO"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_not_available() {
        let provider = ZwoProvider::new();
        assert!(!provider.is_available());
        assert_eq!(provider.name(), "ZWO");
    }

    #[test]
    fn test_sdk_not_available_errors() {
        assert!(matches!(
            ZwoCamera::camera_count(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            ZwoCamera::list_cameras(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            ZwoCamera::open(0),
            Err(CameraError::SdkNotAvailable(_))
        ));
    }
}
