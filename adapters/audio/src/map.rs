//! Pure mapping from a WhisperX transcript plus speaker hypotheses to a batch.

use evidence_intake::{
    CaseId, ContentKind, ExtractionProvenance, NormalizedBatch, NormalizedContent,
    NormalizedSegment, NormalizedSource, ReviewState, SourceKind, TemporalRelation,
};

use crate::channel::{ChannelSide, ChannelSplit, LevelSplit};
use crate::whisperx::WhisperxTranscript;
use crate::{Error, Result};

/// Adapter name written on every WhisperX statement.
pub const EXTRACTOR_WHISPERX: &str = "whisperx";
/// Adapter name written on a statement from the in-process native engine.
pub const EXTRACTOR_NATIVE_WHISPER: &str = "franken_whisper";
/// Adapter name written on a stereo-channel observation.
pub const EXTRACTOR_CHANNEL: &str = "audio_channel_split";
/// Adapter name written on an opt-in level observation.
pub const EXTRACTOR_LEVEL: &str = "audio_level_split";
/// Adapter name written on a diarization-label observation.
pub const EXTRACTOR_DIARIZE: &str = "whisperx_diarize";
/// Adapter name written on a detected dropout.
pub const EXTRACTOR_GAP: &str = "audio_gap";

/// Version stamped on channel, level, and gap observations.
pub const ANALYSIS_VERSION: &str = "0.1.0";

/// Identity of the original recording the transcript was taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceIdentity {
    /// Case that already owns the production.
    pub case_id: CaseId,
    /// Existing production ledger identifier.
    pub production_id: String,
    /// Adapter-assigned source identifier.
    pub source_id: String,
    /// Filename or other display label.
    pub logical_name: String,
    /// MIME type of the untouched original.
    pub media_type: String,
    /// Whether the recording is contemporaneous with the event.
    pub temporal_relation: TemporalRelation,
    /// Audio file, or a video whose audio track was transcribed.
    pub source_kind: SourceKind,
    /// SHA-256 of the untouched original bytes.
    pub sha256: String,
    /// Length of the untouched original.
    pub byte_length: u64,
}

/// How the mapper should attach optional analysis next to each statement.
#[derive(Debug, Clone, Default)]
pub struct MappingOptions {
    /// Extractor name stored on each statement (`franken_whisper` or `whisperx`).
    pub extractor: String,
    /// Exact model / binary version string stored on each statement.
    pub extractor_version: String,
    /// Emit a `recording_gap` when successive segments are this far apart.
    pub gap_ms: u64,
    /// Channel hypotheses keyed to the same millisecond spans as the statements.
    pub channel_splits: Vec<ChannelSplit>,
    /// Opt-in near/far level hypotheses on those same spans.
    pub level_splits: Vec<LevelSplit>,
}

impl MappingOptions {
    /// Defaults used when only a transcript is available.
    pub fn new(extractor_version: impl Into<String>) -> Self {
        Self {
            extractor: EXTRACTOR_WHISPERX.to_owned(),
            extractor_version: extractor_version.into(),
            gap_ms: 2_000,
            channel_splits: Vec::new(),
            level_splits: Vec::new(),
        }
    }
}

/// Maps a WhisperX transcript onto a kernel `NormalizedBatch`.
///
/// Locators and `start_ms`/`end_ms` are the WhisperX times. The pipeline that
/// called WhisperX must have given it a working copy whose duration matches
/// the original, so those times address the original recording.
pub fn transcript_to_batch(
    identity: &SourceIdentity,
    transcript: &WhisperxTranscript,
    options: &MappingOptions,
) -> Result<NormalizedBatch> {
    if transcript.segments.is_empty() {
        return Err(Error::Transcript(
            "transcript produced no segments".to_owned(),
        ));
    }

    let mut segments = Vec::new();
    let mut previous_end: Option<u64> = None;

    for (index, item) in transcript.segments.iter().enumerate() {
        if !item.start.is_finite() || !item.end.is_finite() || item.end < item.start {
            return Err(Error::Mapping(format!(
                "segment {index} has inverted or non-finite times"
            )));
        }
        let start_ms = seconds_to_ms(item.start);
        let end_ms = seconds_to_ms(item.end);
        if end_ms < start_ms {
            return Err(Error::Mapping(format!(
                "segment {index} ends before it starts"
            )));
        }

        if let Some(previous) = previous_end
            && options.gap_ms > 0
            && start_ms.saturating_sub(previous) >= options.gap_ms
        {
            segments.push(gap_segment(
                &identity.source_id,
                segments.len(),
                previous,
                start_ms,
            ));
        }
        previous_end = Some(end_ms);

        let text = item.text.trim();
        if text.is_empty() {
            return Err(Error::Mapping(format!("segment {index} has no text")));
        }

        let mut content = Vec::new();
        let word_scores: Vec<f64> = item.words.iter().filter_map(|word| word.score).collect();
        content.push(statement(
            &identity.source_id,
            index,
            text,
            confidence_from(item.avg_logprob, &word_scores),
            &options.extractor,
            &options.extractor_version,
        ));

        if let Some(split) = options
            .channel_splits
            .iter()
            .find(|split| split.start_ms == start_ms && split.end_ms == end_ms)
        {
            content.push(channel_observation(&identity.source_id, index, split));
        }

        if let Some(split) = options
            .level_splits
            .iter()
            .find(|split| split.start_ms == start_ms && split.end_ms == end_ms)
        {
            content.push(level_observation(&identity.source_id, index, split));
        }

        if let Some(label) = item
            .speaker
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            content.push(diarize_observation(&identity.source_id, index, label));
        }

        segments.push(NormalizedSegment {
            id: format!("{}-seg-{index:04}", identity.source_id),
            locator: format_locator(start_ms, end_ms),
            page: None,
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            bounding_box: None,
            content,
        });
    }

    Ok(NormalizedBatch {
        case_id: identity.case_id.clone(),
        edges: Vec::new(),
        sources: vec![NormalizedSource {
            id: identity.source_id.clone(),
            production_id: identity.production_id.clone(),
            logical_name: identity.logical_name.clone(),
            media_type: identity.media_type.clone(),
            source_kind: identity.source_kind,
            temporal_relation: identity.temporal_relation,
            sha256: identity.sha256.clone(),
            byte_length: identity.byte_length,
            segments,
        }],
    })
}

/// Formats a millisecond span the way the fixtures do: `00:00:08.200–00:00:31.600`.
pub fn format_locator(start_ms: u64, end_ms: u64) -> String {
    format!("{}–{}", format_clock(start_ms), format_clock(end_ms))
}

/// Renders milliseconds as `HH:MM:SS.mmm`.
pub fn format_clock(ms: u64) -> String {
    let hours = ms / 3_600_000;
    let minutes = (ms % 3_600_000) / 60_000;
    let seconds = (ms % 60_000) / 1_000;
    let millis = ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02}.{millis:03}")
}

fn seconds_to_ms(seconds: f64) -> u64 {
    (seconds * 1_000.0).round() as u64
}

fn confidence_from(avg_logprob: Option<f64>, word_scores: &[f64]) -> Option<f64> {
    if !word_scores.is_empty() {
        let mean = word_scores.iter().sum::<f64>() / word_scores.len() as f64;
        return Some(mean.clamp(0.0, 1.0));
    }
    avg_logprob.map(|logprob| (logprob.exp()).clamp(0.0, 1.0))
}

fn machine(extractor: &str, version: &str, confidence: Option<f64>) -> ExtractionProvenance {
    ExtractionProvenance {
        extractor: extractor.to_owned(),
        version: version.to_owned(),
        machine_generated: true,
        confidence,
        review_state: ReviewState::Suggested,
    }
}

fn statement(
    source_id: &str,
    index: usize,
    text: &str,
    confidence: Option<f64>,
    extractor: &str,
    version: &str,
) -> NormalizedContent {
    NormalizedContent {
        id: format!("{source_id}-stmt-{index:04}"),
        kind: ContentKind::Statement,
        text: text.to_owned(),
        speaker_entity_id: None,
        attributed_to_entity_id: None,
        parent_content_id: None,
        raw_time: None,
        content_created_at: None,
        asserted_time: None,
        normalized_start: None,
        normalized_end: None,
        time_basis: None,
        location_text: None,
        extraction: machine(extractor, version, confidence),
    }
}

fn channel_observation(source_id: &str, index: usize, split: &ChannelSplit) -> NormalizedContent {
    let side = match split.side {
        ChannelSide::Left => "left",
        ChannelSide::Right => "right",
    };
    NormalizedContent {
        id: format!("{source_id}-ch-{index:04}"),
        kind: ContentKind::Observation,
        text: format!(
            "Channel energy predominantly {side} (L/R RMS {:.3} / {:.3}).",
            split.left_rms, split.right_rms
        ),
        speaker_entity_id: None,
        attributed_to_entity_id: None,
        parent_content_id: None,
        raw_time: None,
        content_created_at: None,
        asserted_time: None,
        normalized_start: None,
        normalized_end: None,
        time_basis: None,
        location_text: None,
        extraction: machine(EXTRACTOR_CHANNEL, ANALYSIS_VERSION, Some(split.confidence)),
    }
}

fn level_observation(source_id: &str, index: usize, split: &LevelSplit) -> NormalizedContent {
    NormalizedContent {
        id: format!("{source_id}-lvl-{index:04}"),
        kind: ContentKind::Observation,
        text: format!(
            "Near-field vs far-field level split; louder cluster labeled {} (window RMS {:.3}, file RMS {:.3}).",
            split.label, split.window_rms, split.file_rms
        ),
        speaker_entity_id: None,
        attributed_to_entity_id: None,
        parent_content_id: None,
        raw_time: None,
        content_created_at: None,
        asserted_time: None,
        normalized_start: None,
        normalized_end: None,
        time_basis: None,
        location_text: None,
        extraction: machine(EXTRACTOR_LEVEL, ANALYSIS_VERSION, Some(split.confidence)),
    }
}

fn diarize_observation(source_id: &str, index: usize, label: &str) -> NormalizedContent {
    NormalizedContent {
        id: format!("{source_id}-spk-{index:04}"),
        kind: ContentKind::Observation,
        text: format!("Diarization label {label}."),
        speaker_entity_id: None,
        attributed_to_entity_id: None,
        parent_content_id: None,
        raw_time: None,
        content_created_at: None,
        asserted_time: None,
        normalized_start: None,
        normalized_end: None,
        time_basis: None,
        location_text: None,
        extraction: machine(EXTRACTOR_DIARIZE, ANALYSIS_VERSION, None),
    }
}

fn gap_segment(source_id: &str, ordinal: usize, start_ms: u64, end_ms: u64) -> NormalizedSegment {
    NormalizedSegment {
        id: format!("{source_id}-gap-{ordinal:04}"),
        locator: format_locator(start_ms, end_ms),
        page: None,
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        bounding_box: None,
        content: vec![NormalizedContent {
            id: format!("{source_id}-gapc-{ordinal:04}"),
            kind: ContentKind::RecordingGap,
            text: format!(
                "No speech was aligned between {} and {}.",
                format_clock(start_ms),
                format_clock(end_ms)
            ),
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: machine(EXTRACTOR_GAP, ANALYSIS_VERSION, None),
        }],
    }
}
