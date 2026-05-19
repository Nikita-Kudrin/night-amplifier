//! Stub implementation for INDI when the feature is disabled

use crate::camera::{Camera, CameraError, CameraInfo, CameraProvider, CameraResult};

pub struct IndiProvider {
    pub host: String,
    pub port: u16,
}

impl IndiProvider {
    pub fn new(host: String, port: u16) -> Self {
        Self { host, port }
    }

    pub async fn list_cameras_async(&self) -> CameraResult<Vec<CameraInfo>> {
        Ok(vec![])
    }
}

impl CameraProvider for IndiProvider {
    fn name(&self) -> &'static str {
        "indi"
    }

    fn is_available(&self) -> bool {
        false
    }

    fn camera_count(&self) -> CameraResult<usize> {
        Ok(0)
    }

    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
        Ok(vec![])
    }

    fn open(&self, _index: usize) -> CameraResult<Box<dyn Camera>> {
        Err(CameraError::SdkNotAvailable("INDI".to_string()))
    }
}

pub struct IndiCamera;

// Note: We don't implement the Camera trait for IndiCamera stub because
// it's never instantiated when the feature is disabled.
