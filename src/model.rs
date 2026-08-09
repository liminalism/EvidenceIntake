//! Shared domain vocabulary.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable identifier for a case.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CaseId(pub String);

impl fmt::Display for CaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// A typed reference used by the edge table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRef {
    /// The table-level node type, such as `proposition` or `content`.
    pub kind: NodeKind,
    /// Stable identifier inside that node type.
    pub id: String,
}

impl NodeRef {
    /// Builds a reference to one node.
    pub fn new(kind: NodeKind, id: impl Into<String>) -> Self {
        Self {
            kind,
            id: id.into(),
        }
    }
}

impl fmt::Display for NodeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} `{}`", self.kind.as_str(), self.id)
    }
}

/// A node type the polymorphic `edges` table can point at.
///
/// The edge table carries no foreign key, so this enum is what keeps a
/// relationship from naming a table that does not exist. Every variant maps to
/// a case-scoped table, which is what lets a link be checked against the case
/// it claims to belong to. Charges and elements are deliberately absent:
/// elements reach propositions through `element_links`, not through `edges`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// An extracted or hand-entered evidentiary content item.
    Content,
    /// An immutable source record.
    Source,
    /// A contested proposition.
    Proposition,
    /// A timeline event.
    Event,
    /// A typed relationship, which may itself be the subject of another.
    Edge,
    /// A person, organization, object, or location.
    Entity,
    /// A privileged attorney work-product item.
    Advocacy,
}

impl NodeKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Source => "source",
            Self::Proposition => "proposition",
            Self::Event => "event",
            Self::Edge => "edge",
            Self::Entity => "entity",
            Self::Advocacy => "advocacy",
        }
    }

    /// Returns the case-scoped table holding this node type.
    pub(crate) const fn table(self) -> &'static str {
        match self {
            Self::Content => "content",
            Self::Source => "sources",
            Self::Proposition => "propositions",
            Self::Event => "events",
            Self::Edge => "edges",
            Self::Entity => "entities",
            Self::Advocacy => "advocacy_items",
        }
    }
}

/// Broad original-source category, independent of extraction adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Police reports, witness statements, damage reports, and similar records.
    Document,
    /// Original audio recording.
    Audio,
    /// Original video recording.
    Video,
    /// A preserved collection of original photographs.
    ImageSet,
    /// CAD export, device log, or other structured native data.
    StructuredData,
    /// A source not represented by another category.
    Other,
}

impl SourceKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::ImageSet => "image_set",
            Self::StructuredData => "structured_data",
            Self::Other => "other",
        }
    }
}

/// How source creation relates to the event under investigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalRelation {
    /// Captured during the event, such as 911 audio or roadside video.
    Contemporaneous,
    /// Created afterward, such as a police report or damage assessment.
    AfterEvent,
    /// Contains both contemporaneous and retrospective material.
    Mixed,
    /// Not yet classified.
    Unknown,
}

impl TemporalRelation {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Contemporaneous => "contemporaneous",
            Self::AfterEvent => "after_event",
            Self::Mixed => "mixed",
            Self::Unknown => "unknown",
        }
    }
}

/// Evidentiary content present in a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    /// Words spoken or written by a person.
    Statement,
    /// A direct observation, including a bounded negative observation.
    Observation,
    /// An assertion made by a document's author.
    DocumentAssertion,
    /// A reference to evidence that may or may not have been produced.
    EvidenceReference,
    /// A gap or interruption in a recording.
    RecordingGap,
}

impl ContentKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::Observation => "observation",
            Self::DocumentAssertion => "document_assertion",
            Self::EvidenceReference => "evidence_reference",
            Self::RecordingGap => "recording_gap",
        }
    }
}

/// A contest-preserving relationship between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// The source tends to establish the target.
    Supports,
    /// The source is inconsistent with the target.
    Contradicts,
    /// Independent evidence tends to confirm the target.
    Corroborates,
    /// The source may undermine a witness or account.
    Impeaches,
    /// The source narrows or conditions the target.
    Qualifies,
    /// The source offers an explanation for the target.
    Explains,
    /// The target is derived from the source.
    DerivedFrom,
    /// The source mentions the target.
    RefersTo,
    /// The nodes overlap in time.
    TemporallyOverlaps,
    /// The nodes may concern the same person but are not merged.
    PossiblySamePerson,
    /// The target is referenced but not present in discovery.
    ExpectedButMissing,
    /// Review identified a concrete follow-up need.
    RequiresFollowUp,
    /// An element or issue relies on a proposition.
    RelevantTo,
}

impl EdgeKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Contradicts => "contradicts",
            Self::Corroborates => "corroborates",
            Self::Impeaches => "impeaches",
            Self::Qualifies => "qualifies",
            Self::Explains => "explains",
            Self::DerivedFrom => "derived_from",
            Self::RefersTo => "refers_to",
            Self::TemporallyOverlaps => "temporally_overlaps",
            Self::PossiblySamePerson => "possibly_same_person",
            Self::ExpectedButMissing => "expected_but_missing",
            Self::RequiresFollowUp => "requires_follow_up",
            Self::RelevantTo => "relevant_to",
        }
    }
}

/// What kind of thing an entity is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A person. Only people are compared for possible duplication.
    Person,
    /// A company, agency, or other body.
    Organization,
    /// A vehicle, weapon, or other physical thing.
    Object,
    /// A place.
    Location,
}

impl EntityKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Organization => "organization",
            Self::Object => "object",
            Self::Location => "location",
        }
    }
}

/// How a charge stands in the case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargePosture {
    /// Actually charged by the state.
    Charged,
    /// A lesser offense the evidence might instead fit.
    LesserCandidate,
    /// Another offense worth comparing.
    Alternative,
    /// No longer live.
    Dismissed,
}

impl ChargePosture {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Charged => "charged",
            Self::LesserCandidate => "lesser_candidate",
            Self::Alternative => "alternative",
            Self::Dismissed => "dismissed",
        }
    }
}

/// How a proposition bears on one statutory element.
///
/// This is a direction, not a weight. `uncertain` is a first-class answer and
/// the honest one for most contested material; it is not a placeholder for an
/// assessment somebody has yet to sharpen into support or opposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementAssessment {
    /// The proposition tends to establish the element.
    Supports,
    /// The proposition tends to defeat the element.
    Opposes,
    /// The proposition is material but its effect is unresolved.
    Uncertain,
    /// The proposition is set aside from the present analysis.
    Excluded,
}

impl ElementAssessment {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supports => "supports",
            Self::Opposes => "opposes",
            Self::Uncertain => "uncertain",
            Self::Excluded => "excluded",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "supports" => Some(Self::Supports),
            "opposes" => Some(Self::Opposes),
            "uncertain" => Some(Self::Uncertain),
            "excluded" => Some(Self::Excluded),
            _ => None,
        }
    }
}

/// Human review state. Machine suggestions cannot enter a confirmed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    /// Not yet reviewed.
    Unreviewed,
    /// Suggested by a machine and awaiting a person.
    Suggested,
    /// A person reviewed the item but did not verify it against the source.
    Reviewed,
    /// A person verified it against the cited original.
    Verified,
    /// A person rejected the extraction or proposed relationship.
    Rejected,
}

impl ReviewState {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Suggested => "suggested",
            Self::Reviewed => "reviewed",
            Self::Verified => "verified",
            Self::Rejected => "rejected",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "unreviewed" => Some(Self::Unreviewed),
            "suggested" => Some(Self::Suggested),
            "reviewed" => Some(Self::Reviewed),
            "verified" => Some(Self::Verified),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }

    /// Returns whether this state is produced by import rather than by a person.
    pub const fn is_intake_state(self) -> bool {
        matches!(self, Self::Unreviewed | Self::Suggested)
    }
}

/// Privileged attorney-work-product categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvocacyKind {
    /// A defense account that may fit the evidence.
    DefenseTheory,
    /// The apparent prosecution account.
    ProsecutionTheory,
    /// A legal question requiring attorney analysis.
    LegalIssue,
    /// A potential motion and its factual predicates.
    MotionIssue,
    /// A cross-examination point.
    CrossExaminationPoint,
    /// A concrete investigation task.
    InvestigationTask,
    /// A negotiation consideration.
    NegotiationConsideration,
    /// A mitigation theme.
    MitigationTheme,
    /// A human attorney conclusion.
    AttorneyConclusion,
}

impl AdvocacyKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefenseTheory => "defense_theory",
            Self::ProsecutionTheory => "prosecution_theory",
            Self::LegalIssue => "legal_issue",
            Self::MotionIssue => "motion_issue",
            Self::CrossExaminationPoint => "cross_examination_point",
            Self::InvestigationTask => "investigation_task",
            Self::NegotiationConsideration => "negotiation_consideration",
            Self::MitigationTheme => "mitigation_theme",
            Self::AttorneyConclusion => "attorney_conclusion",
        }
    }
}

/// A lane in the contested timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineLane {
    /// Directly recorded events.
    Recorded,
    /// A witness account of an event.
    WitnessAccount,
    /// A law-enforcement narrative.
    PoliceNarrative,
    /// The client's account.
    ClientAccount,
    /// An attorney-created hypothesis.
    AttorneyHypothesis,
}

impl TimelineLane {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recorded => "recorded",
            Self::WitnessAccount => "witness_account",
            Self::PoliceNarrative => "police_narrative",
            Self::ClientAccount => "client_account",
            Self::AttorneyHypothesis => "attorney_hypothesis",
        }
    }
}
