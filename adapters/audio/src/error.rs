//! Adapter errors. These never become kernel review states.

use std::path::PathBuf;

/// Result returned by the audio adapter.
pub type Result<T> = std::result::Result<T, Error>;

/// Failures while decoding, cleaning, transcribing, or mapping a recording.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The file could not be read or was not a usable wav.
    #[error("could not decode `{path}`: {message}")]
    Decode {
        /// Path that failed.
        path: PathBuf,
        /// Why decode refused it.
        message: String,
    },
    /// The recording is empty, truncated, or has no samples.
    #[error("{0}")]
    Empty(String),
    /// A sample rate the resampler crate cannot take, and no fallback applies.
    #[error(
        "unsupported sample rate {rate} Hz; supported rates are 8/16/22.05/32/44.1/48 kHz and their 2× multiples"
    )]
    UnsupportedRate {
        /// The file's sample rate.
        rate: u32,
    },
    /// WhisperX JSON could not be read or did not describe any speech.
    #[error("transcript: {0}")]
    Transcript(String),
    /// The transcription backend was missing or failed.
    #[error("{0}")]
    Backend(String),
    /// ffmpeg could not pull an audio track out of a container.
    #[error("could not extract audio from `{path}`: {message}")]
    Extract {
        /// Path of the original container.
        path: PathBuf,
        /// Why extraction refused it.
        message: String,
    },
    /// Mapping produced a batch the adapter itself refuses (inverted times, empty text).
    #[error("invalid mapping: {0}")]
    Mapping(String),
    /// An I/O failure while writing a batch or a working-copy wav.
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    /// A wav encode or decode failure.
    #[error("wav: {0}")]
    Wav(#[from] hound::Error),
    /// JSON (de)serialization failed.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
