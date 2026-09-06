//! The public-defender office layer: clients, matters, calendar and notes.
//!
//! A sibling of the evidence kernel, never a layer inside it. The kernel knows
//! about originals, locators, review states and charge elements; this crate
//! knows about the people the office represents, the cases it carries for them,
//! where everyone has to be tomorrow, and what is owed by when.
//!
//! The two are joined by exactly one thing: a matter may carry an
//! `evidence_case_id`, a bare identifier with no foreign key. This crate never
//! resolves it and has no dependency on `evidence_intake` at all. That is the
//! whole of the privileged boundary — advocacy items, annotations and decision
//! briefs are not filtered out of these reads, they are unreachable from here.
//! Joining a matter to its evidence is the job of an application layer above
//! both, which is also the only place both databases are ever open at once.
//!
//! # Discipline
//!
//! The store follows the kernel's, deliberately: strict tables, numbered
//! idempotent migrations behind a `PRAGMA user_version` stamp, `prepare_cached`
//! on every query, and vocabulary enums whose `as_str` is mirrored by a
//! `CHECK(... IN (...))` constraint in SQL. Authorship is a real foreign key
//! into [`ProposedUser`]-created rows and is immutable once written, and notes
//! are append-only at the schema level, so an edit is a visible superseding
//! revision rather than a silent overwrite.
//!
//! # Portability
//!
//! The schema is written for a later move to PostgreSQL behind a server, so
//! SQLite-only constructs are avoided where a portable form is cheap. The ones
//! that remain, and what they become:
//!
//! | Here | PostgreSQL |
//! | --- | --- |
//! | `STRICT` tables | native, column types are already enforced |
//! | `datetime('now')` | `now()` |
//! | `date('now','localtime')` | `current_date` in the session's zone |
//! | partial `UNIQUE INDEX ... WHERE` | the same syntax |
//! | FTS5 over `search_documents` | a `tsvector` column and a GIN index |
//! | dates as TEXT `YYYY-MM-DD` | `DATE` |
//! | times as TEXT `HH:MM` | `TIME` |
//!
//! # Two clocks
//!
//! Business dates are **local** and audit timestamps are **UTC**. That reads
//! like an inconsistency and is not one: a court setting happens on the day the
//! courthouse says it does, while an append-only trail is only comparable if
//! every row on it is stamped in one zone. There is exactly one local clock,
//! [`civil_date::today`], and every `created_at` default is `datetime('now')`.

mod authoring;
pub mod civil_date;
mod error;
mod fixture;
mod model;
mod store;
mod views;

pub use authoring::{
    ProposedAppearance, ProposedAssignment, ProposedClient, ProposedClientContact, ProposedCourt,
    ProposedDeadline, ProposedIdentityLinkDecision, ProposedMatter, ProposedNote, ProposedUser,
};
pub use civil_date::{CivilDate, Weekday};
pub use error::{Error, Result};
pub use fixture::OfficeFixture;
pub use model::{
    AppearanceType, AssignmentRole, ContactKind, CustodyState, DeadlineOrigin, IdentityLinkState,
    MatterStatus, MentionTag, NoteScope, OfferState, Sex,
};
pub use store::{ASSIGNMENT_ROLES, COMMON_LANGUAGES, CONTACT_KINDS, OfficeStore, USER_ROLES};
pub use views::{
    AppearanceSummary, ClientContactRow, ClientProfile, DeadlineRow, DocketDay, DocketEntry,
    DocketMatterLine, MatterAssignmentRow, MatterLinkRow, MatterProfile, MatterSummary, NoteEntry,
    NoteHistory, OfficeSearchHit, PossiblePerson, UpcomingDeadlines,
};
