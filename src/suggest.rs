//! Assisted collation: deterministic suggestions that wait for a person.
//!
//! Everything here proposes and nothing here concludes. An analyzer reads the
//! graph a person already built and points at pairs worth a second look — two
//! events that overlap in time, two excerpts standing on opposite sides of the
//! same proposition, one witness whose two accounts pull apart. Each proposal
//! enters as `suggested`, joins the review queue, and stays visibly unconfirmed
//! until a named person acts on it.
//!
//! There is no model here, and there is no score. These are rules over data the
//! case already contains: interval arithmetic and joins. That is deliberate.
//! A suggestion a defender cannot reconstruct is one they cannot argue with, and
//! the rationale on every proposal states the reason in full.
//!
//! What an analyzer may never do: mark anything reviewed or verified, merge two
//! records, alter a record a person wrote, or re-propose something a reviewer
//! already rejected.

use serde::{Deserialize, Serialize};

use crate::AuthoredLink;

/// Prefix marking an edge as machine-proposed rather than human-authored.
///
/// The review queue reads this to sort suggestions ahead of hand-entered work
/// and to name what proposed them, the same way content names its extractor.
pub(crate) const SUGGESTER_PREFIX: &str = "suggest:";

/// One deterministic analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Events in different timeline lanes whose normalized intervals overlap.
    ///
    /// Lanes never collapse, so this proposes only that two accounts describe
    /// overlapping time — never that either is the correct one.
    TemporalOverlap,
    /// Two excerpts standing on opposite sides of the same proposition.
    ///
    /// Pairs sharing an attributed witness are left to
    /// [`SuggestionKind::ConflictingAttribution`]: one account changing is a
    /// question about the witness, not about the facts.
    ContradictionCandidate,
    /// One witness whose two attributed accounts pull opposite ways.
    ConflictingAttribution,
    /// Two people in the case who may be one person.
    ///
    /// Proposes `possibly_same_person`, which is a question and not a merge:
    /// the two records stay two records until a person says otherwise, and even
    /// then nothing in this kernel collapses them.
    DuplicateEntity,
    /// Propositions resting on nothing a reader could open.
    UnsupportedProposition,
    /// Evidence-backed propositions tied to no element of any charge.
    UnmappedProposition,
    /// References to evidence that resolve to no source in the case.
    UnresolvedReference,
    /// Sources placing the same proposition at materially different times.
    ClockDisagreement,
}

impl SuggestionKind {
    /// Returns every analyzer, which is what a bare `suggest` runs.
    pub const ALL: [Self; 8] = [
        Self::TemporalOverlap,
        Self::ContradictionCandidate,
        Self::ConflictingAttribution,
        Self::DuplicateEntity,
        Self::UnsupportedProposition,
        Self::UnmappedProposition,
        Self::UnresolvedReference,
        Self::ClockDisagreement,
    ];

    /// Returns the stable analyzer name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TemporalOverlap => "temporal-overlap",
            Self::ContradictionCandidate => "contradiction-candidate",
            Self::ConflictingAttribution => "conflicting-attribution",
            Self::DuplicateEntity => "duplicate-entity",
            Self::UnsupportedProposition => "unsupported-proposition",
            Self::UnmappedProposition => "unmapped-proposition",
            Self::UnresolvedReference => "unresolved-reference",
            Self::ClockDisagreement => "clock-disagreement",
        }
    }

    /// Returns whether this analyzer proposes relationships or reports gaps.
    ///
    /// A proposal is a claim that needs a decision, so it is written and joins
    /// the review queue. A finding is a gap that needs work — nothing to
    /// confirm, only something to do — so it is derived on every run and stored
    /// nowhere. When the gap closes, the finding stops appearing.
    pub const fn proposes_relationships(self) -> bool {
        matches!(
            self,
            Self::TemporalOverlap
                | Self::ContradictionCandidate
                | Self::ConflictingAttribution
                | Self::DuplicateEntity
        )
    }

    /// Returns the value written to an edge's `created_by`.
    ///
    /// Versioned like an extractor: a later analyzer that proposes differently
    /// is a different author, and the trail should say which one ran.
    pub(crate) fn attribution(self) -> String {
        format!("{SUGGESTER_PREFIX}{}@1", self.as_str())
    }
}

/// A gap in the case, reported rather than proposed.
///
/// Findings are derived on every run and stored nowhere. There is nothing here
/// to confirm or reject — a proposition tied to no element is not a claim a
/// reviewer can disagree with, it is work someone has not done yet. When the
/// work is done the finding stops appearing, which is the only dismissal it
/// needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Node type the finding is about.
    pub subject_kind: String,
    /// Identifier of that record.
    pub subject_id: String,
    /// The record as a person would recognize it.
    pub subject: String,
    /// What is missing, in a sentence.
    pub summary: String,
}

/// What one analyzer produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AnalyzerReport {
    /// Which analyzer ran.
    pub analyzer: String,
    /// Relationships newly proposed, each awaiting a person.
    pub proposed: Vec<AuthoredLink>,
    /// Candidates the analyzer found but did not write, because the claim was
    /// already in the case — asserted by a person, proposed by an earlier run,
    /// or rejected by a reviewer who does not need to be asked twice.
    pub already_recorded: u32,
    /// Gaps reported. Never written, never reviewed.
    pub findings: Vec<Finding>,
}

/// The result of a suggestion run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestionRun {
    /// Case the analyzers ran over.
    pub case_id: String,
    /// One report per analyzer, in the order they ran.
    pub analyzers: Vec<AnalyzerReport>,
    /// Total relationships newly proposed.
    pub proposed: u32,
    /// Total candidates skipped because the case already held the claim.
    pub already_recorded: u32,
    /// Total gaps reported.
    pub findings: u32,
}
