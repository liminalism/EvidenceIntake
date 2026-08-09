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

use crate::{ChargePosture, EdgeKind, ElementAssessment, NodeRef};

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
