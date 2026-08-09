//! Human review workflow.
//!
//! Nothing in this module decides whether evidence is true. It records that a
//! named person looked at a specific record, what they concluded about the
//! record's reliability as an extraction, and — for verification — which
//! original they checked it against.

use serde::{Deserialize, Serialize};

use crate::{NodeKind, ReviewState};

/// A record type that carries a human review state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTarget {
    /// An extracted or hand-entered evidentiary content item.
    Content,
    /// An immutable source record.
    Source,
    /// A typed relationship between two nodes.
    Edge,
    /// A contested proposition.
    Proposition,
    /// A timeline event.
    Event,
}

impl ReviewTarget {
    /// Returns this target as a graph node.
    ///
    /// Every review target is a node; not every node is reviewable. Keeping the
    /// mapping in one place means the table a reviewer writes to and the table
    /// an author links to can never drift apart.
    pub const fn node_kind(self) -> NodeKind {
        match self {
            Self::Content => NodeKind::Content,
            Self::Source => NodeKind::Source,
            Self::Edge => NodeKind::Edge,
            Self::Proposition => NodeKind::Proposition,
            Self::Event => NodeKind::Event,
        }
    }

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        self.node_kind().as_str()
    }

    /// Returns the table holding this target's review state.
    pub(crate) const fn table(self) -> &'static str {
        self.node_kind().table()
    }
}

impl From<ReviewTarget> for NodeKind {
    fn from(value: ReviewTarget) -> Self {
        value.node_kind()
    }
}

/// A reviewer's decision about one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewDecision {
    /// The record type being reviewed.
    pub target: ReviewTarget,
    /// Identifier of the record being reviewed.
    pub target_id: String,
    /// The state the reviewer is moving the record into.
    pub to_state: ReviewState,
    /// The named person accountable for the decision.
    pub actor: String,
    /// Written reason. Required to reject, and to verify a record that has no
    /// single original locator.
    pub basis: Option<String>,
    /// The exact original locator the reviewer opened while verifying.
    pub verified_against_locator: Option<String>,
}

/// One appended entry in the review audit trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewEvent {
    /// Audit-entry identifier.
    pub id: String,
    /// Record type reviewed.
    pub target_kind: String,
    /// Record identifier.
    pub target_id: String,
    /// State the record held before the decision.
    pub from_state: String,
    /// State the reviewer moved it into.
    pub to_state: String,
    /// Named human reviewer.
    pub actor: String,
    /// Written reason, when one was given or required.
    pub basis: Option<String>,
    /// Original locator cited during verification.
    pub verified_against_locator: Option<String>,
    /// When the decision was recorded.
    pub decided_at: String,
}

/// One record still awaiting a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewQueueItem {
    /// Record type.
    pub target_kind: String,
    /// Record identifier.
    pub target_id: String,
    /// Current intake state, either `unreviewed` or `suggested`.
    pub review_state: String,
    /// Whether an adapter produced this record.
    pub machine_generated: bool,
    /// Short human-readable description of what needs review.
    pub summary: String,
    /// Exact original locator to open, when the record has one.
    pub locator: Option<String>,
    /// Extraction adapter, when the record came from one.
    pub extractor: Option<String>,
}

/// Returns whether a reviewer may move a record between two states.
///
/// `unreviewed` and `suggested` are intake states produced by import, so no
/// decision may return a record to them: a suggestion that was looked at and
/// found wanting is `rejected`, not un-suggested. Every other move is allowed,
/// including reinstating a rejected item or withdrawing a verification, because
/// review is continuous and later material can undo an earlier reading.
pub(crate) const fn transition_allowed(from: ReviewState, to: ReviewState) -> bool {
    match to {
        ReviewState::Unreviewed | ReviewState::Suggested => false,
        ReviewState::Reviewed | ReviewState::Verified | ReviewState::Rejected => !matches!(
            (from, to),
            (ReviewState::Reviewed, ReviewState::Reviewed)
                | (ReviewState::Verified, ReviewState::Verified)
                | (ReviewState::Rejected, ReviewState::Rejected)
        ),
    }
}
