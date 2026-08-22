//! Document-adapter errors.

use std::path::PathBuf;

/// Result returned by the document adapter.
pub type Result<T> = std::result::Result<T, Error>;

/// A document runner, DocIR, identity, or mapping failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Filesystem or child-process I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON could not be decoded or encoded.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// The packaged OCR command failed.
    #[error("document OCR backend failed: {0}")]
    Backend(String),
    /// The canonical document has an unsupported or malformed contract.
    #[error("invalid DocIR: {0}")]
    InvalidDocIr(String),
    /// DocIR does not describe the untouched source supplied to the adapter.
    #[error("DocIR source does not match {path}: {reason}")]
    SourceMismatch {
        /// Original file supplied by the operator.
        path: PathBuf,
        /// Mismatch found during validation.
        reason: String,
    },
    /// Mapping produced no reviewable text.
    #[error("document contained no non-empty raw text regions")]
    NoText,
}
