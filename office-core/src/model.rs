//! The office layer's vocabulary.
//!
//! Every enum here has an `as_str` returning the **stable database
//! representation**, mirrored by a `CHECK(... IN (...))` constraint in the
//! migration that owns the column. Adding a variant means editing both the enum
//! and the SQL constraint, exactly as it does in the evidence kernel.

use serde::{Deserialize, Serialize};

/// Where a matter stands in the office, not in the case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterStatus {
    /// Actively being worked.
    Open,
    /// Awaiting formal appointment or conflict clearance.
    PendingAppointment,
    /// Disposed; kept for the record rather than deleted.
    Closed,
    /// Moved to another office or another defender.
    Transferred,
    /// The office withdrew.
    Withdrawn,
}

impl MatterStatus {
    /// Every status, for a chooser.
    pub const ALL: [Self; 5] = [
        Self::Open,
        Self::PendingAppointment,
        Self::Closed,
        Self::Transferred,
        Self::Withdrawn,
    ];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::PendingAppointment => "pending_appointment",
            Self::Closed => "closed",
            Self::Transferred => "transferred",
            Self::Withdrawn => "withdrawn",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "open" => Some(Self::Open),
            "pending_appointment" => Some(Self::PendingAppointment),
            "closed" => Some(Self::Closed),
            "transferred" => Some(Self::Transferred),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }

    /// Returns whether a matter in this state still belongs on a working docket.
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Open | Self::PendingAppointment)
    }
}

/// Where the client is, which decides how urgent everything else is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustodyState {
    /// Nobody has recorded it. Distinct from knowing the client is out.
    Unknown,
    /// At liberty.
    Out,
    /// Held on this matter.
    InCustody,
    /// Released on bond or recognizance.
    ReleasedOnBond,
    /// Held on another authority's detainer.
    DetainedHold,
}

impl CustodyState {
    /// Every custody state, for a chooser.
    pub const ALL: [Self; 5] = [
        Self::Unknown,
        Self::Out,
        Self::InCustody,
        Self::ReleasedOnBond,
        Self::DetainedHold,
    ];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Out => "out",
            Self::InCustody => "in_custody",
            Self::ReleasedOnBond => "released_on_bond",
            Self::DetainedHold => "detained_hold",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "out" => Some(Self::Out),
            "in_custody" => Some(Self::InCustody),
            "released_on_bond" => Some(Self::ReleasedOnBond),
            "detained_hold" => Some(Self::DetainedHold),
            _ => None,
        }
    }

    /// Returns whether the client is being held.
    ///
    /// Reported, never scored: this says where the person is, not how bad the
    /// case is.
    pub const fn is_held(self) -> bool {
        matches!(self, Self::InCustody | Self::DetainedHold)
    }
}

/// Where the plea negotiation stands, as a matter of record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfferState {
    /// None has been made.
    None,
    /// Made and outstanding.
    Extended,
    /// With the client for a decision.
    UnderAdvisement,
    /// Declined by the client.
    Rejected,
    /// Accepted.
    Accepted,
    /// Withdrawn or lapsed.
    Expired,
}

impl OfferState {
    /// Every offer state, for a chooser.
    pub const ALL: [Self; 6] = [
        Self::None,
        Self::Extended,
        Self::UnderAdvisement,
        Self::Rejected,
        Self::Accepted,
        Self::Expired,
    ];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Extended => "extended",
            Self::UnderAdvisement => "under_advisement",
            Self::Rejected => "rejected",
            Self::Accepted => "accepted",
            Self::Expired => "expired",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "extended" => Some(Self::Extended),
            "under_advisement" => Some(Self::UnderAdvisement),
            "rejected" => Some(Self::Rejected),
            "accepted" => Some(Self::Accepted),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }
}

/// How somebody is staffed onto a matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentRole {
    /// Attorney of record.
    Attorney,
    /// Second chair.
    SecondChair,
    /// Investigator.
    Investigator,
    /// Paralegal.
    Paralegal,
    /// Social worker or mitigation specialist.
    SocialWorker,
    /// Supervising attorney.
    Supervisor,
}

impl AssignmentRole {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attorney => "attorney",
            Self::SecondChair => "second_chair",
            Self::Investigator => "investigator",
            Self::Paralegal => "paralegal",
            Self::SocialWorker => "social_worker",
            Self::Supervisor => "supervisor",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "attorney" => Some(Self::Attorney),
            "second_chair" => Some(Self::SecondChair),
            "investigator" => Some(Self::Investigator),
            "paralegal" => Some(Self::Paralegal),
            "social_worker" => Some(Self::SocialWorker),
            "supervisor" => Some(Self::Supervisor),
            _ => None,
        }
    }
}

/// What a court setting is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceType {
    /// First appearance and plea.
    Arraignment,
    /// Status or docket call.
    Status,
    /// Pretrial conference.
    Pretrial,
    /// Motion hearing.
    Motion,
    /// Change of plea.
    Plea,
    /// Trial setting.
    Trial,
    /// Sentencing.
    Sentencing,
    /// Post-disposition review.
    Review,
    /// Probation or supervision violation.
    Violation,
    /// Anything the vocabulary does not name.
    Other,
}

impl AppearanceType {
    /// Every appearance type, for a chooser.
    pub const ALL: [Self; 10] = [
        Self::Arraignment,
        Self::Status,
        Self::Pretrial,
        Self::Motion,
        Self::Plea,
        Self::Trial,
        Self::Sentencing,
        Self::Review,
        Self::Violation,
        Self::Other,
    ];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arraignment => "arraignment",
            Self::Status => "status",
            Self::Pretrial => "pretrial",
            Self::Motion => "motion",
            Self::Plea => "plea",
            Self::Trial => "trial",
            Self::Sentencing => "sentencing",
            Self::Review => "review",
            Self::Violation => "violation",
            Self::Other => "other",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "arraignment" => Some(Self::Arraignment),
            "status" => Some(Self::Status),
            "pretrial" => Some(Self::Pretrial),
            "motion" => Some(Self::Motion),
            "plea" => Some(Self::Plea),
            "trial" => Some(Self::Trial),
            "sentencing" => Some(Self::Sentencing),
            "review" => Some(Self::Review),
            "violation" => Some(Self::Violation),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

/// Where a deadline comes from, which decides whether it can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeadlineOrigin {
    /// Fixed by statute or rule. Not negotiable.
    Statutory,
    /// Set by the court in this case.
    CourtOrdered,
    /// The defender's own working target.
    SelfImposed,
}

impl DeadlineOrigin {
    /// Every origin, for a chooser.
    pub const ALL: [Self; 3] = [Self::Statutory, Self::CourtOrdered, Self::SelfImposed];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statutory => "statutory",
            Self::CourtOrdered => "court_ordered",
            Self::SelfImposed => "self_imposed",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "statutory" => Some(Self::Statutory),
            "court_ordered" => Some(Self::CourtOrdered),
            "self_imposed" => Some(Self::SelfImposed),
            _ => None,
        }
    }
}

/// What a note is attached to. Exactly one of the three, never two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteScope {
    /// Follows the person across every matter they have.
    Client,
    /// Stays with one case.
    Matter,
    /// Belongs to one court setting.
    Appearance,
}

impl NoteScope {
    /// Every scope, for a chooser.
    pub const ALL: [Self; 3] = [Self::Client, Self::Matter, Self::Appearance];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Matter => "matter",
            Self::Appearance => "appearance",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "client" => Some(Self::Client),
            "matter" => Some(Self::Matter),
            "appearance" => Some(Self::Appearance),
            _ => None,
        }
    }
}

/// What a named person decided about a possible identity match.
///
/// There is no `proposed` variant, and its absence is the rule: a candidate is
/// computed and shown, never stored. Only a decision becomes a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityLinkState {
    /// A person confirmed the client and the evidence entity are the same human.
    Linked,
    /// A person declined the match. Both records are left exactly as they were.
    Dismissed,
}

impl IdentityLinkState {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Dismissed => "dismissed",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "linked" => Some(Self::Linked),
            "dismissed" => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// How to reach a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContactKind {
    /// Telephone number.
    Phone,
    /// Email address.
    Email,
    /// Mailing or residential address.
    Address,
    /// Somebody who can reach the client when the client cannot be reached.
    Emergency,
    /// Anything the vocabulary does not name.
    Other,
}

impl ContactKind {
    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Phone => "phone",
            Self::Email => "email",
            Self::Address => "address",
            Self::Emergency => "emergency",
            Self::Other => "other",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        match value {
            "phone" => Some(Self::Phone),
            "email" => Some(Self::Email),
            "address" => Some(Self::Address),
            "emergency" => Some(Self::Emergency),
            "other" => Some(Self::Other),
            _ => None,
        }
    }

    /// Returns whether a value of this kind is worth reducing to bare digits.
    ///
    /// The office search index carries both forms of a phone number, because a
    /// tokenizer that splits on punctuation cannot match an unpunctuated query
    /// against a punctuated record.
    pub const fn is_dialable(self) -> bool {
        matches!(self, Self::Phone | Self::Emergency)
    }
}

/// How the office records a client's sex, as an intake sheet asks it.
///
/// Absence is a value: a client with no recorded sex is one nobody asked,
/// which is not the same as one who answered. `Another` is the honest third
/// row for every answer the two-value form does not name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    /// Recorded as female.
    Female,
    /// Recorded as male.
    Male,
    /// Recorded as something the two-value form does not name.
    Another,
}

impl Sex {
    /// Every value, for a chooser. The three differ in their first letter, so
    /// a dropdown list is quick-selected by typing `f`, `m`, or `a`.
    pub const ALL: [Self; 3] = [Self::Female, Self::Male, Self::Another];

    /// Returns the stable database representation.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Female => "female",
            Self::Male => "male",
            Self::Another => "another",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sex| sex.as_str() == value)
    }
}

/// A mention riding on a note, addressed to a role rather than a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MentionTag {
    /// `@investigator`
    Investigator,
    /// `@socialwork`
    SocialWork,
    /// `@immigration`
    Immigration,
    /// `@supervisor`
    Supervisor,
}

impl MentionTag {
    /// Every tag a note may carry.
    pub const ALL: [Self; 4] = [
        Self::Investigator,
        Self::SocialWork,
        Self::Immigration,
        Self::Supervisor,
    ];

    /// Returns the stable database representation, which is also the text that
    /// follows `@` in a note body.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Investigator => "investigator",
            Self::SocialWork => "socialwork",
            Self::Immigration => "immigration",
            Self::Supervisor => "supervisor",
        }
    }

    /// Parses the stable database representation.
    pub fn from_db(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tag| tag.as_str() == value)
    }
}
