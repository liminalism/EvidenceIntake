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

use crate::{EdgeKind, NodeRef};

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
