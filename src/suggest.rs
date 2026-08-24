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

use crate::{AuthoredLink, ContentInterpretation, SourceProfile};

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
    /// Source-profile values inherited at read time rather than copied per passage.
    InheritProfile,
    /// Cue verbs propose a reported-statement form.
    ReportedStatement,
    /// Balanced quotation marks propose a quoted sub-span.
    QuotedStatement,
    /// A numeric value with a measurement unit proposes measured-result form.
    MeasuredResult,
    /// Statute/probable-cause language proposes official characterization.
    OfficialCharacterization,
    /// Evidence-reference language proposes reference form.
    EvidenceReference,
    /// A clock-shaped token proposes asserted time.
    AssertedClock,
    /// Page-one report-date language proposes a source creation claim.
    HeaderDate,
    /// A diarization observation proposes a structural speaker candidate.
    DiarizationSpeaker,
    /// A shared exact token run proposes a quote/summary dependency.
    CrossDocumentEcho,
    /// A reviewed shared date/location anchor proposes same-occurrence review.
    SharedAnchor,
}

impl SuggestionKind {
    /// Returns every analyzer, which is what a bare `suggest` runs.
    pub const ALL: [Self; 19] = [
        Self::TemporalOverlap,
        Self::ContradictionCandidate,
        Self::ConflictingAttribution,
        Self::DuplicateEntity,
        Self::UnsupportedProposition,
        Self::UnmappedProposition,
        Self::UnresolvedReference,
        Self::ClockDisagreement,
        Self::InheritProfile,
        Self::ReportedStatement,
        Self::QuotedStatement,
        Self::MeasuredResult,
        Self::OfficialCharacterization,
        Self::EvidenceReference,
        Self::AssertedClock,
        Self::HeaderDate,
        Self::DiarizationSpeaker,
        Self::CrossDocumentEcho,
        Self::SharedAnchor,
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
            Self::InheritProfile => "inherit-profile",
            Self::ReportedStatement => "reported-statement",
            Self::QuotedStatement => "quoted-statement",
            Self::MeasuredResult => "measured-result",
            Self::OfficialCharacterization => "official-characterization",
            Self::EvidenceReference => "evidence-reference",
            Self::AssertedClock => "asserted-clock",
            Self::HeaderDate => "header-date",
            Self::DiarizationSpeaker => "diarization-speaker",
            Self::CrossDocumentEcho => "cross-document-echo",
            Self::SharedAnchor => "shared-anchor",
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
                | Self::DiarizationSpeaker
                | Self::CrossDocumentEcho
                | Self::SharedAnchor
        )
    }

    /// Whether this analyzer proposes interpretation candidates.
    ///
    /// An interpretation candidate is a reading of one passage, not a
    /// relationship between two records, so it lands in
    /// `content_interpretations` as `suggested` rather than in `edges`. Like
    /// every proposal it waits for a person; unlike an edge it has no second
    /// endpoint to point at.
    pub const fn proposes_interpretations(self) -> bool {
        matches!(
            self,
            Self::InheritProfile
                | Self::ReportedStatement
                | Self::QuotedStatement
                | Self::MeasuredResult
                | Self::OfficialCharacterization
                | Self::EvidenceReference
                | Self::AssertedClock
                | Self::HeaderDate
        )
    }

    /// Whether this analyzer derives findings instead of writing proposals.
    ///
    /// The three classes are exhaustive and disjoint: an analyzer proposes
    /// relationships, proposes interpretations, or reports findings. Anything
    /// walking every analyzer to collect gaps asks this first, because calling
    /// for the findings of a proposing analyzer is a programming error.
    pub const fn reports_findings(self) -> bool {
        !self.proposes_relationships() && !self.proposes_interpretations()
    }

    /// Whether this is one of the semantic-enrichment rules.
    pub const fn is_enrichment(self) -> bool {
        matches!(
            self,
            Self::InheritProfile
                | Self::ReportedStatement
                | Self::QuotedStatement
                | Self::MeasuredResult
                | Self::OfficialCharacterization
                | Self::EvidenceReference
                | Self::AssertedClock
                | Self::HeaderDate
                | Self::DiarizationSpeaker
                | Self::CrossDocumentEcho
                | Self::SharedAnchor
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
    /// Interpretation candidates newly proposed by this rule.
    pub proposed_interpretations: Vec<ContentInterpretation>,
    /// Source-profile candidates newly proposed by this rule.
    pub proposed_profiles: Vec<SourceProfile>,
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
    /// Total relationships, interpretations, and source profiles newly proposed.
    pub proposed: u32,
    /// Total candidates skipped because the case already held the claim.
    pub already_recorded: u32,
    /// Total gaps reported.
    pub findings: u32,
}
