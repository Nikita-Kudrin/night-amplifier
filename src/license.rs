use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// When `false`, all Pro plugin dispatch falls back to community behavior.
/// Set to `true` by the Pro binary after registering plugins.
/// Set back to `false` by the background license check if the license is expired.
pub static PRO_LICENSE_ACTIVE: AtomicBool = AtomicBool::new(false);

pub fn is_pro_active() -> bool {
    PRO_LICENSE_ACTIVE.load(Ordering::Relaxed)
}

/// Returns a reference to a Pro plugin only when the license is active.
/// Falls back to `None` (community behavior) when the license is expired.
pub fn pro_plugin<T: ?Sized>(plugin: &'static OnceLock<Box<T>>) -> Option<&'static T> {
    if !is_pro_active() {
        return None;
    }
    plugin.get().map(|b| b.as_ref())
}
