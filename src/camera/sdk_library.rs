//! Opening a vendor SDK with eager binding (`RTLD_NOW`): a library with an unresolvable
//! symbol then fails to load, and its provider reports unavailable, instead of killing the
//! whole process at its first call. 2026-09-14: the Linux SVBony SDK leaves 22 `libusb_*`
//! symbols undefined without declaring libusb, and discovery aborted the server with
//! "symbol lookup error" (exit 127).

use dlopen2::wrapper::{Container, WrapperApi};

#[cfg(unix)]
const EAGER: Option<i32> = Some(libc::RTLD_NOW | libc::RTLD_LOCAL);
#[cfg(not(unix))]
const EAGER: Option<i32> = None;

/// # Safety
/// Loading runs the library's initializers, and `T` must match its exported signatures.
pub(crate) unsafe fn load_eagerly<T: WrapperApi>(name: &str) -> Result<Container<T>, dlopen2::Error> {
    Container::load_with_flags(name, EAGER)
}

/// A symbol some versions of an already loaded SDK lack, looked up apart from its
/// `WrapperApi`, where one missing symbol fails the whole load.
///
/// # Safety
/// `T` must match the symbol's type, and the pointer stays valid only while the SDK's own
/// `Container` keeps the library loaded.
pub(crate) unsafe fn optional_symbol<T>(library: &str, symbol: &str) -> Option<T> {
    let handle = dlopen2::raw::Library::open_with_flags(library, EAGER).ok()?;
    handle.symbol::<T>(symbol).ok()
}

/// Whether a failed load means the file is not there, as opposed to a library that exists
/// but cannot load (an unresolvable symbol, the wrong architecture). The platform's message
/// is all there is to go by.
pub(crate) fn is_missing_file(error: &dlopen2::Error) -> bool {
    let message = error.to_string().to_lowercase();
    ["no such file", "could not be found", "cannot find"]
        .iter()
        .any(|missing| message.contains(missing))
}

/// Make libusb's symbols global, for an SDK that calls libusb without declaring it (the
/// Linux SVBony SDK). Kept loaded for the life of the process; a missing libusb surfaces as
/// the eager SDK load's own error.
#[cfg(target_os = "linux")]
pub(crate) fn preload_libusb() {
    let flags = Some(libc::RTLD_NOW | libc::RTLD_GLOBAL);
    if let Ok(libusb) = dlopen2::raw::Library::open_with_flags("libusb-1.0.so.0", flags) {
        std::mem::forget(libusb);
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn preload_libusb() {}

#[cfg(test)]
mod tests {
    use super::*;

    /// The SVBony SDK without libusb: present, but refused. Reported as "not installed", the
    /// user had nothing to fix.
    #[test]
    fn a_refused_library_is_not_a_missing_one() {
        let refused = dlopen2::Error::OpeningLibraryError(std::io::Error::other(
            "/lib/libSVBCameraSDK.so: undefined symbol: libusb_handle_events_timeout",
        ));
        assert!(!is_missing_file(&refused));
    }

    #[cfg(target_os = "linux")]
    #[derive(WrapperApi)]
    struct LibmApi {
        cos: unsafe extern "C" fn(x: f64) -> f64,
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_eagerly_loaded_library_is_callable() {
        let libm = unsafe { load_eagerly::<LibmApi>("libm.so.6") }.expect("libm loads");
        assert_eq!(unsafe { libm.cos(0.0) }, 1.0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_library_is_reported_as_missing() {
        let error = unsafe { load_eagerly::<LibmApi>("libnot-a-vendor-sdk.so") }
            .err()
            .expect("no such library");
        assert!(is_missing_file(&error), "{error}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_optional_symbol_is_found_only_when_present() {
        let cos = unsafe { optional_symbol::<unsafe extern "C" fn(f64) -> f64>("libm.so.6", "cos") }
            .expect("libm exports cos");
        assert_eq!(unsafe { cos(0.0) }, 1.0);
        let absent = unsafe { optional_symbol::<unsafe extern "C" fn(bool)>("libm.so.6", "EnableQHYCCDMessage") };
        assert!(absent.is_none());
    }
}
