//! Serializable read models for the Court and Office surfaces.
//!
//! These are the stable contract the native workspace consumes, the same way
//! the kernel's `views` module is. Enum-valued database columns surface as
//! plain `String`, because they are the stable vocabulary a reader may quote
//! back into a write.
//!
//! Nothing here scores anything. A docket row reports what is scheduled, what
//! is owed, and where the client is; it never ranks a caseload or predicts an
//! outcome. The evidence-posture summary a Court row also carries is composed
//! one layer up, by the application layer that can see both databases, and it
//! obeys the same rule.

use serde::Serialize;

/// Every court setting on one day, in the order they are called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocketDay {
    /// The day, in `YYYY-MM-DD`.
    pub date: String,
    /// The day's name, so a reader knows a Monday from a Friday at a glance.
    pub weekday: String,
    /// One entry per setting, never one per matter.
    pub settings: Vec<DocketEntry>,
    /// Deadlines falling on this same day, which a settings list would hide.
    pub deadlines_due: Vec<DeadlineRow>,
}

/// One court setting, spanning every matter of one client that it covers.
///
/// This is the row the Court pane renders. A client called on three related
/// matters at 9:00 produces one of these with three [`DocketMatterLine`]s — not
/// three rows, which is the duplicate-setting error the office layer exists in
/// part to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocketEntry {
    /// Appearance identifier.
    pub id: String,
    /// The day, in `YYYY-MM-DD`.
    pub date: String,
    /// The time in 24-hour `HH:MM`, absent when the docket gives none.
    pub time: Option<String>,
    /// What the setting is for.
    pub appearance_type: String,
    /// The court sitting.
    pub court: Option<String>,
    /// The courtroom, when recorded.
    pub room: Option<String>,
    /// The judge, when the office knows which one.
    pub judge: Option<String>,
    /// Client identifier.
    pub client_id: String,
    /// The client's name.
    pub client: String,
    /// Every matter this setting covers.
    pub matters: Vec<DocketMatterLine>,
    /// The most recent client contact across the covered matters.
    pub last_contact: Option<String>,
    /// Open deadlines across the covered matters, however far out.
    pub open_deadlines: u32,
    /// Notes written on this setting, current versions only.
    pub notes: u32,
    /// What was recorded as having happened, once the setting is past.
    pub outcome: Option<String>,
}

/// One matter inside a setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocketMatterLine {
    /// Matter identifier.
    pub id: String,
    /// How the matter is captioned.
    pub caption: String,
    /// The court's own number, which is what gets read aloud.
    pub court_number: Option<String>,
    /// The charges, summarized.
    pub charge_summary: Option<String>,
    /// Where the matter stands in the office.
    pub status: String,
    /// Where the client is.
    pub custody_state: String,
    /// Where negotiation stands.
    pub offer_state: String,
    /// The offer in the defender's own words.
    pub offer_summary: Option<String>,
    /// How this matter is being handled differently inside a shared setting.
    pub override_note: Option<String>,
    /// The kernel case holding this matter's discovery, when one exists.
    ///
    /// A bare identifier: this crate never resolves it, and holds nothing else
    /// about the evidence.
    pub evidence_case_id: Option<String>,
}

/// Deadlines coming up, nearest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpcomingDeadlines {
    /// The date the window was measured from, in `YYYY-MM-DD`.
    pub as_of: String,
    /// How many days ahead the window reaches.
    pub within_days: u32,
    /// Deadlines already past and still unsatisfied, oldest first.
    pub overdue: Vec<DeadlineRow>,
    /// Deadlines inside the window, nearest first.
    pub upcoming: Vec<DeadlineRow>,
}

/// One thing owed by a date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadlineRow {
    /// Deadline identifier.
    pub id: String,
    /// The matter it falls on.
    pub matter_id: String,
    /// How that matter is captioned.
    pub matter: String,
    /// The court's own number for the matter.
    pub court_number: Option<String>,
    /// Whose matter it is.
    pub client: String,
    /// What is owed.
    pub description: String,
    /// When, in `YYYY-MM-DD`.
    pub due_date: String,
    /// Days from the measuring date, negative when already past.
    pub days_remaining: i64,
    /// `statutory`, `court_ordered`, or `self_imposed`.
    pub origin: String,
    /// Whether it has been met.
    pub satisfied: bool,
}

/// A person, and everything the office knows about them across matters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientProfile {
    /// Client identifier.
    pub id: String,
    /// The person's name as the office records it.
    pub display_name: String,
    /// Date of birth in `YYYY-MM-DD`, when known.
    pub date_of_birth: Option<String>,
    /// `female`, `male`, or `another`, when recorded.
    pub sex: Option<String>,
    /// The language they ask to be spoken to in, when recorded.
    pub preferred_language: Option<String>,
    /// Anything recorded about the person rather than a case.
    pub notes: Option<String>,
    /// Other names the person goes by.
    pub aliases: Vec<String>,
    /// How to reach them.
    pub contacts: Vec<ClientContactRow>,
    /// Every matter, open ones first.
    pub matters: Vec<MatterSummary>,
    /// The next setting across all of their matters.
    pub next_setting: Option<AppearanceSummary>,
    /// Who opened the record.
    pub opened_by: String,
    /// When it was opened.
    pub created_at: String,
}

/// One way of reaching a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ClientContactRow {
    /// Contact identifier.
    pub id: String,
    /// `phone`, `email`, `address`, `emergency`, or `other`.
    pub kind: String,
    /// The value as a person wrote it down, punctuation and all.
    pub value: String,
    /// What this particular one is, for example `mother` or `work`.
    pub label: Option<String>,
    /// Whether this is the one to try first for its kind.
    pub is_primary: bool,
}

/// A matter reduced to the line a list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatterSummary {
    /// Matter identifier.
    pub id: String,
    /// How the matter is captioned.
    pub caption: String,
    /// The court's own number for it.
    pub court_number: Option<String>,
    /// The court hearing it.
    pub court: Option<String>,
    /// Where it stands in the office.
    pub status: String,
    /// Where the client is.
    pub custody_state: String,
    /// Where negotiation stands.
    pub offer_state: String,
    /// The charges, summarized.
    pub charge_summary: Option<String>,
    /// The next setting on this matter.
    pub next_setting: Option<String>,
    /// Open deadlines on it.
    pub open_deadlines: u32,
    /// The kernel case holding its discovery, when one exists.
    pub evidence_case_id: Option<String>,
}

/// One matter and everything hanging off it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatterProfile {
    /// Matter identifier.
    pub id: String,
    /// Client identifier.
    pub client_id: String,
    /// The client's name.
    pub client: String,
    /// How the matter is captioned.
    pub caption: String,
    /// The court's own number for it.
    pub court_number: Option<String>,
    /// The court hearing it.
    pub court: Option<String>,
    /// Where it stands in the office.
    pub status: String,
    /// Where the client is.
    pub custody_state: String,
    /// Where negotiation stands.
    pub offer_state: String,
    /// The offer in the defender's own words.
    pub offer_summary: Option<String>,
    /// The charges, summarized.
    pub charge_summary: Option<String>,
    /// When the office took it.
    pub opened_on: Option<String>,
    /// When the client was last spoken to.
    pub last_contact_on: Option<String>,
    /// The kernel case holding its discovery, when one exists.
    pub evidence_case_id: Option<String>,
    /// Who is staffed on it.
    pub assignments: Vec<MatterAssignmentRow>,
    /// Other matters of the same client this one is tied to.
    pub related: Vec<MatterLinkRow>,
    /// Every setting, soonest first.
    pub settings: Vec<AppearanceSummary>,
    /// Everything owed, nearest first.
    pub deadlines: Vec<DeadlineRow>,
    /// Notes on the matter, current versions only, newest first.
    pub notes: Vec<NoteEntry>,
    /// Who opened it.
    pub opened_by: String,
}

/// Somebody staffed onto a matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatterAssignmentRow {
    /// The person staffed on.
    pub user: String,
    /// What they are doing on it.
    pub role: String,
    /// Who staffed them.
    pub assigned_by: String,
    /// When.
    pub assigned_at: String,
}

/// Another matter of the same client, and how it is tied to this one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatterLinkRow {
    /// The related matter's identifier.
    pub matter_id: String,
    /// How it is captioned.
    pub caption: String,
    /// The court's own number for it.
    pub court_number: Option<String>,
    /// `related`, `consolidated`, `probation_violation`, `companion`, or `refiled`.
    pub relation: String,
}

/// A setting reduced to the line a list shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppearanceSummary {
    /// Appearance identifier.
    pub id: String,
    /// The day, in `YYYY-MM-DD`.
    pub date: String,
    /// The time in 24-hour `HH:MM`, when the docket gives one.
    pub time: Option<String>,
    /// What the setting is for.
    pub appearance_type: String,
    /// The court sitting.
    pub court: Option<String>,
    /// How many of the client's matters this one setting covers.
    pub matters_covered: u32,
    /// What was recorded as having happened, once it is past.
    pub outcome: Option<String>,
}

/// One note, as its current version.
///
/// A superseded version never appears in a view. It is still in the database —
/// nothing deletes a note — and reachable through the history, which is what
/// makes an edit visible rather than silent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteEntry {
    /// Note identifier, which changes with every revision.
    pub id: String,
    /// `client`, `matter`, or `appearance`.
    pub scope: String,
    /// The identifier of whatever it is attached to.
    pub subject_id: String,
    /// What the note says.
    pub body: String,
    /// Which revision this is, counting from one.
    pub version: u32,
    /// The version this one replaced, when it replaced one.
    pub supersedes_note_id: Option<String>,
    /// Who wrote this version. Immutable.
    pub author: String,
    /// When this version was written. Immutable.
    pub created_at: String,
    /// Roles the note calls on, read out of its own text.
    pub mentions: Vec<String>,
}

/// One office-wide search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfficeSearchHit {
    /// `client`, `matter`, `note`, `user`, or `court`.
    pub kind: String,
    /// The identifier of the record found.
    pub subject_id: String,
    /// The record's name or caption.
    pub title: String,
    /// The matching text, with the matched terms marked.
    pub excerpt: String,
    /// Whose record it is, when the kind has an owner.
    pub client: Option<String>,
}

/// A person the office may already know, offered for a human to judge.
///
/// This is a question, never an answer. Nothing acts on one of these until a
/// named person resolves it, and declining leaves both records untouched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PossiblePerson {
    /// The existing client's identifier.
    pub client_id: String,
    /// That client's name.
    pub display_name: String,
    /// That client's date of birth, when recorded.
    pub date_of_birth: Option<String>,
    /// Why the prompt is being raised, in words a person can check.
    pub matched_on: Vec<String>,
    /// How many matters the existing record already carries.
    pub matters: u32,
}

/// Every version of one note, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NoteHistory {
    /// The identifier of the current version.
    pub current_id: String,
    /// Every version, oldest first, so an edit is visible rather than silent.
    pub versions: Vec<NoteEntry>,
}
