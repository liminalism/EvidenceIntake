//! Route the original container's audio through the audio adapter and merge
//! the resulting statements into the same batch as the scenes.
//!
//! The audio adapter already accepts a video container: it hashes the original
//! container bytes as a video source and lets ffmpeg pull the soundtrack, so
//! the times it returns address the original video timeline. This module only
//! has to prove the two adapters looked at the *same* original and then fold
//! one source's segments into the other.
//!
//! Speaker labels stay anonymous. Diarization arrives as a `SPEAKER_nn`
//! observation next to the statement; tying that label to a person is
//! authoring work, never adapter output.

use std::collections::HashSet;
use std::path::Path;

use evidence_audio::{IntakeRequest, TranscriptBackend};
use evidence_intake::NormalizedBatch;

use crate::map::VideoIdentity;
use crate::{Error, Result, SceneRequest};

/// Default silence, in milliseconds, that becomes a `recording_gap`.
pub const DEFAULT_SPEECH_GAP_MS: u64 = 2_000;

/// How the audio adapter should treat the soundtrack it pulls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundtrackOptions {
    /// Apply the telephone band-pass to the working copy before ASR.
    pub phone_band: bool,
    /// Emit the opt-in near/far level observations.
    pub level_split: bool,
    /// Minimum silence, in milliseconds, that becomes a `recording_gap`.
    pub gap_ms: u64,
}

impl Default for SoundtrackOptions {
    fn default() -> Self {
        Self {
            phone_band: false,
            level_split: false,
            gap_ms: DEFAULT_SPEECH_GAP_MS,
        }
    }
}

/// Where the spoken words come from during [`crate::analyze`].
#[derive(Clone, Copy)]
pub enum SoundtrackInput<'a> {
    /// Skip the soundtrack entirely.
    None,
    /// Map an already-produced WhisperX JSON document against the original.
    Json {
        /// Path to the WhisperX JSON document.
        path: &'a Path,
        /// Version stamped on every mapped statement.
        version: &'a str,
    },
    /// Run a live transcription backend on the pulled soundtrack.
    Live(&'a dyn TranscriptBackend),
}

/// Builds the audio adapter's request from the video job.
///
/// Same case, production, source identifier, and file: both adapters are
/// looking at one original, which is what lets the batches be merged.
pub fn soundtrack_request(request: &SceneRequest, options: SoundtrackOptions) -> IntakeRequest {
    IntakeRequest {
        case_id: request.case_id.clone(),
        production_id: request.production_id.clone(),
        source_id: request.source_id.clone(),
        path: request.path.clone(),
        logical_name: request.logical_name.clone(),
        temporal_relation: request.temporal_relation,
        phone_band: options.phone_band,
        level_split: options.level_split,
        gap_ms: options.gap_ms,
    }
}

/// Runs the audio adapter over the video's soundtrack.
///
/// Returns `None` for [`SoundtrackInput::None`]. Both other arms hash the
/// original container themselves, inside the audio adapter, so
/// [`merge_soundtrack`] can check the two hashes against each other.
pub fn soundtrack_batch(
    request: &SceneRequest,
    input: SoundtrackInput<'_>,
    options: SoundtrackOptions,
) -> Result<Option<NormalizedBatch>> {
    let intake = soundtrack_request(request, options);
    match input {
        SoundtrackInput::None => Ok(None),
        SoundtrackInput::Json { path, version } => {
            Ok(Some(evidence_audio::map_json_file(&intake, path, version)?))
        }
        SoundtrackInput::Live(backend) => Ok(Some(evidence_audio::transcribe(&intake, backend)?)),
    }
}

/// Folds a soundtrack batch into the video batch under one source.
///
/// Refuses when the two batches are not about the same original: a different
/// case, no matching source, or a hash or length the video adapter did not
/// compute. That check is the whole guarantee — a statement is only allowed to
/// address the video timeline if the audio adapter read the same bytes.
/// Colliding segment identifiers are refused too, since the store would take
/// the second one as a rewrite of the first.
///
/// Nothing is rewritten on the way in. Review states, extractor names, raw
/// time, and `speaker_entity_id` (which stays `None`; the anonymous
/// `SPEAKER_nn` label lives in the diarization observation's text) pass
/// through exactly as the audio adapter wrote them.
pub fn merge_soundtrack(
    batch: &mut NormalizedBatch,
    identity: &VideoIdentity,
    soundtrack: NormalizedBatch,
) -> Result<()> {
    if soundtrack.case_id != batch.case_id {
        return Err(Error::Mapping(format!(
            "soundtrack is for case `{}` but the scenes are for case `{}`",
            soundtrack.case_id, batch.case_id
        )));
    }

    let mut spoken = None;
    let mut others = Vec::new();
    for source in soundtrack.sources {
        if source.id == identity.source_id {
            spoken = Some(source);
        } else {
            others.push(source);
        }
    }
    let spoken = spoken.ok_or_else(|| {
        Error::Mapping(format!(
            "soundtrack has no source `{}` to merge onto",
            identity.source_id
        ))
    })?;

    if spoken.sha256 != identity.sha256 {
        return Err(Error::Mapping(format!(
            "soundtrack hashed a different original: {} is not {}",
            spoken.sha256, identity.sha256
        )));
    }
    if spoken.byte_length != identity.byte_length {
        return Err(Error::Mapping(format!(
            "soundtrack hashed a different original: {} bytes is not {}",
            spoken.byte_length, identity.byte_length
        )));
    }
    if let Some(named) = spoken
        .segments
        .iter()
        .flat_map(|segment| &segment.content)
        .find(|content| content.speaker_entity_id.is_some())
    {
        return Err(Error::Mapping(format!(
            "soundtrack content `{}` names a speaker; speaker labels stay anonymous",
            named.id
        )));
    }

    let video = batch
        .sources
        .iter_mut()
        .find(|source| source.id == identity.source_id)
        .ok_or_else(|| Error::Mapping("batch has no original video source".to_owned()))?;

    let held: HashSet<&str> = video
        .segments
        .iter()
        .map(|segment| segment.id.as_str())
        .collect();
    if let Some(clash) = spoken
        .segments
        .iter()
        .find(|segment| held.contains(segment.id.as_str()))
    {
        return Err(Error::Mapping(format!(
            "soundtrack segment `{}` collides with a segment already on the video",
            clash.id
        )));
    }

    video.segments.extend(spoken.segments);
    batch.sources.extend(others);
    batch.edges.extend(soundtrack.edges);
    Ok(())
}
