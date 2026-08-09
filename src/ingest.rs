//! Adapter-neutral contract for already-extracted evidence.
//!
//! OCR, speech recognition, diarization, and video models are deliberately
//! outside this crate. They hand the kernel normalized records through these
//! types while retaining exact locators in the original evidence.

use serde::{Deserialize, Serialize};

use crate::{CaseId, ContentKind, ReviewState, SourceKind, TemporalRelation};

/// One source and all normalized records extracted from it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedSource {
    /// Adapter-assigned stable source identifier.
    pub id: String,
    /// Existing production ledger identifier.
    pub production_id: String,
    /// Filename or other display label.
    pub logical_name: String,
    /// MIME media type of the untouched original.
    pub media_type: String,
    /// Broad original modality.
    pub source_kind: SourceKind,
    /// Whether the source was captured during or created after the event.
    pub temporal_relation: TemporalRelation,
    /// Lowercase or uppercase hexadecimal SHA-256 of the original bytes.
    pub sha256: String,
    /// Length of the untouched original.
    pub byte_length: u64,
    /// Segments and content extracted from this source.
    pub segments: Vec<NormalizedSegment>,
}

/// An exact address within an original source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedSegment {
    /// Adapter-assigned stable segment identifier.
    pub id: String,
    /// Human-readable exact locator.
    pub locator: String,
    /// One-indexed document page.
    pub page: Option<u32>,
    /// Start offset in original audio or video.
    pub start_ms: Option<u64>,
    /// End offset in original audio or video.
    pub end_ms: Option<u64>,
    /// Optional `[x, y, width, height]` coordinates in original page space.
    pub bounding_box: Option<[f64; 4]>,
    /// Statements, observations, and assertions present in this segment.
    pub content: Vec<NormalizedContent>,
}

/// Evidentiary content produced by a human or extraction adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedContent {
    /// Adapter-assigned stable content identifier.
    pub id: String,
    /// Epistemic category; this is not a truth assessment.
    pub kind: ContentKind,
    /// Verbatim transcript/OCR text or bounded model observation.
    pub text: String,
    /// Resolved speaker or author, if known.
    pub speaker_entity_id: Option<String>,
    /// Person to whom a reporting-chain statement is attributed.
    pub attributed_to_entity_id: Option<String>,
    /// Parent content when this is a nested report of another statement.
    pub parent_content_id: Option<String>,
    /// Unmodified source time expression or device timestamp.
    pub raw_time: Option<String>,
    /// When the statement, report, or observation itself was created.
    pub content_created_at: Option<String>,
    /// Time claimed in the content, distinct from recording time.
    pub asserted_time: Option<String>,
    /// Proposed normalized interval start.
    pub normalized_start: Option<String>,
    /// Proposed normalized interval end.
    pub normalized_end: Option<String>,
    /// Human-reviewable basis for normalization.
    pub time_basis: Option<String>,
    /// Location as stated or observed.
    pub location_text: Option<String>,
    /// Extraction provenance.
    pub extraction: ExtractionProvenance,
}

/// Provenance and review boundary for an extracted record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionProvenance {
    /// Adapter or human workflow name.
    pub extractor: String,
    /// Exact adapter/model version.
    pub version: String,
    /// Whether the record was machine-generated.
    pub machine_generated: bool,
    /// Adapter confidence, when meaningful.
    pub confidence: Option<f64>,
    /// Current human-review state.
    pub review_state: ReviewState,
}

/// An atomic delivery from one or more extraction adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedBatch {
    /// Existing case that owns every source in the batch.
    pub case_id: CaseId,
    /// Originals and their normalized extracted content.
    pub sources: Vec<NormalizedSource>,
}
