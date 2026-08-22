//! Versioned, adapter-neutral process contract for persistent intake jobs.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Current request/result schema.
pub const ADAPTER_PROTOCOL_VERSION: u32 = 1;

/// Immutable identity of a checksum-pinned broker model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    /// Installed model-pack identifier.
    pub id: String,
    /// Exact model/export revision.
    pub revision: String,
}

/// Relationship between creation of a source and the investigated event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRelationArg {
    /// Captured during the event.
    Contemporaneous,
    /// Created after the event.
    AfterEvent,
    /// Contains both contemporaneous and later material.
    Mixed,
    /// Not yet classified by a person.
    Unknown,
}

/// Video processing depth selected by the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoTier {
    /// Scenes, recording gaps, clock OCR and soundtrack transcription.
    Tier1,
    /// Tier 1 plus embeddings, detections and bounded scene captions.
    Overnight,
}

/// Modality-specific, fully resolved execution profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "modality", rename_all = "snake_case")]
#[allow(
    clippy::large_enum_variant,
    reason = "serialized process contract parsed once per durable job"
)]
pub enum AdapterProfile {
    /// Lege PDF/DocIR processing through brokered TensorRT OCR.
    Document {
        /// Packaged Lege CLI.
        lege_ocr: PathBuf,
        /// Lege-protocol-to-Evidence-broker bridge executable.
        broker_bridge: PathBuf,
        /// Local Evidence broker endpoint.
        broker_endpoint: String,
        /// OCR model selected from the broker.
        ocr: ModelRef,
    },
    /// Audio transcription and deterministic signal observations.
    Audio {
        /// Local Evidence broker endpoint.
        broker_endpoint: String,
        /// Whisper/diarization model pack.
        whisper: ModelRef,
        /// Language hint.
        language: String,
        /// Apply telephone band limiting to the working copy.
        phone_band: bool,
        /// Emit near/far level observations.
        level_split: bool,
        /// Minimum recording hole retained as a gap.
        gap_ms: u64,
    },
    /// Video scene, soundtrack and optional overnight visual processing.
    Video {
        /// Local Evidence broker endpoint.
        broker_endpoint: String,
        /// Selected processing tier.
        tier: VideoTier,
        /// Clock OCR model.
        ocr: ModelRef,
        /// Soundtrack transcription model.
        whisper: ModelRef,
        /// Embedding model required by the overnight tier.
        embedding: Option<ModelRef>,
        /// Detector model required by the overnight tier.
        detector: Option<ModelRef>,
        /// Caption model required by the overnight tier.
        caption: Option<ModelRef>,
        /// Language hint for OCR and speech.
        language: String,
        /// ffmpeg scene-cut threshold.
        threshold: f64,
        /// Minimum recording gap.
        gap_ms: u64,
        /// Longest interval between retained finder frames.
        sample_gap_ms: u64,
        /// Deduplication interval for adjacent samples.
        sample_dedup_ms: u64,
        /// Minimum detector confidence.
        detector_confidence: f64,
        /// Bounded caption instruction.
        caption_prompt: String,
    },
}

/// One durable request consumed by an adapter executable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdapterJobRequest {
    /// Must equal [`ADAPTER_PROTOCOL_VERSION`].
    pub schema_version: u32,
    /// Stable intake job identifier.
    pub job_id: String,
    /// Existing case identifier.
    pub case_id: String,
    /// Existing production identifier.
    pub production_id: String,
    /// Stable source identifier reserved for this import.
    pub source_id: String,
    /// Untouched original, referenced in place.
    pub original_path: PathBuf,
    /// SHA-256 computed before the job is queued.
    pub original_sha256: String,
    /// Byte length computed before the job is queued.
    pub original_byte_length: u64,
    /// Display name shown in the case.
    pub logical_name: String,
    /// Creation relationship selected by the operator.
    pub temporal_relation: TemporalRelationArg,
    /// Durable directory owned by this job attempt.
    pub artifacts_dir: PathBuf,
    /// Fully resolved modality profile.
    pub profile: AdapterProfile,
}

impl AdapterJobRequest {
    /// Validate identity and profile invariants before touching the original.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ADAPTER_PROTOCOL_VERSION {
            return Err(Error::IncompatibleVersion {
                expected: ADAPTER_PROTOCOL_VERSION,
                actual: self.schema_version,
            });
        }
        for (label, value) in [
            ("job id", self.job_id.as_str()),
            ("case id", self.case_id.as_str()),
            ("production id", self.production_id.as_str()),
            ("source id", self.source_id.as_str()),
            ("logical name", self.logical_name.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(Error::Invalid(format!("{label} is empty")));
            }
        }
        if !self.original_path.is_file() {
            return Err(Error::Invalid(format!(
                "original does not exist: {}",
                self.original_path.display()
            )));
        }
        validate_hash(&self.original_sha256)?;
        if self.original_byte_length == 0 {
            return Err(Error::Invalid("original is empty".to_owned()));
        }
        if self.artifacts_dir.as_os_str().is_empty() {
            return Err(Error::Invalid("artifact directory is empty".to_owned()));
        }
        validate_profile(&self.profile)
    }
}

/// Progress-event kind written as one JSON object per stdout line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterEventKind {
    /// A stage began.
    Started,
    /// Work advanced.
    Progress,
    /// A stage completed.
    Completed,
}

/// One machine-readable adapter progress event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterEvent {
    /// Protocol schema version.
    pub schema_version: u32,
    /// Matching job identifier.
    pub job_id: String,
    /// Stable stage name.
    pub stage: String,
    /// Event kind.
    pub kind: AdapterEventKind,
    /// Completed units, when measurable.
    pub completed: Option<u64>,
    /// Total units, when known.
    pub total: Option<u64>,
    /// Short operator-facing message.
    pub message: String,
}

/// A retained output associated with a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterArtifact {
    /// Stable artifact kind (`docir`, `qa`, `still`, `batch`, `index`, `log`).
    pub kind: String,
    /// Artifact path inside the attempt directory.
    pub path: PathBuf,
    /// Optional lowercase SHA-256.
    pub sha256: Option<String>,
}

/// A source path the application may reopen or relink by hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterSourceLocation {
    /// Imported source identifier.
    pub source_id: String,
    /// Referenced original or managed derived artifact.
    pub path: PathBuf,
    /// SHA-256 matching the imported source.
    pub sha256: String,
    /// Byte length matching the imported source.
    pub byte_length: u64,
}

/// Atomic result descriptor written after all adapter outputs are durable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterResultManifest {
    /// Protocol schema version.
    pub schema_version: u32,
    /// Matching job identifier.
    pub job_id: String,
    /// Normalized batch JSON inside the attempt directory.
    pub batch_path: PathBuf,
    /// Optional keyframe finder index inside the attempt directory.
    pub keyframe_index_path: Option<PathBuf>,
    /// Paths for originals and derived source artifacts.
    pub source_locations: Vec<AdapterSourceLocation>,
    /// Retained processing artifacts.
    pub artifacts: Vec<AdapterArtifact>,
}

impl AdapterResultManifest {
    /// Validate version, job correlation and artifact confinement.
    pub fn validate_for(&self, request: &AdapterJobRequest) -> Result<()> {
        if self.schema_version != ADAPTER_PROTOCOL_VERSION {
            return Err(Error::IncompatibleVersion {
                expected: ADAPTER_PROTOCOL_VERSION,
                actual: self.schema_version,
            });
        }
        if self.job_id != request.job_id {
            return Err(Error::Invalid(format!(
                "result job `{}` does not match request `{}`",
                self.job_id, request.job_id
            )));
        }
        require_confined_file(&request.artifacts_dir, &self.batch_path)?;
        if let Some(index) = &self.keyframe_index_path {
            require_confined_file(&request.artifacts_dir, index)?;
        }
        for artifact in &self.artifacts {
            if artifact.kind.trim().is_empty() {
                return Err(Error::Invalid("artifact kind is empty".to_owned()));
            }
            require_confined_file(&request.artifacts_dir, &artifact.path)?;
        }
        if !self
            .source_locations
            .iter()
            .any(|location| location.source_id == request.source_id)
        {
            return Err(Error::Invalid(
                "result does not locate the requested original source".to_owned(),
            ));
        }
        for location in &self.source_locations {
            validate_hash(&location.sha256)?;
            if !location.path.is_file() {
                return Err(Error::Invalid(format!(
                    "source location does not exist: {}",
                    location.path.display()
                )));
            }
        }
        Ok(())
    }

    /// Write a manifest via a sibling temporary file and atomic rename.
    pub fn write_atomic(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| Error::Invalid("manifest has no parent directory".to_owned()))?;
        std::fs::create_dir_all(parent)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

fn validate_profile(profile: &AdapterProfile) -> Result<()> {
    let validate_model = |model: &ModelRef| {
        if model.id.trim().is_empty() || model.revision.trim().is_empty() {
            Err(Error::Invalid(
                "model id and revision are required".to_owned(),
            ))
        } else {
            Ok(())
        }
    };
    match profile {
        AdapterProfile::Document { ocr, .. } => validate_model(ocr),
        AdapterProfile::Audio {
            whisper,
            language,
            gap_ms,
            ..
        } => {
            validate_model(whisper)?;
            if language.trim().is_empty() || *gap_ms == 0 {
                return Err(Error::Invalid(
                    "audio language and nonzero gap are required".to_owned(),
                ));
            }
            Ok(())
        }
        AdapterProfile::Video {
            tier,
            ocr,
            whisper,
            embedding,
            detector,
            caption,
            threshold,
            gap_ms,
            sample_gap_ms,
            detector_confidence,
            ..
        } => {
            validate_model(ocr)?;
            validate_model(whisper)?;
            if *tier == VideoTier::Overnight {
                for model in [embedding, detector, caption] {
                    validate_model(model.as_ref().ok_or_else(|| {
                        Error::Invalid(
                            "overnight video requires embedding, detector and caption models"
                                .to_owned(),
                        )
                    })?)?;
                }
            }
            if !(0.0..=1.0).contains(threshold)
                || !(0.0..=1.0).contains(detector_confidence)
                || *gap_ms == 0
                || *sample_gap_ms == 0
            {
                return Err(Error::Invalid("invalid video thresholds".to_owned()));
            }
            Ok(())
        }
    }
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::Invalid(format!("invalid SHA-256 `{hash}`")));
    }
    Ok(())
}

fn require_confined_file(root: &Path, candidate: &Path) -> Result<()> {
    let root = root.canonicalize()?;
    let candidate = candidate.canonicalize()?;
    if !candidate.starts_with(&root) || !candidate.is_file() {
        return Err(Error::Invalid(format!(
            "artifact escapes job directory: {}",
            candidate.display()
        )));
    }
    Ok(())
}

/// Adapter process-contract failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An incompatible request or result version.
    #[error("adapter protocol version {actual} is incompatible; expected {expected}")]
    IncompatibleVersion {
        /// Supported version.
        expected: u32,
        /// Received version.
        actual: u32,
    },
    /// Invalid identity, profile, path or result.
    #[error("invalid adapter job: {0}")]
    Invalid(String),
    /// Filesystem failure.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON failure.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Protocol result.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overnight_video_requires_all_three_optional_models() {
        let profile = AdapterProfile::Video {
            broker_endpoint: "evidence-trt".into(),
            tier: VideoTier::Overnight,
            ocr: model("ocr"),
            whisper: model("whisper"),
            embedding: Some(model("siglip")),
            detector: None,
            caption: Some(model("qwen")),
            language: "en".into(),
            threshold: 0.3,
            gap_ms: 2_000,
            sample_gap_ms: 5_000,
            sample_dedup_ms: 250,
            detector_confidence: 0.25,
            caption_prompt: "Describe only what is visible".into(),
        };
        assert!(validate_profile(&profile).is_err());
    }

    fn model(id: &str) -> ModelRef {
        ModelRef {
            id: id.into(),
            revision: "revision".into(),
        }
    }

    #[test]
    fn request_round_trip_and_version_rejection_are_fail_closed() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let original = temporary.path().join("recording.wav");
        std::fs::write(&original, b"audio").expect("original");
        let request = AdapterJobRequest {
            schema_version: ADAPTER_PROTOCOL_VERSION,
            job_id: "job".into(),
            case_id: "case".into(),
            production_id: "production".into(),
            source_id: "source".into(),
            original_path: original,
            original_sha256: "a".repeat(64),
            original_byte_length: 5,
            logical_name: "recording.wav".into(),
            temporal_relation: TemporalRelationArg::Unknown,
            artifacts_dir: temporary.path().join("artifacts"),
            profile: AdapterProfile::Audio {
                broker_endpoint: "evidence-trt".into(),
                whisper: model("whisper"),
                language: "en".into(),
                phone_band: false,
                level_split: false,
                gap_ms: 2_000,
            },
        };
        let decoded: AdapterJobRequest =
            serde_json::from_slice(&serde_json::to_vec(&request).expect("encode")).expect("decode");
        assert_eq!(decoded, request);
        decoded.validate().expect("current request");
        let mut incompatible = decoded;
        incompatible.schema_version += 1;
        assert!(matches!(
            incompatible.validate(),
            Err(Error::IncompatibleVersion { .. })
        ));
    }
}
