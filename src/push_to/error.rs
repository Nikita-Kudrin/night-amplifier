//! Error types for Push-To navigation

use thiserror::Error;

/// Push-To specific errors
#[derive(Debug, Error)]
pub enum PushToError {
    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Push-To plugin not found. This feature requires Night Amplifier Pro.")]
    PluginRequired,

    #[error("Detection failed: {0}")]
    DetectionFailed(String),

    #[error("Plate solve failed: {0}")]
    SolveFailed(String),

    #[error("Not enough stars for plate solving (found {found}, need {required})")]
    NotEnoughStars { found: usize, required: usize },

    #[error("Target '{0}' not found in catalog")]
    TargetNotFound(String),

    #[error("Database load failed: {0}")]
    DatabaseLoadFailed(String),

    #[error("Installation failed: {0}")]
    InstallFailed(String),

    #[error("Extraction failed: {0}")]
    ExtractionFailed(String),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result type for Push-To operations
pub type PushToResult<T> = Result<T, PushToError>;
