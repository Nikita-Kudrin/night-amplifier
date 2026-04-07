//! Camera abstraction traits
//!
//! These traits define the common interface for all camera implementations.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::Frame;

use super::error::CameraResult;
use super::types::{CameraInfo, CameraStatus, CaptureConfig, GainPresets};

/// Core trait for camera operations
///
/// This trait defines the common interface for all astronomy cameras,
/// regardless of manufacturer. Implementations handle the specifics
/// of each SDK.
///
/// # Example
///
/// ```no_run
/// use night_amplifier::camera::{Camera, CaptureConfig};
///
/// fn capture_image(camera: &mut dyn Camera) -> night_amplifier::camera::CameraResult<night_amplifier::Frame> {
///     let config = CaptureConfig::default()
///         .with_exposure_us(2_000_000)
///         .with_gain(100);
///     camera.capture(&config)
/// }
/// ```
pub trait Camera: Send {
    /// Get camera information
    fn info(&self) -> &CameraInfo;

    /// Get gain presets (optimal gain values for different scenarios)
    ///
    /// Returns preset gain values for:
    /// - Highest dynamic range
    /// - HCG (High Conversion Gain) mode
    /// - Unity gain (e/ADU = 1)
    /// - Lowest read noise
    fn gain_presets(&self) -> CameraResult<GainPresets>;

    /// Get current camera status
    ///
    /// Returns real-time status including:
    /// - Sensor temperature
    /// - Cooler power and state
    /// - Current gain/offset/exposure settings
    /// - Exposure state
    fn status(&self) -> CameraResult<CameraStatus>;

    /// Set target cooling temperature
    ///
    /// # Errors
    /// Returns `CameraError::ParameterNotSupported` if camera has no cooler
    fn set_target_temperature(&mut self, temp_c: f64) -> CameraResult<()>;

    /// Enable or disable cooler
    ///
    /// # Errors
    /// Returns `CameraError::ParameterNotSupported` if camera has no cooler
    fn set_cooler(&mut self, enabled: bool) -> CameraResult<()>;

    /// Capture an image with the given configuration
    ///
    /// This method blocks until the exposure is complete or times out.
    /// Use `cancel()` from another thread to abort a long exposure.
    ///
    /// # Arguments
    /// * `config` - Capture configuration (exposure, gain, binning, etc.)
    ///
    /// # Returns
    /// The captured image as a `Frame`
    ///
    /// # Errors
    /// - `CameraError::ExposureTimeout` - Exposure exceeded timeout
    /// - `CameraError::Cancelled` - Exposure was cancelled
    /// - `CameraError::Disconnected` - Camera was disconnected
    fn capture(&mut self, config: &CaptureConfig) -> CameraResult<Frame>;

    /// Cancel an ongoing exposure
    ///
    /// This is safe to call from another thread while `capture()` is blocking.
    fn cancel(&self);

    /// Get a cancellation token for this camera
    ///
    /// The returned `AtomicBool` can be used to cancel exposures from
    /// another thread by setting it to `true`.
    fn cancel_token(&self) -> Arc<AtomicBool>;

    /// Close the camera connection
    ///
    /// This is called automatically when the camera is dropped, but
    /// can be called explicitly for error handling.
    fn close(&mut self) -> CameraResult<()>;

    /// Get the provider name for this camera
    fn provider_name(&self) -> &'static str;
}

/// Factory trait for discovering and opening cameras
///
/// Each camera manufacturer implements this trait to provide
/// camera discovery and instantiation.
///
/// # Example
///
/// ```no_run
/// use night_amplifier::camera::{CameraProvider, PlayerOneProvider};
///
/// let provider = PlayerOneProvider::new();
/// let cameras = provider.list_cameras()?;
/// if !cameras.is_empty() {
///     let mut camera = provider.open(0)?;
///     // Use camera...
/// }
/// # Ok::<(), night_amplifier::camera::CameraError>(())
/// ```
pub trait CameraProvider: Send + Sync {
    /// Get the provider name (e.g., "PlayerOne", "ZWO", "SVBony")
    fn name(&self) -> &'static str;

    /// Check if the SDK for this provider is available
    ///
    /// Returns `false` if the required feature is not enabled or
    /// the SDK libraries are not installed.
    fn is_available(&self) -> bool;

    /// Get the number of connected cameras
    fn camera_count(&self) -> CameraResult<usize>;

    /// List all connected cameras
    fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>>;

    /// Open a camera by index
    fn open(&self, index: usize) -> CameraResult<Box<dyn Camera>>;

    /// Open a camera by name (partial match)
    fn open_by_name(&self, name: &str) -> CameraResult<Box<dyn Camera>> {
        let cameras = self.list_cameras()?;
        if cameras.is_empty() {
            return Err(super::error::CameraError::NoCamerasFound);
        }

        for (index, info) in cameras.iter().enumerate() {
            if info.name.contains(name) {
                return self.open(index);
            }
        }

        Err(super::error::CameraError::OpenFailed(format!(
            "Camera '{}' not found",
            name
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::types::SensorType;

    // Mock camera for testing
    struct MockCamera {
        info: CameraInfo,
        cancel_flag: Arc<AtomicBool>,
    }

    impl MockCamera {
        fn new() -> Self {
            Self {
                info: CameraInfo {
                    name: "Mock Camera".to_string(),
                    id: 0,
                    max_width: 1920,
                    max_height: 1080,
                    sensor_type: SensorType::Mono,
                    ..Default::default()
                },
                cancel_flag: Arc::new(AtomicBool::new(false)),
            }
        }
    }

    impl Camera for MockCamera {
        fn info(&self) -> &CameraInfo {
            &self.info
        }

        fn gain_presets(&self) -> CameraResult<GainPresets> {
            Ok(GainPresets::default())
        }

        fn status(&self) -> CameraResult<CameraStatus> {
            Ok(CameraStatus::default())
        }

        fn set_target_temperature(&mut self, _temp_c: f64) -> CameraResult<()> {
            Err(super::super::error::CameraError::ParameterNotSupported(
                "cooler".to_string(),
            ))
        }

        fn set_cooler(&mut self, _enabled: bool) -> CameraResult<()> {
            Err(super::super::error::CameraError::ParameterNotSupported(
                "cooler".to_string(),
            ))
        }

        fn capture(&mut self, _config: &CaptureConfig) -> CameraResult<Frame> {
            Frame::zeros(
                self.info.max_width as usize,
                self.info.max_height as usize,
                1,
            )
            .map_err(|e| super::super::error::CameraError::ImageReadFailed(e.to_string()))
        }

        fn cancel(&self) {
            self.cancel_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }

        fn cancel_token(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.cancel_flag)
        }

        fn close(&mut self) -> CameraResult<()> {
            Ok(())
        }

        fn provider_name(&self) -> &'static str {
            "Mock"
        }
    }

    #[test]
    fn test_mock_camera_trait() {
        let mut camera = MockCamera::new();
        assert_eq!(camera.info().name, "Mock Camera");
        assert_eq!(camera.provider_name(), "Mock");
        assert!(camera.gain_presets().is_ok());
        assert!(camera.status().is_ok());
        assert!(camera.close().is_ok());
    }

    #[test]
    fn test_camera_capture() {
        let mut camera = MockCamera::new();
        let config = CaptureConfig::default();
        let frame = camera.capture(&config);
        assert!(frame.is_ok());
        let frame = frame.unwrap();
        assert_eq!(frame.width(), 1920);
        assert_eq!(frame.height(), 1080);
    }

    #[test]
    fn test_cancel_token() {
        let camera = MockCamera::new();
        let token = camera.cancel_token();
        assert!(!token.load(std::sync::atomic::Ordering::SeqCst));
        camera.cancel();
        assert!(token.load(std::sync::atomic::Ordering::SeqCst));
    }
}
