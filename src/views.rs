//! Decision-oriented case views.

use serde::Serialize;

/// One row of the case docket: enough to pick a matter, not a reading of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseSummary {
    /// Stable case identifier.
    pub id: String,
    /// Human-readable case name.
    pub name: String,
    /// Docket, incident, or file number.
    pub reference: Option<String>,
    /// Court or charging jurisdiction.
    pub jurisdiction: Option<String>,
    /// When the case row was opened.
    pub created_at: String,
    /// Number of discovery productions.
    pub productions: u32,
    /// Number of immutable source records.
    pub sources: u32,
    /// Records still in an intake state and awaiting a person.
    pub pending_review: u32,
}

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
    /// The named person who made the mapping.
    ///
    /// Absent for mappings written before authorship was recorded; they are not
    /// backfilled with a name nobody actually stood behind.
    pub mapped_by: Option<String>,
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

/// One source-grounded passage in the case collation index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollationEntry {
    /// Stable content identifier.
    pub content_id: String,
    /// Stable immutable-source identifier.
    pub source_id: String,
    /// Source name shown to the reviewer.
    pub source: String,
    /// Broad original-source category.
    pub source_kind: String,
    /// Coarse immutable intake category retained for provenance badges.
    pub content_kind: String,
    /// Exact locator in the original source.
    pub locator: String,
    /// Exact extracted or human-authored text.
    pub text: String,
    /// Unmodified time expression from the source.
    pub raw_time: Option<String>,
    /// When the passage or source record says the content was created.
    pub content_created_at: Option<String>,
    /// Time the passage asserts, distinct from its creation time.
    pub asserted_time: Option<String>,
    /// Proposed normalized interval start; never replaces raw time.
    pub normalized_start: Option<String>,
    /// Proposed normalized interval end.
    pub normalized_end: Option<String>,
    /// Written basis for the proposed normalization.
    pub time_basis: Option<String>,
    /// Location text exactly as stored with the passage.
    pub location: Option<String>,
    /// Whether an extractor, rather than a person, created the passage.
    pub machine_generated: bool,
    /// Extractor name for machine material.
    pub extractor: Option<String>,
    /// Current human review state.
    pub review_state: String,
}

/// Evidence sharing one transparent date and/or location collation key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollationGroup {
    /// Date cut directly from an ISO-like normalized start, when applicable.
    pub normalized_date: Option<String>,
    /// Conservatively matched location text, when applicable.
    pub location: Option<String>,
    /// Number of distinct immutable sources represented in this group.
    pub distinct_originals: u32,
    /// Exact structural reason the entries appear together.
    pub rationale: String,
    /// Chronologically ordered source-grounded entries.
    pub entries: Vec<CollationEntry>,
}

/// Structural anchor counts for one immutable source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceAnchorCoverage {
    /// Stable immutable-source identifier.
    pub source_id: String,
    /// Source name shown to the reviewer.
    pub source: String,
    /// Broad original-source category.
    pub source_kind: String,
    /// Active source-grounded passages in this source.
    pub passages: u32,
    /// Passages carrying an unmodified source time expression.
    pub with_raw_time: u32,
    /// Passages carrying a content-creation time.
    pub with_content_created_at: u32,
    /// Passages carrying a distinct asserted time.
    pub with_asserted_time: u32,
    /// Passages carrying a usable normalized date.
    pub with_normalized_date: u32,
    /// Passages carrying non-empty location text.
    pub with_location: u32,
}

/// One active passage and the anchors it still lacks for collation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlacementGap {
    /// Stable missing-anchor names, currently `normalized_date` and `location`.
    pub missing_anchors: Vec<String>,
    /// Source-grounded passage that needs placement work.
    pub entry: CollationEntry,
}

/// Case-level time and location index that does not declare common events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CollationIndex {
    /// Case whose evidence is indexed.
    pub case_id: String,
    /// Active content arranged by normalized calendar date.
    pub by_date: Vec<CollationGroup>,
    /// Active content arranged by conservative exact location text.
    pub by_location: Vec<CollationGroup>,
    /// Multi-source groups sharing both a normalized date and location key.
    ///
    /// These are navigation groups, not stored relationships or event claims.
    pub shared_anchor_unconfirmed: Vec<CollationGroup>,
    /// Per-original inventory of which placement anchors intake supplied.
    pub source_coverage: Vec<SourceAnchorCoverage>,
    /// Active passages missing a normalized date, location, or both.
    pub needs_placement: Vec<PlacementGap>,
    /// Active passages without a usable normalized date.
    pub without_normalized_date: u32,
    /// Active passages without location text.
    pub without_location: u32,
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
    /// Human review state of the underlying content.
    pub review_state: String,
    /// Human rationale for the relationship.
    pub rationale: Option<String>,
    /// Human review state of the relationship itself.
    ///
    /// Distinct from `review_state`: a verified excerpt can be tied to a
    /// proposition by a relationship nobody has looked at, and a reader has to
    /// be able to see that the connection — not just the words — is unchecked.
    pub relation_review_state: String,
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

/// One excerpt matching a search, with the original it came from.
///
/// A hit is a place to look, never a finding. It carries the exact locator so
/// the passage can be opened in the original, and the propositions it is already
/// tied to so a defender can see whether the case has done anything with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SearchHit {
    /// Content identifier.
    pub id: String,
    /// Content kind, such as `statement` or `document_assertion`.
    pub kind: String,
    /// The matching passage, with the matched terms bracketed.
    pub excerpt: String,
    /// The full extracted text of the matching content.
    pub text: String,
    /// Original source name.
    pub source: String,
    /// Exact locator in the original source.
    pub locator: String,
    /// Human review state of the content.
    pub review_state: String,
    /// Whether this remains machine-generated content.
    pub machine_generated: bool,
    /// Propositions this excerpt is already tied to, with the relationship.
    ///
    /// Empty means the case has not connected the passage to anything, which is
    /// worth seeing: a search hit nobody has used is work waiting to be done.
    pub bears_on: Vec<String>,
}

/// One still matching a visual query, with the original it was cut from.
///
/// A hit is a place to look, never a finding. It names the original by hash
/// and the time range on that original, and the derived still so the working
/// copy can be opened. Internal similarity selects the bounded candidate pool;
/// the pool is then ordered by still identifier. The number itself is not a
/// field: a number printed next to a frame gets read as a measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyframeHit {
    /// Derived still source identifier.
    pub source_id: String,
    /// SHA-256 of the working-copy jpeg.
    pub still_sha256: String,
    /// Logical name of the original video.
    pub source: String,
    /// SHA-256 of the original container.
    pub sha256: String,
    /// Exact locator on the original timeline.
    pub locator: String,
    /// Human review state of the still's observation.
    pub review_state: String,
    /// Whether the still observation remains machine-generated.
    pub machine_generated: bool,
    /// Propositions this still is already tied to, with the relationship.
    pub bears_on: Vec<String>,
}

/// Where a case stands, element by element, without saying who wins it.
///
/// Everything here is a count of what the case holds or a fact about how its
/// material is connected. Nothing is weighted, ranked by strength, or reduced
/// to a number standing for how the case will come out — an element with no
/// supporting proposition is reported as having none, which is an observation,
/// not a prediction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseStanding {
    /// Case identifier.
    pub case_id: String,
    /// Human-readable case name.
    pub case_name: String,
    /// Every charge, element by element.
    pub charges: Vec<ChargeStanding>,
    /// Sources that alone carry an element's support.
    pub load_bearing_sources: Vec<LoadBearingSource>,
    /// Propositions with evidence pulling both ways.
    pub live_disputes: Vec<LiveDispute>,
    /// Gaps the analyzers found, those touching a charge first.
    pub open_gaps: Vec<OpenGap>,
}

/// One charge and the standing of each of its elements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChargeStanding {
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
    /// Element-by-element standing, in statutory order.
    pub elements: Vec<ElementStanding>,
}

/// What one statutory element rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElementStanding {
    /// Ordered element number.
    pub ordinal: u32,
    /// Element text.
    pub element: String,
    /// Propositions mapped as tending to support the element.
    pub supporting: u32,
    /// Propositions mapped as tending to oppose it.
    pub opposing: u32,
    /// Propositions whose effect a person recorded as uncertain.
    pub uncertain: u32,
    /// Propositions a person excluded from the present analysis.
    pub excluded: u32,
    /// Distinct original sources the supporting propositions rest on.
    ///
    /// Three propositions quoting one report are not three sources, and an
    /// element cannot be argued about without knowing which it is.
    pub sources_behind_support: u32,
    /// Named when every supporting proposition traces back to one source.
    ///
    /// The single point at which the element's support fails, which is where a
    /// suppression or foundation argument is worth the effort.
    pub sole_source: Option<String>,
    /// Supporting propositions no person has checked any evidence for.
    ///
    /// Counted, not hidden: an element resting on material nobody has opened is
    /// standing on an assumption about what the original says.
    pub unchecked_support: u32,
    /// Propositions mapped here that no source-grounded evidence reaches.
    pub unbacked: u32,
}

/// A source that alone carries the support for at least one element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadBearingSource {
    /// Source identifier.
    pub id: String,
    /// Source logical name.
    pub source: String,
    /// Human review state of the source.
    pub review_state: String,
    /// Elements whose support rests on this source and nothing else.
    pub sole_support_for: Vec<String>,
}

/// A proposition with source-grounded evidence pulling both ways.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveDispute {
    /// Proposition identifier.
    pub id: String,
    /// Proposition text.
    pub proposition: String,
    /// Excerpts supporting or corroborating it.
    pub supporting_evidence: u32,
    /// Excerpts contradicting or impeaching it.
    pub contradicting_evidence: u32,
    /// Elements this proposition is mapped to, if any.
    pub bears_on: Vec<String>,
}

/// A gap an analyzer reported, placed against the charges it touches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenGap {
    /// The analyzer that reported it.
    pub analyzer: String,
    /// Node type of the record the gap concerns.
    pub subject_kind: String,
    /// Identifier of that record.
    pub subject_id: String,
    /// The record's own words.
    pub subject: String,
    /// What is missing and what closing it would take.
    pub summary: String,
    /// Elements the gap bears on; empty when it touches no charge.
    pub bears_on: Vec<String>,
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
