//! Audio intake adapter for the evidence collation kernel.
//!
//! Decode a discovery recording, form channel/level hypotheses from the
//! original PCM, clean a working copy, transcribe it, and emit a
//! [`NormalizedBatch`](evidence_intake::NormalizedBatch) whose locators address
//! the original timeline. The kernel crate never depends on this one.

mod backend;
mod channel;
mod clean;
mod decode;
mod error;
mod map;
mod media;
mod trt;
mod whisperx;

pub use backend::{JsonFileBackend, TranscriptBackend};
pub use channel::{
    CHANNEL_RATIO, ChannelSide, ChannelSplit, DUAL_MONO_BALANCE, LEVEL_RATIO, LevelSplit,
    SILENCE_RMS, channel_split, file_rms, level_split,
};
pub use clean::{PHONE_HIGH_HZ, PHONE_LOW_HZ, WORKING_RATE, prepare_working_copy, resample_to};
pub use decode::{DecodedAudio, decode_wav, write_wav};
pub use error::{Error, Result};
pub use map::{
    ANALYSIS_VERSION, EXTRACTOR_CHANNEL, EXTRACTOR_DIARIZE, EXTRACTOR_LEVEL, EXTRACTOR_NO_SPEECH,
    EXTRACTOR_WHISPERX, MappingOptions, SourceIdentity, format_clock, format_locator,
    transcript_to_batch,
};
pub use media::{MediaClass, OpenedMedia, classify, ffmpeg_available, media_type, open_media};
pub use trt::{EXTRACTOR_TRT_WHISPER, TrtWhisperBackend};
pub use whisperx::{WhisperxSegment, WhisperxTranscript, WhisperxWord};

use std::path::Path;

use evidence_intake::{CaseId, NormalizedBatch, TemporalRelation};

/// Inputs for one recording → one [`NormalizedBatch`].
#[derive(Debug, Clone)]
pub struct IntakeRequest {
    /// Existing case.
    pub case_id: CaseId,
    /// Existing production that will own the source.
    pub production_id: String,
    /// Adapter-assigned source identifier.
    pub source_id: String,
    /// Path to the untouched original.
    pub path: std::path::PathBuf,
    /// Display name; the file name when omitted by the caller.
    pub logical_name: Option<String>,
    /// Whether the recording is contemporaneous. Default is `unknown`.
    pub temporal_relation: TemporalRelation,
    /// Apply the telephone band-pass on the working copy.
    pub phone_band: bool,
    /// Emit near/far level observations. Off by default.
    pub level_split: bool,
    /// Minimum hole, in milliseconds, that becomes a `recording_gap`.
    pub gap_ms: u64,
}

/// Decode, analyse, optionally clean, transcribe, and map one file.
///
/// Accepts a wav or a local video/audio container. Video is hashed as video;
/// ffmpeg pulls the soundtrack for analysis and ASR. `backend` sees a cleaned
/// 48 kHz mono wav. Times it returns address the original timeline because
/// cleanup never time-stretches.
pub fn transcribe(
    request: &IntakeRequest,
    backend: &dyn TranscriptBackend,
) -> Result<NormalizedBatch> {
    let opened = open_media(&request.path)?;
    let working = prepare_working_copy(&opened.audio, request.phone_band)?;
    let working_file = tempfile::Builder::new()
        .prefix("evidence-audio-")
        .suffix(".wav")
        .tempfile()?;
    write_wav(working_file.path(), &working)?;

    let transcript = backend.transcribe(working_file.path())?;
    let identity = source_identity(request, &opened);
    let options = analysis_options(
        request,
        &opened.audio,
        &transcript,
        backend.extractor(),
        backend.version(),
    );
    transcript_to_batch(&identity, &transcript, &options)
}

/// Map an already-produced WhisperX JSON document against an original file.
///
/// Used when the operator ran WhisperX themselves (`--from-json`) and when
/// tests exercise the mapper without a model.
pub fn map_json_file(
    request: &IntakeRequest,
    json_path: &Path,
    version: impl Into<String>,
) -> Result<NormalizedBatch> {
    let opened = open_media(&request.path)?;
    let backend = JsonFileBackend {
        path: json_path.to_path_buf(),
        version: version.into(),
    };
    let transcript = backend.transcribe(&request.path)?;
    let identity = source_identity(request, &opened);
    let options = analysis_options(
        request,
        &opened.audio,
        &transcript,
        EXTRACTOR_WHISPERX,
        backend.version(),
    );
    transcript_to_batch(&identity, &transcript, &options)
}

fn source_identity(request: &IntakeRequest, opened: &OpenedMedia) -> SourceIdentity {
    let logical_name = request.logical_name.clone().unwrap_or_else(|| {
        request
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("audio.wav")
            .to_owned()
    });
    SourceIdentity {
        case_id: request.case_id.clone(),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        logical_name,
        media_type: opened.media_type.clone(),
        temporal_relation: request.temporal_relation,
        source_kind: opened.source_kind,
        sha256: opened.sha256.clone(),
        byte_length: opened.byte_length,
    }
}

fn analysis_options(
    request: &IntakeRequest,
    original: &DecodedAudio,
    transcript: &WhisperxTranscript,
    extractor: &str,
    extractor_version: String,
) -> MappingOptions {
    let file = file_rms(original);
    let mut channel_splits = Vec::new();
    let mut level_splits = Vec::new();
    for segment in &transcript.segments {
        if !segment.start.is_finite() || !segment.end.is_finite() {
            continue;
        }
        let start_ms = (segment.start * 1_000.0).round() as u64;
        let end_ms = (segment.end * 1_000.0).round() as u64;
        if let Some(split) = channel_split(original, start_ms, end_ms) {
            channel_splits.push(split);
        }
        if request.level_split
            && let Some(split) = level_split(original, start_ms, end_ms, file)
        {
            level_splits.push(split);
        }
    }
    MappingOptions {
        extractor: extractor.to_owned(),
        extractor_version,
        gap_ms: request.gap_ms,
        channel_splits,
        level_splits,
    }
}
