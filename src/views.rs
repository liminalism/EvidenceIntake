//! Decision-oriented case views.

use serde::Serialize;

/// Counts that describe a case without pretending to assess its truth.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Overview {
    /// Case identifier.
    pub case_id: String,
    /// Human-readable case name.
    pub case_name: String,
    /// Number of discovery productions.
    pub productions: u32,
    /// Number of immutable source records.
    pub sources: u32,
    /// Sources not yet reviewed by a person.
    pub unreviewed_sources: u32,
    /// References to expected but absent evidence.
    pub missing_references: u32,
    /// Contested propositions in the factual model.
    pub propositions: u32,
    /// Records still in an intake state and awaiting a person.
    pub pending_review: u32,
    /// Open privileged work-product items.
    pub open_advocacy_items: u32,
}

/// One row of the production and discovery-completeness ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveryItem {
    /// Production label.
    pub production: String,
    /// When the production was received, if recorded.
    pub received_at: Option<String>,
    /// Logical filename or expected item description.
    pub source: String,
    /// MIME-like media type.
    pub media_type: String,
    /// Broad original-source category.
    pub source_kind: String,
    /// Whether it was captured during or created after the investigated event.
    pub temporal_relation: String,
    /// Availability or integrity status.
    pub integrity_status: String,
    /// Human review state.
    pub review_state: String,
    /// Earlier source replaced by this source, when applicable.
    pub supersedes: Option<String>,
}

/// An element and the contested propositions mapped to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElementRow {
    /// Charge label.
    pub charge: String,
    /// Statutory or other citation.
    pub citation: Option<String>,
    /// Ordered element number.
    pub ordinal: u32,
    /// Element text.
    pub element: String,
    /// Assessment of the linked proposition.
    pub assessment: Option<String>,
    /// Proposition text, if linked.
    pub proposition: Option<String>,
    /// Attorney notes about this mapping.
    pub notes: Option<String>,
}

/// A statement by, or attributed to, a witness.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WitnessStatement {
    /// Content identifier.
    pub id: String,
    /// Speaker or author who made this report.
    pub reporting_person: Option<String>,
    /// Person to whom the words are attributed.
    pub attributed_to: Option<String>,
    /// Exact extracted or human-entered content.
    pub text: String,
    /// When the statement was made, distinct from the alleged-event time.
    pub statement_time: Option<String>,
    /// Exact source locator.
    pub locator: String,
    /// Source logical name.
    pub source: String,
    /// Human review state.
    pub review_state: String,
    /// Relationships that bear on credibility.
    pub credibility_links: Vec<String>,
}

/// One lane-specific entry in the contested timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineEntry {
    /// Event identifier.
    pub id: String,
    /// Timeline lane; lanes must remain visually distinct.
    pub lane: String,
    /// Event label.
    pub label: String,
    /// Unmodified time expression from the source.
    pub raw_time: Option<String>,
    /// Proposed normalized start; never replaces raw time.
    pub normalized_start: Option<String>,
    /// Basis for the proposed alignment.
    pub time_basis: Option<String>,
    /// Location as expressed or normalized for review.
    pub location: Option<String>,
    /// Linked contested proposition.
    pub proposition: Option<String>,
}

/// A legal or procedural issue and the factual work surrounding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssueWorkspace {
    /// Advocacy-item identifier.
    pub id: String,
    /// Issue title.
    pub title: String,
    /// Human attorney analysis.
    pub body: String,
    /// Workflow state.
    pub status: String,
    /// Linked propositions and evidence with relationship labels.
    pub linked_material: Vec<String>,
    /// Open investigation or follow-up tasks.
    pub follow_up: Vec<String>,
}

/// A posture-specific, privileged decision brief.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionBrief {
    /// Decision posture, such as motions or negotiation.
    pub posture: String,
    /// Human-authored summary.
    pub summary: String,
    /// Strong portions of the defense position.
    pub strengths: String,
    /// Material risks.
    pub risks: String,
    /// Questions that could change advice.
    pub unresolved_questions: String,
    /// Topics to discuss with the client.
    pub client_topics: String,
    /// Version number.
    pub version: u32,
    /// Human author.
    pub author: String,
}

/// One exact source-grounded item bearing on a contested proposition.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PropositionEvidence {
    /// Relationship to the proposition.
    pub relation: String,
    /// Evidentiary content text.
    pub text: String,
    /// Original source name.
    pub source: String,
    /// Exact locator in the original source.
    pub locator: String,
    /// When the statement or recording was made.
    pub source_time: Option<String>,
    /// Time the content claims the underlying event occurred.
    pub asserted_time: Option<String>,
    /// Proposed normalized event time.
    pub normalized_start: Option<String>,
    /// Extraction adapter or human workflow.
    pub extractor: Option<String>,
    /// Exact extractor/model version.
    pub extractor_version: Option<String>,
    /// Whether this remains machine-generated content.
    pub machine_generated: bool,
    /// Extractor confidence, not factual confidence.
    pub extractor_confidence: Option<f64>,
    /// Human review state.
    pub review_state: String,
    /// Human rationale for the relationship.
    pub rationale: Option<String>,
}

/// Evidence mappings for one statutory element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElementCoverage {
    /// Ordered element number.
    pub ordinal: u32,
    /// Element text.
    pub element: String,
    /// Propositions tending to support the element.
    pub supporting: Vec<String>,
    /// Propositions tending to oppose the element.
    pub opposing: Vec<String>,
    /// Material propositions whose effect remains uncertain.
    pub uncertain: Vec<String>,
    /// Propositions excluded from the present analysis.
    pub excluded: Vec<String>,
}

/// A charged offense or lesser candidate shown without a recommendation score.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OffenseComparison {
    /// Charge identifier.
    pub id: String,
    /// Human-readable offense.
    pub charge: String,
    /// Statutory citation.
    pub citation: Option<String>,
    /// `charged`, `lesser_candidate`, `alternative`, or `dismissed`.
    pub posture: String,
    /// Felony, misdemeanor, infraction, or jurisdiction-specific grade.
    pub grade: Option<String>,
    /// Element-by-element evidence mapping.
    pub elements: Vec<ElementCoverage>,
}
