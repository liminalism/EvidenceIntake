//! Transcription backends. The kernel never sees these types.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::whisperx::WhisperxTranscript;
use crate::{Error, Result};

/// Something that can turn a wav into a WhisperX-shaped transcript.
pub trait TranscriptBackend {
    /// Transcribe `wav`. Times in the result are seconds of that file.
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript>;

    /// Version string stored on each statement (`franken_whisper@ggml-base/…`).
    fn version(&self) -> String;

    /// Extractor name stored on each statement.
    fn extractor(&self) -> &'static str {
        crate::EXTRACTOR_WHISPERX
    }
}

/// Reads a WhisperX JSON document from disk. Used in tests and when the
/// operator already ran WhisperX themselves.
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

/// Shells out to a local WhisperX install.
#[derive(Debug, Clone)]
pub struct WhisperxCliBackend {
    /// Binary name or path. Default `whisperx`.
    pub bin: PathBuf,
    /// Model name passed as `--model`.
    pub model: String,
}

impl WhisperxCliBackend {
    /// `whisperx` on `PATH`, `large-v3`.
    pub fn default_local() -> Self {
        Self {
            bin: PathBuf::from("whisperx"),
            model: "large-v3".to_owned(),
        }
    }
}

impl TranscriptBackend for WhisperxCliBackend {
    fn transcribe(&self, wav: &Path) -> Result<WhisperxTranscript> {
        let out_dir = tempfile::tempdir().map_err(|error| {
            Error::Backend(format!("could not create WhisperX output dir: {error}"))
        })?;
        let status = Command::new(&self.bin)
            .arg(wav)
            .arg("--model")
            .arg(&self.model)
            .arg("--output_format")
            .arg("json")
            .arg("--output_dir")
            .arg(out_dir.path())
            .status()
            .map_err(|error| {
                Error::Backend(format!(
                    "could not run `{}`: {error}. Install WhisperX locally (`pip install whisperx`) or pass --from-json.",
                    self.bin.display()
                ))
            })?;
        if !status.success() {
            return Err(Error::Backend(format!(
                "`{}` exited with {status}",
                self.bin.display()
            )));
        }
        let stem = wav
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| Error::Backend("audio path has no file stem".to_owned()))?;
        let json_path = out_dir.path().join(format!("{stem}.json"));
        let json = std::fs::read_to_string(&json_path).map_err(|error| {
            Error::Backend(format!(
                "WhisperX did not write {}: {error}",
                json_path.display()
            ))
        })?;
        WhisperxTranscript::from_json(&json)
            .map_err(|error| Error::Backend(format!("WhisperX JSON was unreadable: {error}")))
    }

    fn version(&self) -> String {
        format!("whisperx@{}", self.model)
    }
}
