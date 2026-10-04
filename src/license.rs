use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LicenseDetails {
    pub name: String,
    pub email: String,
    pub issued_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LicenseStatus {
    pub active: bool,
    pub details: Option<LicenseDetails>,
}

/// Holds the parsed license details when a Pro license is active.
pub static PRO_LICENSE_DATA: OnceLock<RwLock<Option<LicenseDetails>>> = OnceLock::new();

/// An injected closure from the Pro binary to validate and update the license JWT.
/// Signature: fn(jwt_string) -> Result<LicenseDetails, String>
pub type LicenseUpdaterFn = Box<dyn Fn(String) -> Result<LicenseDetails, String> + Send + Sync>;
pub static LICENSE_UPDATER: OnceLock<LicenseUpdaterFn> = OnceLock::new();

/// When `false`, every [`crate::plugins::Plugins`] accessor answers `None` (Community
/// behaviour), unless the set was built `always_licensed`. Set by the Pro binary.
pub static PRO_LICENSE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn is_pro_active() -> bool {
    PRO_LICENSE_ACTIVE.load(Ordering::Relaxed)
}

