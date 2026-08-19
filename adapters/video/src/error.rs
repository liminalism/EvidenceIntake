//! Adapter errors. These never become kernel review states.

use std::path::PathBuf;

/// Result returned by the video adapter.
pub type Result<T> = std::result::Result<T, Error>;

/// Failures while probing, cutting, or mapping a recording.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read or has no video stream.
    #[error("could not open `{path}`: {message}")]
    Open {
        /// Path that failed.
        path: PathBuf,
        /// Why open refused it.
        message: String,
    },
    /// ffmpeg / ffprobe was missing or failed.
    #[error("{0}")]
    Probe(String),
    /// Mapping produced a batch the adapter itself refuses.
    #[error("invalid mapping: {0}")]
    Mapping(String),
    /// The vision, caption, OCR, or embedding backend was missing or failed.
    #[error("{0}")]
    Backend(String),
    /// The audio adapter refused the soundtrack.
    #[error("audio adapter: {0}")]
    Audio(#[from] evidence_audio::Error),
    /// An I/O failure.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
