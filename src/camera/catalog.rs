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
    /// Every provider discovery asks, in the order it lists their cameras.
    fn provider_names(&self, use_simulated: bool) -> Vec<String>;

    /// One provider's cameras, empty when its SDK is not installed. Discovery bounds each
    /// provider's call on its own, so one hung SDK neither holds up nor hides the others.
    fn list(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<CameraEntry>>;

    /// One provider's devices, in the order [`Self::open`] indexes them, without
    /// opening any. `provider` is matched case-insensitively.
    fn identities(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>>;

    /// Open `provider`'s device at `index`.
    fn open(&self, provider: &str, index: usize, use_simulated: bool) -> CameraResult<OpenedCamera>;
}

/// The production catalog: every vendor provider, plus the simulator when it is switched on.
/// INDI is not here — it discovers through its own server connection and is not connectable.
#[derive(Debug, Clone, Copy)]
pub struct RegistryCatalog {
    vendors: bool,
}

impl RegistryCatalog {
    pub fn new() -> Self {
        Self { vendors: true }
    }

    /// The simulator alone, so tests make no vendor SDK calls whatever the machine has
    /// installed — discovery used to open real cameras from parallel tests.
    #[cfg(test)]
    pub(crate) fn simulator_only() -> Self {
        Self { vendors: false }
    }

    fn registry(&self, use_simulated: bool) -> CameraRegistry {
        let mut registry = CameraRegistry::new();
        if self.vendors {
            registry.register_vendors();
        }
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

impl Default for RegistryCatalog {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceCatalog for RegistryCatalog {
    fn provider_names(&self, use_simulated: bool) -> Vec<String> {
        self.registry(use_simulated)
            .providers()
            .into_iter()
            .map(str::to_string)
            .collect()
    }

    fn list(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<CameraEntry>> {
        let registry = self.registry(use_simulated);
        let name = Self::canonical_name(&registry, provider)?;
        let found = registry
            .get_provider(&name)
            .ok_or_else(|| CameraError::ProviderNotFound(name.clone()))?;
        if !found.is_available() {
            return Ok(Vec::new());
        }
        Ok(found
            .list_cameras()?
            .into_iter()
            .enumerate()
            .map(|(index, info)| CameraEntry {
                provider: name.clone(),
                index,
                info,
            })
            .collect())
    }

    fn identities(&self, provider: &str, use_simulated: bool) -> CameraResult<Vec<DeviceIdentity>> {
        let registry = self.registry(use_simulated);
        let name = Self::canonical_name(&registry, provider)?;
        registry
            .get_provider(&name)
            .ok_or(CameraError::ProviderNotFound(name))?
            .identities()
    }

    fn open(&self, provider: &str, index: usize, use_simulated: bool) -> CameraResult<OpenedCamera> {
        let registry = self.registry(use_simulated);
        let name = Self::canonical_name(&registry, provider)?;
        let camera = registry.open_camera(&name, index)?;
        Ok(OpenedCamera {
            camera,
            provider: name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_names(catalog: RegistryCatalog, use_simulated: bool) -> Vec<String> {
        let mut names = catalog.provider_names(use_simulated);
        names.sort();
        names
    }

    /// Discovery, connect and recovery all see this one set, so a vendor missing from it
    /// lists nowhere and never connects — QHY, ToupTek and SVBony until 2026-09-14.
    #[test]
    fn every_vendor_provider_is_offered() {
        assert_eq!(
            provider_names(RegistryCatalog::new(), false),
            ["PlayerOne", "QHY", "SVBony", "ToupTek", "ZWO"]
        );
        assert_eq!(
            provider_names(RegistryCatalog::new(), true),
            ["PlayerOne", "QHY", "SVBony", "Simulator", "ToupTek", "ZWO"]
        );
    }

    #[test]
    fn the_test_catalog_offers_the_simulator_alone() {
        assert!(provider_names(RegistryCatalog::simulator_only(), false).is_empty());
        assert_eq!(provider_names(RegistryCatalog::simulator_only(), true), ["Simulator"]);
    }

    /// Every `/api/cameras` call builds a fresh registry and the UI does not sort, so an order
    /// that depends on the registry instance reshuffles the camera list on each refresh.
    #[test]
    fn providers_enumerate_in_the_same_order_every_time() {
        let first = RegistryCatalog::new().provider_names(true);
        for _ in 0..20 {
            assert_eq!(RegistryCatalog::new().provider_names(true), first);
        }
    }

    /// Camera ids carry the provider lower-cased (`qhy_sn-…`).
    #[test]
    fn provider_names_from_camera_ids_resolve() {
        let registry = RegistryCatalog::new().registry(false);
        for (from_id, canonical) in [("qhy", "QHY"), ("touptek", "ToupTek"), ("svbony", "SVBony")] {
            assert_eq!(RegistryCatalog::canonical_name(&registry, from_id).unwrap(), canonical);
        }
    }

    #[test]
    fn listing_an_unknown_provider_is_an_error() {
        assert!(matches!(
            RegistryCatalog::new().list("NoSuchVendor", false),
            Err(CameraError::ProviderNotFound(_))
        ));
    }
}
