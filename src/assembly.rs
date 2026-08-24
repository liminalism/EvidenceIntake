//! Reviewed provenance assembly and count vocabulary.

use serde::{Deserialize, Serialize};

use crate::{AuthoredLink, NodeRef, TimelineLane};

/// Reviewed reporting/derivation lineage for one node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lineage {
    /// Node whose dependency roots were requested.
    pub node: NodeRef,
    /// Nodes reached while following reviewed dependency edges.
    pub members: Vec<NodeRef>,
    /// Immutable source roots, or non-content roots when no source can be resolved.
    pub roots: Vec<NodeRef>,
}

/// Three non-evaluative counts used by packets, standing, and digests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
pub struct StructuralCount {
    /// Distinct immutable original files.
    pub original_files: u32,
    /// Distinct reviewed reporting lineages.
    pub reporting_lineages: u32,
    /// Items with no reviewed dependency edge.
    pub without_reviewed_dependency: u32,
}

impl std::fmt::Display for StructuralCount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} original files · {} reporting lineages · {} without a reviewed dependency",
            self.original_files, self.reporting_lineages, self.without_reviewed_dependency
        )
    }
}

/// One source-addressed passage in a proposition packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PacketItem {
    /// Immutable content row.
    pub content_id: String,
    /// Relationship by which the item bears on the proposition.
    pub relation: String,
    /// Extracted text, unchanged.
    pub text: String,
    /// Original file's display name.
    pub original: String,
    /// Exact page, time range, or other address in the original.
    pub locator: String,
    /// Reviewed semantic form, when available.
    pub content_form: Option<String>,
    /// Reviewed temporal stance, when available.
    pub temporal_stance: Option<String>,
    /// Time the source content itself was made.
    pub content_created_at: Option<String>,
    /// Time the passage says the occurrence took place.
    pub asserted_start: Option<String>,
    /// Reviewer-normalized case time, never substituted for the asserted time.
    pub normalized_start: Option<String>,
    /// Raw or reviewer-entered location wording.
    pub location: Option<String>,
    /// Passage interpretation state.
    pub interpretation_state: String,
    /// Proposition-link review state.
    pub relation_state: String,
    /// Written reason for the proposition link.
    pub rationale: Option<String>,
    /// Reviewed reporting/derivation roots.
    pub lineage_roots: Vec<NodeRef>,
}

/// One non-scoring packet section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PacketSection {
    /// Stable section key.
    pub key: String,
    /// Reviewer-facing heading.
    pub title: String,
    /// Source-addressed items.
    pub items: Vec<PacketItem>,
    /// Non-evaluative file/lineage/dependency counts.
    pub count: StructuralCount,
    /// Derived omissions or unresolved-work notices.
    pub notes: Vec<String>,
}

/// Source-linked workspace around one contested proposition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropositionPacket {
    /// Owning case.
    pub case_id: String,
    /// Stable proposition identifier.
    pub proposition_id: String,
    /// Neutral proposition wording entered by a person.
    pub proposition: String,
    /// Proposition posture, rendered without a calculated assessment.
    pub status: String,
    /// Fixed packet sections in reviewer order.
    pub sections: Vec<PacketSection>,
    /// Count across distinct linked passages.
    pub count: StructuralCount,
}

/// Exact support carried by a generated digest sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DigestLocator {
    /// Original file name.
    pub original: String,
    /// Exact address in that file.
    pub locator: String,
    /// Content node providing the sentence's slots.
    pub content_id: String,
}

/// One typed-template factual sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DigestSentence {
    /// Stable template identifier.
    pub template_id: String,
    /// Deterministically rendered text.
    pub text: String,
    /// One or more exact source addresses.
    pub locators: Vec<DigestLocator>,
}

/// Why a typed template was not emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OmittedDigestTemplate {
    /// Stable template identifier.
    pub template_id: String,
    /// Missing slot or review prerequisite.
    pub reason: String,
}

/// One of the digest's fixed layers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DigestSection {
    /// Stable layer key.
    pub key: String,
    /// Reviewer-facing heading.
    pub title: String,
    /// Source-linked factual sentences.
    pub sentences: Vec<DigestSentence>,
    /// Non-factual metadata or privileged analysis.
    pub analysis: Vec<String>,
    /// Templates withheld because required slots were absent or unreviewed.
    pub omitted: Vec<OmittedDigestTemplate>,
}

/// Deterministic, layered case digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaseDigest {
    /// Owning case.
    pub case_id: String,
    /// Case display name.
    pub case_name: String,
    /// `disclosable` or `work_file`.
    pub audience: String,
    /// Whether privileged tables were consulted.
    pub includes_privileged: bool,
    /// Fixed nine layers; privileged content is empty for disclosable output.
    pub sections: Vec<DigestSection>,
    /// Total typed templates withheld.
    pub omitted_sentences: u32,
}

/// A human request to turn reviewed same-occurrence candidates into an event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedOccurrence {
    /// Stable event identifier, generated when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Neutral event label.
    pub label: String,
    /// Account lane; lanes never collapse automatically.
    pub lane: TimelineLane,
    /// Raw time wording, when any.
    #[serde(default)]
    pub raw_time: Option<String>,
    /// Reviewer-normalized start.
    #[serde(default)]
    pub normalized_start: Option<String>,
    /// Reviewer-normalized end.
    #[serde(default)]
    pub normalized_end: Option<String>,
    /// Written alignment basis, required with normalized time.
    #[serde(default)]
    pub time_basis: Option<String>,
    /// Location wording.
    #[serde(default)]
    pub location_text: Option<String>,
    /// Optional contested proposition.
    #[serde(default)]
    pub proposition_id: Option<String>,
    /// Content accounts joined by reviewed same-occurrence candidate edges.
    pub passage_ids: Vec<String>,
    /// Written reason for the account grouping.
    pub rationale: String,
    /// Named person making the occurrence decision.
    pub author: String,
}

/// Newly authored event and its unreviewed account links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredOccurrence {
    /// Event identifier.
    pub event_id: String,
    /// One `account_of` edge per selected passage.
    pub account_links: Vec<AuthoredLink>,
}
