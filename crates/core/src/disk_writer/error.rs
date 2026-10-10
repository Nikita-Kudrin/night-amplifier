/// Errors that can occur during disk writing
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskWriterError {
    /// The write queue is full
    QueueFull,
    /// The writer task has been closed
    WriterClosed,
    /// Failed to create directory
    DirectoryCreationFailed(String),
    /// Failed to write file
    WriteFailed(String),
}

impl std::fmt::Display for DiskWriterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueFull => write!(f, "Disk writer queue is full"),
            Self::WriterClosed => write!(f, "Disk writer has been closed"),
            Self::DirectoryCreationFailed(msg) => write!(f, "Failed to create directory: {}", msg),
            Self::WriteFailed(msg) => write!(f, "Failed to write file: {}", msg),
        }
    }
}

impl std::error::Error for DiskWriterError {}
