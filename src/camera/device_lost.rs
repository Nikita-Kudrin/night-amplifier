//! One canonical signal for "the vendor SDK says this device is gone".
//!
//! The decision has to be made inside the shim, where the vendor's error is
//! still an enum or an `i32`. By the time an error reaches
//! [`CameraError`](crate::camera::CameraError) it is a rendered string, and the
//! providers do not render alike: PlayerOne prints its enum symbolically
//! (`POA_ERROR_NOT_OPENED`) while ZWO, SVBony, QHY and ToupTek print a bare
//! number. Matching vendor substrings after the fact therefore worked for
//! exactly one provider and silently failed for the rest.
//!
//! So each shim classifies its own code against its own table and, for a
//! device-loss code, prepends [`MARKER`] — a token this codebase writes and
//! this codebase reads, with no vendor vocabulary in it.
//! [`CameraError::is_sdk_disconnected`](crate::camera::CameraError::is_sdk_disconnected)
//! looks for that one token plus the typed variants, and knows nothing about
//! any SDK.
//!
//! A provider whose SDK has no distinct device-loss code (QHY reports one
//! generic `QHYCCD_ERROR`) simply never marks. Those faults are still caught,
//! one layer up, by the capture and status watchdog timeouts.

use std::fmt::Display;

use super::error::{CameraError, CameraResult};

/// Prefix identifying a vendor error whose code means the device is gone.
///
/// Written only by [`mark`] and read only by [`is_marked`]. Provider layers
/// wrap shim messages (`"Failed to set exposure: {shim}"`), so the marker can
/// end up anywhere in the final string — hence the substring test.
pub const MARKER: &str = "device lost: ";

/// Tag a rendered vendor error as device loss.
pub fn mark(detail: impl Display) -> String {
    format!("{}{}", MARKER, detail)
}

/// Whether a rendered error carries the device-loss marker.
pub fn is_marked(message: &str) -> bool {
    message.contains(MARKER)
}

/// Fold a `status()` field read whose failure is usually benign.
///
/// Most fields a `CameraStatus` carries are optional in practice — a camera
/// without a cooler has no cooler power, some models refuse to report offset —
/// so every provider wrote these reads as `.unwrap_or(default)`. That also
/// swallowed the one failure that is never benign: on a handle whose device
/// has gone, *every* read fails, and `status()` still returned `Ok` with a
/// temperature of 0.0. The monitor polled a dead camera indefinitely and its
/// disconnect detector never saw a single error.
///
/// This keeps the tolerance and removes the blind spot: an unsupported
/// parameter still falls back, a lost device propagates.
pub fn tolerate_unsupported<T>(reading: Result<T, String>, fallback: T) -> CameraResult<T> {
    match reading {
        Ok(value) => Ok(value),
        Err(message) if is_marked(&message) => Err(CameraError::DeviceLost(message)),
        Err(_) => Ok(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marked_message_keeps_the_vendor_detail_readable() {
        let m = mark("POASetConfig failed: POA_ERROR_NOT_OPENED");
        assert!(is_marked(&m));
        assert!(m.contains("POASetConfig failed: POA_ERROR_NOT_OPENED"));
    }

    #[test]
    fn unmarked_vendor_text_is_not_device_loss() {
        assert!(!is_marked("POASetConfig failed: POA_ERROR_OUT_OF_LIMIT"));
        assert!(!is_marked(
            "ASISetControlValue failed: ASI_ERROR_TIMEOUT (11)"
        ));
        assert!(!is_marked(""));
    }

    #[test]
    fn an_unsupported_parameter_falls_back_but_a_lost_device_does_not() {
        let unsupported: Result<f32, String> =
            Err("POAGetConfig failed: POA_ERROR_CONF_CANNOT_READ".to_string());
        assert_eq!(tolerate_unsupported(unsupported, -273.0).unwrap(), -273.0);

        let lost: Result<f32, String> = Err(mark("POAGetConfig failed: POA_ERROR_NOT_OPENED"));
        let err = tolerate_unsupported(lost, -273.0).unwrap_err();
        assert!(
            err.is_sdk_disconnected(),
            "a lost device must not be folded into the fallback"
        );
    }

    /// Provider layers wrap the shim's message, so the marker is not at the
    /// front by the time `is_sdk_disconnected` sees it.
    #[test]
    fn marker_survives_being_wrapped_by_the_provider_layer() {
        let shim = mark("POAImageReady failed: POA_ERROR_NOT_OPENED");
        let wrapped = format!("Failed to set exposure: {}", shim);
        assert!(is_marked(&wrapped));
    }
}
