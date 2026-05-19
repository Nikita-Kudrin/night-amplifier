//! Stub implementation when QHYCCD SDK is not available

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::Frame;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};

/// QHY camera provider (stub)
pub struct QhyProvider;

impl QhyProvider {
    /// Create a new QHY provider
    pub fn new() -> Self {
        Self
    }
}

impl Default for QhyProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraProvider for QhyProvider {
    fn name(&self) -> &'static str {
        "QHY"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }
}

/// QHY camera handle (stub)
pub struct QhyCamera {
    _private: (),
}

impl QhyCamera {
    /// Get the number of connected QHY cameras
    pub fn camera_count() -> CameraResult<usize> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    /// List all connected cameras
    pub fn list_cameras() -> CameraResult<Vec<CameraInfo>> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    /// Open a camera by index
    pub fn open(_index: usize) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    /// Open a camera by name
    pub fn open_by_name(_name: &str) -> CameraResult<Self> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }
}

impl Camera for QhyCamera {
    fn info(&self) -> &CameraInfo {
        unreachable!("QhyCamera stub should never be instantiated")
    }

    fn gain_presets(&self) -> CameraResult<GainPresets> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn status(&self) -> CameraResult<CameraStatus> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn set_dew_heater(&mut self, _enabled: bool, _power: i32) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn cancel(&self) {}

    fn cancel_token(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn close(&mut self) -> CameraResult<()> {
        Err(CameraError::SdkNotAvailable("QHY".to_string()))
    }

    fn provider_name(&self) -> &'static str {
        "QHY"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_not_available() {
        let provider = QhyProvider::new();
        assert!(!provider.is_available());
        assert_eq!(provider.name(), "QHY");
    }

    #[test]
    fn test_sdk_not_available_errors() {
        assert!(matches!(
            QhyCamera::camera_count(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            QhyCamera::list_cameras(),
            Err(CameraError::SdkNotAvailable(_))
        ));
        assert!(matches!(
            QhyCamera::open(0),
            Err(CameraError::SdkNotAvailable(_))
        ));
    }
}
