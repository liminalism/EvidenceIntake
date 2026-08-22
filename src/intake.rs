//! Persistent application-level intake jobs and source locations.

use std::path::PathBuf;

use serde::Serialize;

use crate::CaseId;

/// Durable intake lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntakeJobState {
    /// Waiting for the local coordinator.
    Queued,
    /// Adapter process is active.
    Running,
    /// Adapter output is being validated and imported.
    Importing,
    /// Result and locations were committed.
    Completed,
    /// Attempt ended with a visible error.
    Failed,
    /// The process ended without a terminal update, normally on application restart.
    Interrupted,
}

impl IntakeJobState {
    /// Stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Importing => "importing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "importing" => Some(Self::Importing),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }
}

/// New persistent job created from a validated adapter request.
#[derive(Debug, Clone)]
pub struct NewIntakeJob {
    /// Serialized [`evidence_adapter_protocol::AdapterJobRequest`].
    pub request_json: String,
}

/// One queue row returned to application frontends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntakeJob {
    /// Stable job identifier.
    pub id: String,
    /// Owning case.
    pub case_id: CaseId,
    /// Owning production.
    pub production_id: String,
    /// Reserved source identifier.
    pub source_id: String,
    /// `document`, `audio`, or `video`.
    pub modality: String,
    /// Stable profile label.
    pub profile: String,
    /// Referenced original.
    pub original_path: PathBuf,
    /// Preflight SHA-256 of the original.
    pub original_sha256: String,
    /// Preflight byte length of the original.
    pub original_byte_length: u64,
    /// Source display name.
    pub logical_name: String,
    /// Exact adapter request JSON.
    pub request_json: String,
    /// Attempt artifact directory.
    pub artifact_dir: PathBuf,
    /// Current lifecycle state.
    pub state: IntakeJobState,
    /// One-based attempt number.
    pub attempt: u32,
    /// Current adapter stage.
    pub stage: Option<String>,
    /// Completed stage units.
    pub progress_completed: Option<u64>,
    /// Total stage units.
    pub progress_total: Option<u64>,
    /// Current operator-facing message.
    pub message: Option<String>,
    /// Terminal diagnostic for failed/interrupted work.
    pub error: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Attempt start timestamp.
    pub started_at: Option<String>,
    /// Terminal timestamp.
    pub finished_at: Option<String>,
}

/// One retained processing artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntakeArtifact {
    /// Owning job.
    pub job_id: String,
    /// Stable artifact kind.
    pub kind: String,
    /// Local artifact path.
    pub path: PathBuf,
    /// Optional SHA-256.
    pub sha256: Option<String>,
}

/// Local path registered for an imported source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceLocation {
    /// Source identifier.
    pub source_id: String,
    /// Owning case.
    pub case_id: CaseId,
    /// Current reference-in-place path.
    pub path: PathBuf,
    /// Last successful hash/length verification timestamp.
    pub last_verified_at: String,
}
