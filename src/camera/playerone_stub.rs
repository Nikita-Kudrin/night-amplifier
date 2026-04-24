//! Stub implementation when Player One SDK is not available

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::Frame;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};

/// Player One camera provider (stub)
pub struct PlayerOneProvider;

impl PlayerOneProvider {
    /// Create a new Player One provider
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
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }
}

/// Player One camera handle (stub)
pub struct PlayerOneCamera {
    _private: (),
}

impl PlayerOneCamera {
    /// Get the number of connected Player One cameras
    pub fn camera_count() -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    /// List all connected cameras
    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    /// Open a camera by index
    pub fn open(_index: usize) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    /// Open a camera by name
    pub fn open_by_name(_name: &str) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }
}

impl Camera for PlayerOneCamera {
    fn info(&self) -> &CameraInfo {
        unreachable!("PlayerOneCamera stub should never be instantiated")
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Ok(())
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Ok(())
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn cancel(&self) {}

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn close(&mut self) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("PlayerOne".to_string()))
    }

    fn provider_name(&self) -> &'static str {
        "PlayerOne"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_not_available() {
        let provider = PlayerOneProvider::new();
        assert!(!provider.is_available());
        assert_eq!(provider.name(), "PlayerOne");
    }

    #[test]
    fn test_sdk_not_available_errors() {
        assert!(matches!(
            PlayerOneCamera::camera_count(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            PlayerOneCamera::list_cameras(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            PlayerOneCamera::open(0),
            Err(CameraError::SdkNotAvailable(_))
        ));
    }
}
