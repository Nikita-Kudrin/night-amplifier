//! Error types for the stacking engine

use thiserror::Error;

use crate::ffi_safety::FfiError;

/// Result type alias for stacking operations
pub type Result<T> = std::result::Result<T, StackError>;

/// Errors that can occur during stacking operations
#[derive(Error, Debug, Clone, PartialEq)]
pub enum StackError {
    /// Buffer size doesn't match expected dimensions
    #[error("Buffer size mismatch: expected {expected} bytes, got {actual}")]
    BufferSizeMismatch { expected: usize, actual: usize },

    /// Invalid image dimensions
    #[error("Invalid dimensions: {width}x{height} with {channels} channels")]
    InvalidDimensions {
        width: usize,
        height: usize,
        channels: usize,
    },

    /// Calibration frame dimensions don't match
    #[error("Calibration frame dimension mismatch: frame is {frame_width}x{frame_height}, calibration is {cal_width}x{cal_height}")]
    CalibrationDimensionMismatch {
        frame_width: usize,
        frame_height: usize,
        cal_width: usize,
        cal_height: usize,
    },

    /// Flat field contains zero or near-zero values
    #[error("Flat field contains invalid values (zero or near-zero) at {count} pixels")]
    InvalidFlatField { count: usize },

    /// Channel count mismatch between frames
    #[error("Channel count mismatch: expected {expected}, got {actual}")]
    ChannelMismatch { expected: usize, actual: usize },

    /// Arithmetic overflow or invalid value
    #[error("Arithmetic error: {message}")]
    ArithmeticError { message: String },

    /// Image registration failed
    #[error("Registration failed: {0}")]
    Registration(String),

    /// Star detection failed
    #[error("Star detection failed: {0}")]
    Detection(String),

    /// Invalid configuration parameter
    #[error("Invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// FFI boundary error (panic in C/C++ library, null pointer, etc.)
    #[error("FFI error: {0}")]
    FfiBoundaryError(String),
}

impl From<FfiError> for StackError {
    fn from(err: FfiError) -> Self {
        StackError::FfiBoundaryError(err.to_string())
    }
}
