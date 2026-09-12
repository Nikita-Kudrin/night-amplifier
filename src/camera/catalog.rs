//! The port the server discovers and opens cameras through.
//!
//! Discovery, connect and reconnect each used to build their own [`CameraRegistry`],
//! with provider sets that had already drifted apart. One catalog, held by `AppState`,
//! gives them the same view of the hardware — and gives tests a seam to script a USB
//! bus that reorders itself, which is the failure recovery has to survive.

use super::error::{CameraError, CameraResult};
use super::identity::DeviceIdentity;
use super::registry::{CameraEntry, CameraRegistry};
use super::traits::Camera;

/// A freshly opened camera and the canonical name of the provider that opened it.
pub struct OpenedCamera {
    pub camera: Box<dyn Camera>,
    pub provider: String,
}

/// Blocking: every method may call into a vendor SDK, so async callers go through
/// `spawn_blocking`.
pub trait DeviceCatalog: Send + Sync {
    /// Every camera discovery offers, across providers.
    fn list_all(&self, use_simulated: bool) -> CameraResult<Vec<CameraEntry>>;

    /// One provider's devices, in the order [`Self::open`] indexes them, without
    /// opening any. `provider` is matched case-insensitively.
    fn identities(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>>;

    /// Open `provider`'s device at `index`.
    fn open(&self, provider: &str, index: usize, use_simulated: bool) -> CameraResult<OpenedCamera>;
}

/// The production catalog: Player One and ZWO, the providers connect has always offered,
/// plus the simulator when it is switched on. SVBony, QHY and ToupTek implement
/// `identities` too but are not connectable yet, so recovery never reaches them.
#[derive(Debug, Default, Clone, Copy)]
pub struct RegistryCatalog;

impl RegistryCatalog {
    fn registry(use_simulated: bool) -> CameraRegistry {
        let mut registry = CameraRegistry::new();
        let _ = registry.register(super::PlayerOneProvider::new());
        let _ = registry.register(super::ZwoProvider::new());
        if use_simulated {
            let _ = registry.register(super::SimulatedProvider::new());
        }
        registry
    }

    fn canonical_name(registry: &CameraRegistry, provider: &str) -> CameraResult<String> {
        registry
            .providers()
            .into_iter()
            .find(|name| name.eq_ignore_ascii_case(provider))
            .map(str::to_string)
            .ok_or_else(|| CameraError::ProviderNotFound(provider.to_string()))
    }
}

impl DeviceCatalog for RegistryCatalog {
    fn list_all(&self, use_simulated: bool) -> CameraResult<Vec<CameraEntry>> {
        Self::registry(use_simulated).list_all_cameras()
    }

    fn identities(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>> {
        let registry = Self::registry(use_simulated);
        let name = Self::canonical_name(&registry, provider)?;
        registry
            .get_provider(&name)
            .ok_or(CameraError::ProviderNotFound(name))?
            .identities()
    }

    fn open(&self, provider: &str, index: usize, use_simulated: bool) -> CameraResult<OpenedCamera> {
        let registry = Self::registry(use_simulated);
        let name = Self::canonical_name(&registry, provider)?;
        let camera = registry.open_camera(&name, index)?;
        Ok(OpenedCamera {
            camera,
            provider: name,
        })
    }
}
