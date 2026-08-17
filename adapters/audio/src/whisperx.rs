//! WhisperX JSON as emitted by the Python CLI (`--output_format json`).

use serde::{Deserialize, Serialize};

/// One aligned word. Times are seconds in the audio WhisperX was given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhisperxWord {
    /// The token. Some WhisperX builds use `text` instead of `word`.
    #[serde(alias = "text")]
    pub word: String,
    /// Word start in seconds, when alignment produced one.
    pub start: Option<f64>,
    /// Word end in seconds, when alignment produced one.
    pub end: Option<f64>,
    /// Alignment confidence in `0..=1`, when present.
    pub score: Option<f64>,
}

/// One WhisperX segment, optionally carrying a diarization label.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhisperxSegment {
    /// Segment start in seconds.
    pub start: f64,
    /// Segment end in seconds.
    pub end: f64,
    /// Transcript text for the span.
    pub text: String,
    /// Word-level alignment, when the aligner ran.
    #[serde(default)]
    pub words: Vec<WhisperxWord>,
    /// Optional pyannote / WhisperX speaker label (`SPEAKER_00`).
    pub speaker: Option<String>,
    /// Segment-level log probability, when the backend reports one.
    pub avg_logprob: Option<f64>,
}

/// A full WhisperX transcript document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WhisperxTranscript {
    /// Ordered speech segments.
    #[serde(default)]
    pub segments: Vec<WhisperxSegment>,
    /// Detected language code, when reported.
    pub language: Option<String>,
}

impl WhisperxTranscript {
    /// Parses WhisperX JSON from a string.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}
