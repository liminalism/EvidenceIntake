//! Human authoring input and the records it produces.
//!
//! Authoring is how a defender adds their own reading of the case: a contested
//! proposition, and the typed relationships tying source-grounded content to it.
//! It is the counterpart to [`crate::NormalizedBatch`], which is how an adapter
//! adds a machine's reading.
//!
//! Nothing here confers review. A person writing a proposition down has not
//! thereby checked it, so authored records enter `unreviewed` and wait in the
//! same queue as everything else. The types in this module carry no behavior;
//! the rules live with the store that has to enforce them.

use serde::{Deserialize, Serialize};

use crate::{AdvocacyKind, ChargePosture, EdgeKind, ElementAssessment, EntityKind, NodeRef};

/// A contested proposition a person is asking the case to hold.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedProposition {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The proposition as a person would state it.
    pub text: String,
    /// The named person writing it down.
    pub author: String,
}

/// A typed relationship a person is asserting between two nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedLink {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The node doing the supporting, contradicting, or qualifying.
    pub from: NodeRef,
    /// How the two nodes stand to one another.
    pub relation: EdgeKind,
    /// The node the relationship bears on.
    pub to: NodeRef,
    /// Why the author says the relationship holds. Required: a relationship
    /// spans sources and has no original of its own, so this is the only thing
    /// a later reader can weigh.
    pub rationale: String,
    /// The named person asserting it.
    pub author: String,
}

/// A proposition as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredProposition {
    /// Stable identifier.
    pub id: String,
    /// The proposition text as stored.
    pub text: String,
    /// Always `contested` at authoring time.
    pub status: String,
    /// Always `unreviewed` at authoring time.
    pub review_state: String,
    /// The named person who wrote it.
    pub created_by: String,
}

/// A person, organization, object, or place appearing in the case.
///
/// Two entities with the same name are allowed. Whether they are one thing is a
/// question for a person, not a uniqueness constraint: refusing the second would
/// merge them by default, which is exactly what this kernel does not do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedEntity {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// What kind of thing this is.
    pub kind: EntityKind,
    /// The name as it should be shown.
    pub display_name: String,
    /// Whether this is the client.
    #[serde(default)]
    pub is_client: bool,
    /// Anything worth recording about the identification itself.
    #[serde(default)]
    pub notes: Option<String>,
}

/// An entity as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredEntity {
    /// Stable identifier.
    pub id: String,
    /// What kind of thing this is.
    pub kind: String,
    /// The name as stored.
    pub display_name: String,
    /// Whether this is the client.
    pub is_client: bool,
    /// Notes about the identification, if given.
    pub notes: Option<String>,
}

/// A charge and its statutory elements, as a person reads the charging document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedCharge {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The offense as a person would name it.
    pub label: String,
    /// Statutory or other citation.
    #[serde(default)]
    pub citation: Option<String>,
    /// Whether this is charged, a lesser candidate, an alternative, or dismissed.
    pub posture: ChargePosture,
    /// Felony, misdemeanor, infraction, or a jurisdiction-specific grade.
    #[serde(default)]
    pub grade: Option<String>,
    /// The elements, in the order the statute states them. A charge without
    /// elements cannot be reasoned about, so at least one is required.
    pub elements: Vec<ProposedElement>,
}

/// One statutory element of a charge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedElement {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The element as the statute states it.
    pub text: String,
}

/// A person's reading of how one proposition bears on one element.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedElementMapping {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The element being mapped.
    pub element_id: String,
    /// The contested proposition bearing on it.
    pub proposition_id: String,
    /// The direction of that bearing. Not a weight.
    pub assessment: ElementAssessment,
    /// Why the mapping reads that way.
    #[serde(default)]
    pub notes: Option<String>,
    /// The named person making the assessment.
    pub author: String,
}

/// A charge as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredCharge {
    /// Stable identifier.
    pub id: String,
    /// The offense label as stored.
    pub label: String,
    /// Statutory citation, if given.
    pub citation: Option<String>,
    /// Charge posture.
    pub posture: String,
    /// Offense grade, if given.
    pub grade: Option<String>,
    /// The elements as stored, in statutory order.
    pub elements: Vec<AuthoredElement>,
}

/// One element as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredElement {
    /// Stable identifier.
    pub id: String,
    /// Position in the statute, starting at one.
    pub ordinal: u32,
    /// The element text as stored.
    pub text: String,
}

/// An element mapping as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredElementMapping {
    /// Stable identifier.
    pub id: String,
    /// The element mapped.
    pub element_id: String,
    /// The proposition mapped to it.
    pub proposition_id: String,
    /// Direction of the bearing.
    pub assessment: String,
    /// The author's reasoning, if given.
    pub notes: Option<String>,
    /// The named person who made the assessment.
    pub created_by: String,
}

/// A privileged work-product item a person is writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAdvocacyItem {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Which kind of work product this is.
    pub kind: AdvocacyKind,
    /// Short title.
    pub title: String,
    /// The analysis itself.
    pub body: String,
    /// Workflow state; `open` when absent.
    #[serde(default)]
    pub status: Option<String>,
    /// The named person writing it.
    pub author: String,
}

/// A privileged note attached to one record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAnnotation {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The record being annotated.
    pub target: NodeRef,
    /// The note itself.
    pub body: String,
    /// The named person writing it.
    pub author: String,
}

/// A posture-specific decision brief a person is writing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedBrief {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// release, motions, negotiation, trial, sentencing, or appeal.
    pub posture: String,
    /// What the brief says.
    pub summary: String,
    /// Strong portions of the defense position.
    pub strengths: String,
    /// Material risks.
    pub risks: String,
    /// Questions that could change the advice.
    pub unresolved_questions: String,
    /// Topics to discuss with the client.
    pub client_topics: String,
    /// The named person writing it.
    pub author: String,
}

/// A work-product record as it stands after being written or revised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkProductVersion {
    /// Stable identifier of this version.
    pub id: String,
    /// Which version this is, counting from one.
    pub version: u32,
    /// The version this one replaces, when it replaces one.
    pub supersedes: Option<String>,
    /// Whether this is the current version.
    pub current: bool,
    /// Title, or the posture for a brief.
    pub title: String,
    /// The text of this version.
    pub body: String,
    /// Workflow state.
    pub status: String,
    /// Whether the record is privileged. Work product is, by default.
    pub privileged: bool,
    /// The named person who wrote this version.
    pub author: String,
    /// When this version was written.
    pub created_at: String,
}

/// A new case a person is opening in this database.
///
/// Opening a case is not authoring evidence. It creates the empty docket row
/// and a first production so intake has a ledger to attach originals to.
/// Nothing in the case is reviewed or verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedCase {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The name as it should appear on the docket.
    pub name: String,
    /// Docket, incident, or file number.
    #[serde(default)]
    pub reference: Option<String>,
    /// Court or charging jurisdiction.
    #[serde(default)]
    pub jurisdiction: Option<String>,
    /// Label for the first production. Defaults to `Initial production`.
    #[serde(default)]
    pub production: Option<String>,
}

/// A case as it was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedCase {
    /// Stable identifier.
    pub id: String,
    /// The name as stored.
    pub name: String,
    /// Docket, incident, or file number, if given.
    pub reference: Option<String>,
    /// Court or charging jurisdiction, if given.
    pub jurisdiction: Option<String>,
    /// The first production, opened with the case.
    pub production: OpenedProduction,
}

/// A new production on an existing case.
///
/// A production is the intake hook: every original must belong to one. It is
/// not evidence and carries no review state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedProduction {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// How this delivery is labelled on the ledger.
    pub label: String,
    /// When the production was received, if recorded.
    #[serde(default)]
    pub received_at: Option<String>,
    /// Who produced it.
    #[serde(default)]
    pub producing_party: Option<String>,
    /// Anything worth recording about the delivery itself.
    #[serde(default)]
    pub notes: Option<String>,
}

/// A production as it was opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OpenedProduction {
    /// Stable identifier.
    pub id: String,
    /// Case that owns this production.
    pub case_id: String,
    /// Ledger label.
    pub label: String,
    /// When the production was received, if recorded.
    pub received_at: Option<String>,
    /// Who produced it.
    pub producing_party: Option<String>,
    /// Notes about the delivery, if given.
    pub notes: Option<String>,
}

/// A relationship as it was written into the case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthoredLink {
    /// Stable identifier.
    pub id: String,
    /// Node type of the asserting side.
    pub from_kind: String,
    /// Identifier of the asserting side.
    pub from_id: String,
    /// The typed relation.
    pub relation: String,
    /// Node type of the side the relationship bears on.
    pub to_kind: String,
    /// Identifier of that side.
    pub to_id: String,
    /// The written reason the relationship holds.
    pub rationale: String,
    /// Always `unreviewed` at authoring time.
    pub review_state: String,
    /// The named person who asserted it.
    pub created_by: String,
}
