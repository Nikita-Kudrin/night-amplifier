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

    /// The solve was abandoned on request, not because it could not find the field.
    ///
    /// Kept distinct from [`PushToError::SolveFailed`] because the two demand opposite
    /// responses: a failure means the cached position is stale and the solver should
    /// try again, while a cancel means the user asked it to stop — the cached position
    /// is still the best thing known, and re-solving on the next frame would make the
    /// cancel button do nothing.
    #[error("Plate solve cancelled")]
    Cancelled,

    #[error("Not enough stars for plate solving (found {found}, need {required})")]
    NotEnoughStars { found: usize, required: usize },

    #[error("Frame quality too poor for plate solving: {0}")]
    PoorFrameQuality(String),

    /// The view has not stopped moving yet, so the frame is smeared.
    ///
    /// Kept apart from [`PushToError::PoorFrameQuality`] because it is expected and
    /// self-correcting — it is what every frame looks like for the second or two
    /// after a Dobsonian is nudged — and the UI should say "waiting" rather than
    /// reporting a fault.
    #[error("Waiting for the view to settle: {0}")]
    NotSettled(String),

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

impl PushToError {
    /// Whether this says the *frame* was unusable rather than the *sky* unmatchable.
    ///
    /// The two demand opposite responses and were handled identically. A solve that
    /// ASTAP ran and lost means the cached position is stale — we settled somewhere
    /// new and could not place it. A frame rejected before ASTAP ever saw it means
    /// only that this picture was poor; the scope has not moved, the cached position
    /// is still the best thing known, and the next frame routinely fixes it by
    /// itself. Conflating them discarded a good fix over one blurred frame and then
    /// refused to look again for the length of an exponential backoff.
    pub fn is_frame_quality(&self) -> bool {
        matches!(
            self,
            Self::NotEnoughStars { .. } | Self::PoorFrameQuality(_) | Self::NotSettled(_)
        )
    }
}

/// Result type for Push-To operations
pub type PushToResult<T> = Result<T, PushToError>;
