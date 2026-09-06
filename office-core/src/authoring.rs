//! Input types for office writes.
//!
//! The types here carry no behavior; the rules live with the store that has to
//! enforce them, exactly as they do in the evidence kernel's `authoring` module.
//! Every one of them names the person doing the writing, because the office
//! layer has no anonymous mutations.

use serde::{Deserialize, Serialize};

use crate::model::{
    AppearanceType, AssignmentRole, ContactKind, CustodyState, DeadlineOrigin, IdentityLinkState,
    MatterStatus, OfferState, Sex,
};

/// Somebody who can author office records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedUser {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The name colleagues know them by.
    pub display_name: String,
    /// What they do in the office.
    pub role: String,
    /// Work email, when there is one.
    #[serde(default)]
    pub email: Option<String>,
}

/// A person the office represents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedClient {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The person's name as the office records it.
    pub display_name: String,
    /// Date of birth in `YYYY-MM-DD`, when known.
    #[serde(default)]
    pub date_of_birth: Option<String>,
    /// How the office records the person's sex. Absent means not recorded.
    #[serde(default)]
    pub sex: Option<Sex>,
    /// The language they ask to be spoken to in, as they name it.
    #[serde(default)]
    pub preferred_language: Option<String>,
    /// Anything worth recording about the person rather than a case.
    #[serde(default)]
    pub notes: Option<String>,
    /// Other names the person goes by.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// How to reach them.
    #[serde(default)]
    pub contacts: Vec<ProposedClientContact>,
    /// The user opening the record.
    pub author_user_id: String,
}

/// One way of reaching a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedClientContact {
    /// Telephone, email, address, emergency contact, or other.
    pub kind: ContactKind,
    /// The value as a person would write it down.
    pub value: String,
    /// What this particular one is, for example `mother` or `work`.
    #[serde(default)]
    pub label: Option<String>,
    /// Whether this is the one to try first for its kind.
    #[serde(default)]
    pub is_primary: bool,
}

/// A case, as the office rather than the evidence kernel understands it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedMatter {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The client this matter belongs to.
    pub client_id: String,
    /// How the matter is captioned on the docket.
    pub caption: String,
    /// The court's own number for it.
    #[serde(default)]
    pub court_number: Option<String>,
    /// The court hearing it.
    #[serde(default)]
    pub court_id: Option<String>,
    /// Where the matter stands in the office. Defaults to open.
    #[serde(default)]
    pub status: Option<MatterStatus>,
    /// Where the client is. Defaults to unknown, which is not the same as out.
    #[serde(default)]
    pub custody_state: Option<CustodyState>,
    /// Where negotiation stands. Defaults to none.
    #[serde(default)]
    pub offer_state: Option<OfferState>,
    /// The offer in the defender's own words.
    #[serde(default)]
    pub offer_summary: Option<String>,
    /// The charges, summarized for a docket line.
    #[serde(default)]
    pub charge_summary: Option<String>,
    /// When the office took it, in `YYYY-MM-DD`.
    #[serde(default)]
    pub opened_on: Option<String>,
    /// When the client was last spoken to, in `YYYY-MM-DD`.
    #[serde(default)]
    pub last_contact_on: Option<String>,
    /// The kernel case holding this matter's discovery, when one exists.
    ///
    /// A bare identifier. Nothing in this crate resolves it; the application
    /// layer that holds both databases does.
    #[serde(default)]
    pub evidence_case_id: Option<String>,
    /// The user opening the matter.
    pub author_user_id: String,
}

/// Somebody staffed onto a matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAssignment {
    /// The matter being staffed.
    pub matter_id: String,
    /// The person being staffed onto it.
    pub user_id: String,
    /// What they are doing on it.
    pub role: AssignmentRole,
    /// The user making the assignment.
    pub assigned_by_user_id: String,
}

/// A court setting, which spans every matter of one client that it covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedAppearance {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Whose setting it is. Matters from any other client are refused.
    pub client_id: String,
    /// Every matter this one setting covers. At least one is required.
    pub matter_ids: Vec<String>,
    /// The court sitting.
    #[serde(default)]
    pub court_id: Option<String>,
    /// The judge, when the office knows which one.
    #[serde(default)]
    pub judge_id: Option<String>,
    /// The day, in `YYYY-MM-DD`.
    pub appearance_date: String,
    /// The time, in 24-hour `HH:MM`, when the docket gives one.
    #[serde(default)]
    pub appearance_time: Option<String>,
    /// What the setting is for.
    pub appearance_type: AppearanceType,
    /// Anything worth recording about the setting itself.
    #[serde(default)]
    pub notes: Option<String>,
    /// The user scheduling it.
    pub author_user_id: String,
}

/// Something owed by a date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedDeadline {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The matter it falls on.
    pub matter_id: String,
    /// What is owed.
    pub description: String,
    /// When, in `YYYY-MM-DD`.
    pub due_date: String,
    /// Where it comes from, which decides whether the date can move.
    pub origin: DeadlineOrigin,
    /// The user recording it.
    pub author_user_id: String,
}

/// A note on a client, a matter, or a setting.
///
/// Exactly one of the three scopes may be filled. A note carrying two of them
/// is refused rather than silently filed under the first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedNote {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The person this note follows, across every matter they have.
    #[serde(default)]
    pub client_id: Option<String>,
    /// The matter this note stays with.
    #[serde(default)]
    pub matter_id: Option<String>,
    /// The setting this note belongs to.
    #[serde(default)]
    pub appearance_id: Option<String>,
    /// What the note says. Mentions are read out of this text.
    pub body: String,
    /// The user writing it. Immutable once written.
    pub author_user_id: String,
}

/// A named person's decision about a possible identity match.
///
/// There is no way to express "maybe" here, and that is the point: a candidate
/// is shown, and either a person links it or a person dismisses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedIdentityLinkDecision {
    /// The office's record of the person.
    pub client_id: String,
    /// The kernel case the entity lives in.
    pub evidence_case_id: String,
    /// The kernel entity being considered.
    pub evidence_entity_id: String,
    /// What the person decided.
    pub state: IdentityLinkState,
    /// Why the prompt was raised, in the words shown when it was offered.
    #[serde(default)]
    pub matched_on: Option<String>,
    /// The user deciding.
    pub author_user_id: String,
}

/// A court the office appears in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedCourt {
    /// Stable identifier. A generated one is used when this is absent.
    #[serde(default)]
    pub id: Option<String>,
    /// The court's name.
    pub name: String,
    /// Division or department, when the court has them.
    #[serde(default)]
    pub division: Option<String>,
    /// Street address.
    #[serde(default)]
    pub address: Option<String>,
    /// Courtroom.
    #[serde(default)]
    pub room: Option<String>,
}
