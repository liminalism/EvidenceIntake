//! Transcription backends. The kernel never sees these types.

use std::path::{Path, PathBuf};

use crate::whisperx::WhisperxTranscript;
use crate::{Error, Result};

/// Something that can turn a wav into a WhisperX-shaped transcript.
pub trait TranscriptBackend {
    /// Transcribe `wav`. Times in the result are seconds of that file.
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript>;

    /// Version string stored on each statement.
    fn version(&self) -> String;

    /// Extractor name stored on each statement.
    fn extractor(&self) -> &'static str {
        crate::EXTRACTOR_WHISPERX
    }
}

/// Reads a WhisperX JSON document from disk. Used in tests and when the
/// operator already ran an external reference backend themselves.
#[derive(Debug, Clone)]
pub struct JsonFileBackend {
    /// Path to the JSON document.
    pub path: PathBuf,
    /// Version stamped on the mapped statements.
    pub version: String,
}

impl TranscriptBackend for JsonFileBackend {
    fn transcribe(&self, _wav: &Path) -> Result<WhisperxTranscript> {
        let json = std::fs::read_to_string(&self.path).map_err(|error| {
            Error::Transcript(format!("could not read {}: {error}", self.path.display()))
        })?;
        WhisperxTranscript::from_json(&json).map_err(|error| {
            Error::Transcript(format!(
                "{} is not WhisperX JSON: {error}",
                self.path.display()
            ))
        })
    }

    fn version(&self) -> String {
        self.version.clone()
    }
}
