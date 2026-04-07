//! FFI safety utilities for handling C/C++ library boundaries
//!
//! Rust's safety guarantees end at FFI boundaries. This module provides utilities
//! to defensively handle operations that call into C/C++ libraries (camera SDKs, cfitsio)
//! which may panic, segfault, or otherwise misbehave.

use std::panic::{self, AssertUnwindSafe};
use std::time::Duration;
use tracing::{error, warn};

/// Error type for FFI boundary failures
#[derive(Debug, Clone, PartialEq)]
pub enum FfiError {
    /// The FFI call panicked
    Panic(String),
    /// The FFI call returned an unexpected null pointer
    NullPointer(&'static str),
    /// The operation timed out (SDK hung)
    Timeout(Duration),
    /// The FFI library is in an invalid state
    InvalidState(String),
    /// Memory allocation failed in the FFI layer
    AllocationFailed(String),
    /// Buffer overflow or underflow detected
    BufferOverflow { expected: usize, actual: usize },
}

impl std::fmt::Display for FfiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FfiError::Panic(msg) => write!(f, "FFI call panicked: {}", msg),
            FfiError::NullPointer(ctx) => write!(f, "Null pointer from FFI: {}", ctx),
            FfiError::Timeout(d) => write!(f, "FFI call timed out after {:?}", d),
            FfiError::InvalidState(msg) => write!(f, "FFI library in invalid state: {}", msg),
            FfiError::AllocationFailed(msg) => write!(f, "FFI allocation failed: {}", msg),
            FfiError::BufferOverflow { expected, actual } => {
                write!(
                    f,
                    "Buffer overflow: expected {} bytes, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl std::error::Error for FfiError {}

/// Result type for FFI operations
pub type FfiResult<T> = Result<T, FfiError>;

/// Execute an FFI-calling closure with panic catching
///
/// This wraps the closure in `catch_unwind` to prevent panics from unwinding
/// across the FFI boundary, which would be undefined behavior.
///
/// # Safety
///
/// The closure should not hold any non-unwind-safe references. This function
/// uses `AssertUnwindSafe` to bypass the check, so the caller must ensure
/// the closure is actually safe to unwind.
///
/// # Example
///
/// ```ignore
/// use night_amplifier::ffi_safety::catch_ffi_panic;
///
/// let result = catch_ffi_panic("camera_open", || {
///     unsafe_sdk_call()
/// });
/// ```
pub fn catch_ffi_panic<T, F>(context: &str, f: F) -> FfiResult<T>
where
    F: FnOnce() -> T,
{
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => Ok(result),
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else {
                "Unknown panic".to_string()
            };
            error!(context = context, error = %msg, "FFI call panicked");
            Err(FfiError::Panic(format!("{}: {}", context, msg)))
        }
    }
}

/// Execute an FFI-calling closure that returns a Result, with panic catching
///
/// This combines panic catching with Result handling for cleaner error propagation.
pub fn catch_ffi_panic_result<T, E, F>(context: &str, f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E>,
    E: From<FfiError>,
{
    match catch_ffi_panic(context, f) {
        Ok(inner_result) => inner_result,
        Err(ffi_err) => Err(E::from(ffi_err)),
    }
}

/// Validate that a buffer has the expected size before passing to FFI
///
/// Many C libraries assume buffers are correctly sized. This validates
/// the assumption and returns a clear error if not.
pub fn validate_buffer_size(buffer: &[u8], expected_size: usize, context: &str) -> FfiResult<()> {
    if buffer.len() < expected_size {
        warn!(
            context = context,
            expected = expected_size,
            actual = buffer.len(),
            "Buffer underflow detected"
        );
        return Err(FfiError::BufferOverflow {
            expected: expected_size,
            actual: buffer.len(),
        });
    }
    Ok(())
}

/// Validate that dimensions will not overflow when computing buffer sizes
///
/// Prevents integer overflow in width * height * channels calculations
/// that could lead to buffer overflows.
pub fn validate_dimensions(
    width: usize,
    height: usize,
    channels: usize,
    bytes_per_sample: usize,
) -> FfiResult<usize> {
    width
        .checked_mul(height)
        .and_then(|wh| wh.checked_mul(channels))
        .and_then(|whc| whc.checked_mul(bytes_per_sample))
        .ok_or_else(|| FfiError::BufferOverflow {
            expected: 0,
            actual: usize::MAX,
        })
}

/// Guard for ensuring FFI resources are properly cleaned up
///
/// Some C libraries require explicit cleanup even on error paths.
/// This guard calls the cleanup function on drop.
pub struct FfiCleanupGuard<F: FnOnce()> {
    cleanup: Option<F>,
}

impl<F: FnOnce()> FfiCleanupGuard<F> {
    /// Create a new cleanup guard
    pub fn new(cleanup: F) -> Self {
        Self {
            cleanup: Some(cleanup),
        }
    }

    /// Disarm the guard (don't call cleanup on drop)
    ///
    /// Use this when the operation succeeded and cleanup is not needed.
    pub fn disarm(&mut self) {
        self.cleanup = None;
    }
}

impl<F: FnOnce()> Drop for FfiCleanupGuard<F> {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            // Catch panics in cleanup to prevent double-panic
            let _ = panic::catch_unwind(AssertUnwindSafe(cleanup));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_catch_ffi_panic_success() {
        let result = catch_ffi_panic("test", || 42);
        assert_eq!(result, Ok(42));
    }

    #[test]
    fn test_catch_ffi_panic_catches_panic() {
        let result: FfiResult<i32> = catch_ffi_panic("test_panic", || {
            panic!("test panic message");
        });
        assert!(result.is_err());
        match result {
            Err(FfiError::Panic(msg)) => {
                assert!(msg.contains("test_panic"));
                assert!(msg.contains("test panic message"));
            }
            _ => panic!("Expected Panic error"),
        }
    }

    #[test]
    fn test_validate_buffer_size_ok() {
        let buffer = vec![0u8; 100];
        assert!(validate_buffer_size(&buffer, 100, "test").is_ok());
        assert!(validate_buffer_size(&buffer, 50, "test").is_ok());
    }

    #[test]
    fn test_validate_buffer_size_underflow() {
        let buffer = vec![0u8; 50];
        let result = validate_buffer_size(&buffer, 100, "test");
        assert!(matches!(
            result,
            Err(FfiError::BufferOverflow {
                expected: 100,
                actual: 50
            })
        ));
    }

    #[test]
    fn test_validate_dimensions_ok() {
        let result = validate_dimensions(1920, 1080, 3, 2);
        assert_eq!(result, Ok(1920 * 1080 * 3 * 2));
    }

    #[test]
    fn test_validate_dimensions_overflow() {
        let result = validate_dimensions(usize::MAX, 2, 1, 1);
        assert!(result.is_err());
    }

    #[test]
    fn test_cleanup_guard_calls_on_drop() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        {
            let _guard = FfiCleanupGuard::new(move || {
                called_clone.store(true, Ordering::SeqCst);
            });
        }

        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_cleanup_guard_disarm() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        let called = Arc::new(AtomicBool::new(false));
        let called_clone = Arc::clone(&called);

        {
            let mut guard = FfiCleanupGuard::new(move || {
                called_clone.store(true, Ordering::SeqCst);
            });
            guard.disarm();
        }

        assert!(!called.load(Ordering::SeqCst));
    }

    #[test]
    fn test_ffi_error_display() {
        assert_eq!(
            FfiError::Panic("test".into()).to_string(),
            "FFI call panicked: test"
        );
        assert_eq!(
            FfiError::NullPointer("camera").to_string(),
            "Null pointer from FFI: camera"
        );
        assert_eq!(
            FfiError::BufferOverflow {
                expected: 100,
                actual: 50
            }
            .to_string(),
            "Buffer overflow: expected 100 bytes, got 50"
        );
    }
}
