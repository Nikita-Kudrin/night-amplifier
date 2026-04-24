//! Camera registry for managing multiple providers

use std::collections::HashMap;

use super::error::{CameraError, CameraResult};
use super::traits::{Camera, CameraProvider};
use super::types::CameraInfo;

/// Entry in the camera list with provider information
#[derive(Debug, Clone)]
pub struct CameraEntry {
    /// Provider name (e.g., "PlayerOne", "ZWO")
    pub provider: String,
    /// Index within the provider
    pub index: usize,
    /// Camera information
    pub info: CameraInfo,
}

/// Registry for managing multiple camera providers
///
/// The registry allows registering multiple camera providers (Player One, ZWO,
/// SVBony, etc.) and provides a unified interface for discovering and opening
/// cameras from any provider.
///
/// # Example
///
/// ```no_run
/// use night_amplifier::camera::{CameraRegistry, CaptureConfig};
///
/// let mut registry = CameraRegistry::new();
/// registry.register_defaults();
///
/// // List all cameras from all providers
/// for entry in registry.list_all_cameras()? {
///     println!("{}: {} ({}x{})",
///              entry.provider, entry.info.name,
///              entry.info.max_width, entry.info.max_height);
/// }
///
/// // Open first camera from a specific provider
/// let mut camera = registry.open_camera("PlayerOne", 0)?;
///
/// // Or open by unique identifier
/// let mut camera = registry.open_by_id("PlayerOne:0")?;
/// # Ok::<(), night_amplifier::camera::CameraError>(())
/// ```
pub struct CameraRegistry {
    providers: HashMap<String, Box<dyn CameraProvider>>,
}

impl CameraRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Register a camera provider
    ///
    /// # Errors
    /// Returns `CameraError::ProviderAlreadyRegistered` if a provider with
    /// the same name is already registered.
    pub fn register<P: CameraProvider + 'static>(&mut self, provider: P) -> CameraResult<()> {
        let name = provider.name().to_string();
        if self.providers.contains_key(&name) {
            return Err(CameraError::ProviderAlreadyRegistered(name));
        }
        self.providers.insert(name, Box::new(provider));
        Ok(())
    }

    /// Register all available default providers
    ///
    /// This registers all camera providers that are compiled in (based on
    /// enabled features). Providers whose SDK is not available will still
    /// be registered but will return errors when used.
    pub fn register_defaults(&mut self) {
        // Register Player One provider
        let _ = self.register(super::PlayerOneProvider::new());

        // Register ZWO provider
        let _ = self.register(super::ZwoProvider::new());

        // Register simulated camera provider (always available)
        let _ = self.register(super::SimulatedProvider::new());

        // Future providers will be added here:
        // let _ = self.register(super::SvbonyProvider::new());
        // let _ = self.register(super::TouptekProvider::new());
        // let _ = self.register(super::QhyccdProvider::new());
    }

    /// Get a list of registered provider names
    pub fn providers(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }

    /// Get a specific provider by name
    pub fn get_provider(&self, name: &str) -> Option<&dyn CameraProvider> {
        self.providers.get(name).map(|p| p.as_ref())
    }

    /// Check if a provider is available (SDK loaded)
    pub fn is_provider_available(&self, name: &str) -> bool {
        self.providers
            .get(name)
            .map(|p| p.is_available())
            .unwrap_or(false)
    }

    /// List cameras from a specific provider
    pub fn list_cameras(&self, provider: &str) -> CameraResult<Vec<CameraInfo>> {
        let provider = self
            .providers
            .get(provider)
            .ok_or_else(|| CameraError::ProviderNotFound(provider.to_string()))?;
        provider.list_cameras()
    }

    /// List all cameras from all providers
    ///
    /// Returns a list of `CameraEntry` structs containing the provider name,
    /// index, and camera info for each discovered camera.
    pub fn list_all_cameras(&self) -> CameraResult<Vec<CameraEntry>> {
        let mut all_cameras = Vec::new();

        for (name, provider) in &self.providers {
            if !provider.is_available() {
                continue;
            }

            match provider.list_cameras() {
                Ok(cameras) => {
                    for (index, info) in cameras.into_iter().enumerate() {
                        all_cameras.push(CameraEntry {
                            provider: name.clone(),
                            index,
                            info,
                        });
                    }
                }
                Err(_) => continue, // Skip providers that fail to enumerate
            }
        }

        Ok(all_cameras)
    }

    /// Get total camera count across all providers
    pub fn total_camera_count(&self) -> usize {
        self.providers
            .values()
            .filter(|p| p.is_available())
            .filter_map(|p| p.camera_count().ok())
            .sum()
    }

    /// Open a camera from a specific provider by index
    pub fn open_camera(&self, provider: &str, index: usize) -> CameraResult<Box<dyn Camera>> {
        let provider = self
            .providers
            .get(provider)
            .ok_or_else(|| CameraError::ProviderNotFound(provider.to_string()))?;
        provider.open(index)
    }

    /// Open a camera by unique identifier
    ///
    /// The identifier format is "Provider:Index", e.g., "PlayerOne:0" or "ZWO:1".
    pub fn open_by_id(&self, id: &str) -> CameraResult<Box<dyn Camera>> {
        let parts: Vec<&str> = id.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err(CameraError::OpenFailed(format!(
                "Invalid camera ID '{}'. Expected format: 'Provider:Index'",
                id
            )));
        }

        let provider_name = parts[0];
        let index: usize = parts[1].parse().map_err(|_| {
            CameraError::OpenFailed(format!(
                "Invalid camera index '{}' in ID '{}'",
                parts[1], id
            ))
        })?;

        self.open_camera(provider_name, index)
    }

    /// Open a camera by name (searches all providers)
    ///
    /// Searches all available providers for a camera whose name contains
    /// the given substring.
    pub fn open_by_name(&self, name: &str) -> CameraResult<Box<dyn Camera>> {
        for (_, provider) in &self.providers {
            if !provider.is_available() {
                continue;
            }

            if let Ok(camera) = provider.open_by_name(name) {
                return Ok(camera);
            }
        }

        Err(CameraError::OpenFailed(format!(
            "Camera '{}' not found in any provider",
            name
        )))
    }

    /// Open the first available camera from any provider
    pub fn open_first(&self) -> CameraResult<Box<dyn Camera>> {
        for (_, provider) in &self.providers {
            if !provider.is_available() {
                continue;
            }

            if let Ok(count) = provider.camera_count() {
                if count > 0 {
                    return provider.open(0);
                }
            }
        }

        Err(CameraError::NoCamerasFound)
    }
}

impl Default for CameraRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::error::CameraError;
    use crate::camera::traits::CameraProvider;
    use crate::camera::types::{CameraInfo, SensorType};

    // Mock provider for testing
    struct MockProvider {
        cameras: Vec<CameraInfo>,
    }

    impl MockProvider {
        fn new(count: usize) -> Self {
            let cameras = (0..count)
                .map(|i| CameraInfo {
                    name: format!("Mock Camera {}", i),
                    id: i as i32,
                    max_width: 1920,
                    max_height: 1080,
                    sensor_type: SensorType::Mono,
                    ..Default::default()
                })
                .collect();
            Self { cameras }
        }
    }

    impl CameraProvider for MockProvider {
        fn name(&self) -> &'static str {
            "Mock"
        }

        fn is_available(&self) -> bool {
            true
        }

        fn camera_count(&self) -> CameraResult<usize> {
            Ok(self.cameras.len())
        }

        fn list_cameras(&self) -> CameraResult<Vec<CameraInfo>> {
            Ok(self.cameras.clone())
        }

        fn open(&self, index: usize) -> CameraResult<Box<dyn super::super::traits::Camera>> {
            if index >= self.cameras.len() {
                return Err(CameraError::InvalidCameraIndex {
                    index,
                    count: self.cameras.len(),
                });
            }
            // Return a mock camera
            Ok(Box::new(MockCamera::new(&self.cameras[index])))
        }
    }

    // Simple mock camera
    struct MockCamera {
        info: CameraInfo,
        cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl MockCamera {
        fn new(info: &CameraInfo) -> Self {
            Self {
                info: info.clone(),
                cancel_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            }
        }
    }

    impl super::super::traits::Camera for MockCamera {
        fn info(&self) -> &CameraInfo {
            &self.info
        }
        fn gain_presets(&self) -> CameraResult<super::super::types::GainPresets> {
            Ok(Default::default())
        }
        fn status(&self) -> CameraResult<super::super::types::CameraStatus> {
            Ok(Default::default())
        }
        fn set_target_temperature(&mut self, _: f64) -> CameraResult<()> {
            Ok(())
        }
        fn set_cooler(&mut self, _: bool) -> CameraResult<()> {
            Ok(())
        }
        fn set_dew_heater(&mut self, _: bool, _: i32) -> CameraResult<()> {
            Ok(())
        }
        fn capture(
            &mut self,
            _: &super::super::types::CaptureConfig,
        ) -> CameraResult<crate::Frame> {
            crate::Frame::zeros(100, 100, 1)
                .map_err(|e| CameraError::ImageReadFailed(e.to_string()))
        }
        fn cancel(&self) {}
        fn cancel_token(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
            self.cancel_flag.clone()
        }
        fn close(&mut self) -> CameraResult<()> {
            Ok(())
        }
        fn provider_name(&self) -> &'static str {
            "Mock"
        }
    }

    #[test]
    fn test_registry_new() {
        let registry = CameraRegistry::new();
        assert!(registry.providers().is_empty());
    }

    #[test]
    fn test_registry_register_provider() {
        let mut registry = CameraRegistry::new();
        assert!(registry.register(MockProvider::new(2)).is_ok());
        assert_eq!(registry.providers().len(), 1);
        assert!(registry.providers().contains(&"Mock"));
    }

    #[test]
    fn test_registry_duplicate_provider() {
        let mut registry = CameraRegistry::new();
        assert!(registry.register(MockProvider::new(1)).is_ok());
        assert!(matches!(
            registry.register(MockProvider::new(1)),
            Err(CameraError::ProviderAlreadyRegistered(_))
        ));
    }

    #[test]
    fn test_registry_list_cameras() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(3)).unwrap();

        let cameras = registry.list_cameras("Mock").unwrap();
        assert_eq!(cameras.len(), 3);

        assert!(matches!(
            registry.list_cameras("Unknown"),
            Err(CameraError::ProviderNotFound(_))
        ));
    }

    #[test]
    fn test_registry_list_all_cameras() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(2)).unwrap();

        let all = registry.list_all_cameras().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].provider, "Mock");
        assert_eq!(all[0].index, 0);
        assert_eq!(all[1].index, 1);
    }

    #[test]
    fn test_registry_total_count() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(3)).unwrap();
        assert_eq!(registry.total_camera_count(), 3);
    }

    #[test]
    fn test_registry_open_camera() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(2)).unwrap();

        let camera = registry.open_camera("Mock", 0);
        assert!(camera.is_ok());
        assert_eq!(camera.unwrap().info().name, "Mock Camera 0");

        assert!(matches!(
            registry.open_camera("Mock", 10),
            Err(CameraError::InvalidCameraIndex { .. })
        ));

        assert!(matches!(
            registry.open_camera("Unknown", 0),
            Err(CameraError::ProviderNotFound(_))
        ));
    }

    #[test]
    fn test_registry_open_by_id() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(2)).unwrap();

        let camera = registry.open_by_id("Mock:1");
        assert!(camera.is_ok());
        assert_eq!(camera.unwrap().info().name, "Mock Camera 1");

        assert!(registry.open_by_id("invalid").is_err());
        assert!(registry.open_by_id("Mock:abc").is_err());
    }

    #[test]
    fn test_registry_open_first() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(2)).unwrap();

        let camera = registry.open_first();
        assert!(camera.is_ok());
    }

    #[test]
    fn test_registry_open_first_empty() {
        let mut registry = CameraRegistry::new();
        registry.register(MockProvider::new(0)).unwrap();

        assert!(matches!(
            registry.open_first(),
            Err(CameraError::NoCamerasFound)
        ));
    }
}
