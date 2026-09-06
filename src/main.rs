//! Command-line shell for the collation kernel.

use std::fs;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use evidence_intake::{
    AdvocacyKind, CaseId, ChargePosture, DemoFixture, EdgeKind, ElementAssessment, EntityKind,
    ExportAudience, KeyframeIndex, NodeKind, NodeRef, NormalizedBatch, OfficeDesk,
    ProposedAdvocacyItem, ProposedAnnotation, ProposedBrief, ProposedCase, ProposedCharge,
    ProposedElement, ProposedElementMapping, ProposedEntity, ProposedLink, ProposedProduction,
    ProposedProposition, Result, ReviewDecision, ReviewState, ReviewTarget, Store, SuggestionKind,
};
use office_core::{
    AppearanceType, AssignmentRole, ContactKind, CustodyState, DeadlineOrigin, IdentityLinkState,
    MatterStatus, MentionTag, NoteScope, OfferState, OfficeFixture, ProposedAppearance,
    ProposedAssignment, ProposedClient, ProposedClientContact, ProposedCourt, ProposedDeadline,
    ProposedIdentityLinkDecision, ProposedMatter, ProposedNote, ProposedUser, Sex,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// `SQLite` case database.
    #[arg(long, default_value = "evidence.sqlite", global = true)]
    database: PathBuf,
    /// `SQLite` office database. Defaults to `office.sqlite` beside the case
    /// database, which is the file-level shape of the same boundary the two
    /// schemas keep: two databases, no shared transaction, no cross-database
    /// foreign key.
    #[arg(long, global = true)]
    office_database: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create or migrate the local database.
    Init,
    /// Load a permanent hand-authored fixture.
    Seed {
        #[arg(value_enum, default_value_t = FixtureName::VehicleStop)]
        fixture: FixtureName,
    },
    /// List cases on the docket.
    Cases,
    /// Open a new case and its first production.
    NewCase {
        /// The name as it should appear on the docket.
        #[arg(long)]
        name: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
        /// Docket, incident, or file number.
        #[arg(long)]
        reference: Option<String>,
        /// Court or charging jurisdiction.
        #[arg(long)]
        jurisdiction: Option<String>,
        /// Label for the first production. Defaults to `Initial production`.
        #[arg(long)]
        production: Option<String>,
    },
    /// Open a new production on an existing case.
    NewProduction {
        /// Existing case that will own the production.
        case_id: String,
        /// How this delivery is labelled on the ledger.
        #[arg(long)]
        label: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
        /// When the production was received.
        #[arg(long)]
        received: Option<String>,
        /// Who produced it.
        #[arg(long)]
        from: Option<String>,
        /// Anything worth recording about the delivery.
        #[arg(long)]
        notes: Option<String>,
    },
    /// List productions on a case.
    Productions {
        /// Existing case identifier.
        case_id: String,
    },
    /// Render a decision-oriented view as JSON.
    View {
        /// Stable case identifier.
        case_id: String,
        #[command(subcommand)]
        view: View,
    },
    /// Inspect and record human review decisions.
    Review {
        /// Stable case identifier.
        case_id: String,
        #[command(subcommand)]
        action: ReviewAction,
    },
    /// Run deterministic analyzers and propose what they find, for review.
    Suggest {
        /// Stable case identifier.
        case_id: String,
        /// Which analyzer to run; repeat to select several. All when omitted.
        #[arg(long = "analyzer", value_enum)]
        analyzers: Vec<AnalyzerArg>,
    },
    /// Find excerpts by their words, best match first.
    Search {
        /// Stable case identifier.
        case_id: String,
        /// Full-text query. Supports `"exact phrase"`, `AND`, `OR`, `NOT`, `term*`.
        query: String,
        /// Most hits to return.
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
    /// Produce a source-linked export of the case.
    Export {
        /// Stable case identifier.
        case_id: String,
        /// Who the export is for. `disclosable` never reads privileged tables.
        #[arg(long, value_enum, default_value_t = AudienceArg::Disclosable)]
        audience: AudienceArg,
    },
    /// Write a person's own reading of the case into it.
    Author {
        /// Stable case identifier.
        case_id: String,
        #[command(subcommand)]
        item: AuthorItem,
    },
    /// Import a `NormalizedBatch` JSON document produced by an extraction adapter.
    Ingest {
        /// Existing case. Must match `case_id` inside the batch.
        case_id: String,
        /// Path to a `NormalizedBatch` JSON document. `-` reads stdin.
        path: PathBuf,
    },
    /// Store keyframe embeddings against derived stills the case already holds.
    IndexFrames {
        /// Existing case. Must match `case_id` inside the index.
        case_id: String,
        /// Path to a `KeyframeIndex` JSON document. `-` reads stdin.
        path: PathBuf,
    },
    /// Clients, matters, calendar, notes and office search.
    ///
    /// The office layer is a sibling of the evidence kernel, not a part of it:
    /// these commands run against `office.sqlite`, and only `docket` and
    /// `matter show` open the case database at all.
    Office {
        #[command(subcommand)]
        action: OfficeAction,
    },
    /// Find stills by a precomputed query vector. Does not rank by similarity.
    FindFrames {
        /// Existing case identifier.
        case_id: String,
        /// Embedding space the stills were indexed under.
        #[arg(long)]
        model: String,
        /// Query vector as a JSON array of numbers, or `@path` / `-` for stdin.
        vector: String,
        /// Most hits to return.
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
}

#[derive(Debug, Subcommand)]
enum AuthorItem {
    /// State a contested proposition. It enters unreviewed, like anything else.
    Proposition {
        /// The proposition as a person would state it.
        #[arg(long)]
        text: String,
        /// Named person accountable for it.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Assert a typed relationship between two nodes in the case.
    Link {
        /// Node type of the asserting side.
        #[arg(long, value_enum)]
        from_kind: NodeKindArg,
        /// Identifier of the asserting side.
        #[arg(long)]
        from: String,
        /// How the two nodes stand to one another.
        #[arg(long, value_enum)]
        relation: RelationArg,
        /// Node type of the side the relationship bears on.
        #[arg(long, value_enum)]
        to_kind: NodeKindArg,
        /// Identifier of that side.
        #[arg(long)]
        to: String,
        /// Why the relationship holds. Required: an edge has no original.
        #[arg(long)]
        rationale: String,
        /// Named person accountable for it.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Record a person, organization, object, or place in the case.
    Entity {
        /// What kind of thing this is.
        #[arg(long, value_enum)]
        kind: EntityKindArg,
        /// The name as it should be shown.
        #[arg(long)]
        name: String,
        /// Mark this entity as the client.
        #[arg(long)]
        client: bool,
        /// Anything worth recording about the identification itself.
        #[arg(long)]
        notes: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Record a charge and its statutory elements, in statutory order.
    Charge {
        /// The offense as a person would name it.
        #[arg(long)]
        label: String,
        /// One element; repeat once per element, in statutory order.
        #[arg(long = "element", required = true)]
        elements: Vec<String>,
        /// How the charge stands in the case.
        #[arg(long, value_enum, default_value_t = PostureArg::Charged)]
        posture: PostureArg,
        /// Statutory or other citation.
        #[arg(long)]
        citation: Option<String>,
        /// Felony, misdemeanor, infraction, or a local grade.
        #[arg(long)]
        grade: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Write a privileged work-product item.
    Work {
        /// Which kind of work product this is.
        #[arg(long, value_enum)]
        kind: AdvocacyArg,
        /// Short title.
        #[arg(long)]
        title: String,
        /// The analysis itself.
        #[arg(long)]
        body: String,
        /// Workflow state; `open` when omitted.
        #[arg(long)]
        status: Option<String>,
        /// Named person accountable for it.
        #[arg(long)]
        author: String,
        /// Revise this item, writing a new version rather than overwriting it.
        #[arg(long)]
        revises: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Attach a privileged note to one record.
    Note {
        /// Node type being annotated.
        #[arg(long, value_enum)]
        target_kind: NodeKindArg,
        /// Identifier of the record being annotated.
        #[arg(long)]
        target: String,
        /// The note itself.
        #[arg(long)]
        body: String,
        /// Named person accountable for it.
        #[arg(long)]
        author: String,
        /// Revise this note, writing a new version rather than overwriting it.
        #[arg(long)]
        revises: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Write the next version of a posture's decision brief.
    Brief {
        /// release, motions, negotiation, trial, sentencing, or appeal.
        #[arg(long)]
        posture: String,
        /// What the brief says.
        #[arg(long)]
        summary: String,
        /// Strong portions of the defense position.
        #[arg(long, default_value = "")]
        strengths: String,
        /// Material risks.
        #[arg(long, default_value = "")]
        risks: String,
        /// Questions that could change the advice.
        #[arg(long, default_value = "")]
        unresolved: String,
        /// Topics to discuss with the client.
        #[arg(long, default_value = "")]
        client_topics: String,
        /// Named person accountable for it.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Record how one proposition bears on one statutory element.
    Mapping {
        /// The element being mapped.
        #[arg(long)]
        element: String,
        /// The contested proposition bearing on it.
        #[arg(long)]
        proposition: String,
        /// The direction of that bearing. Not a weight.
        #[arg(long, value_enum)]
        assessment: AssessmentArg,
        /// Why the mapping reads that way.
        #[arg(long)]
        notes: Option<String>,
        /// Named person accountable for the assessment.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AnalyzerArg {
    TemporalOverlap,
    ContradictionCandidate,
    ConflictingAttribution,
    DuplicateEntity,
    UnsupportedProposition,
    UnmappedProposition,
    UnresolvedReference,
    ClockDisagreement,
}

impl From<AnalyzerArg> for SuggestionKind {
    fn from(value: AnalyzerArg) -> Self {
        match value {
            AnalyzerArg::TemporalOverlap => Self::TemporalOverlap,
            AnalyzerArg::ContradictionCandidate => Self::ContradictionCandidate,
            AnalyzerArg::ConflictingAttribution => Self::ConflictingAttribution,
            AnalyzerArg::DuplicateEntity => Self::DuplicateEntity,
            AnalyzerArg::UnsupportedProposition => Self::UnsupportedProposition,
            AnalyzerArg::UnmappedProposition => Self::UnmappedProposition,
            AnalyzerArg::UnresolvedReference => Self::UnresolvedReference,
            AnalyzerArg::ClockDisagreement => Self::ClockDisagreement,
        }
    }
}

/// Naming the audience is deliberate: producing a work file when a disclosable
/// export was meant is the mistake this command exists to make hard.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum AudienceArg {
    /// Nothing privileged. Safe to hand outside the defense team.
    Disclosable,
    /// The defense team's own complete file, privileged analysis included.
    WorkFile,
}

impl From<AudienceArg> for ExportAudience {
    fn from(value: AudienceArg) -> Self {
        match value {
            AudienceArg::Disclosable => Self::Disclosable,
            AudienceArg::WorkFile => Self::WorkFile,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EntityKindArg {
    Person,
    Organization,
    Object,
    Location,
}

impl From<EntityKindArg> for EntityKind {
    fn from(value: EntityKindArg) -> Self {
        match value {
            EntityKindArg::Person => Self::Person,
            EntityKindArg::Organization => Self::Organization,
            EntityKindArg::Object => Self::Object,
            EntityKindArg::Location => Self::Location,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AdvocacyArg {
    DefenseTheory,
    ProsecutionTheory,
    LegalIssue,
    MotionIssue,
    CrossExaminationPoint,
    InvestigationTask,
    NegotiationConsideration,
    MitigationTheme,
    AttorneyConclusion,
}

impl From<AdvocacyArg> for AdvocacyKind {
    fn from(value: AdvocacyArg) -> Self {
        match value {
            AdvocacyArg::DefenseTheory => Self::DefenseTheory,
            AdvocacyArg::ProsecutionTheory => Self::ProsecutionTheory,
            AdvocacyArg::LegalIssue => Self::LegalIssue,
            AdvocacyArg::MotionIssue => Self::MotionIssue,
            AdvocacyArg::CrossExaminationPoint => Self::CrossExaminationPoint,
            AdvocacyArg::InvestigationTask => Self::InvestigationTask,
            AdvocacyArg::NegotiationConsideration => Self::NegotiationConsideration,
            AdvocacyArg::MitigationTheme => Self::MitigationTheme,
            AdvocacyArg::AttorneyConclusion => Self::AttorneyConclusion,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PostureArg {
    Charged,
    LesserCandidate,
    Alternative,
    Dismissed,
}

impl From<PostureArg> for ChargePosture {
    fn from(value: PostureArg) -> Self {
        match value {
            PostureArg::Charged => Self::Charged,
            PostureArg::LesserCandidate => Self::LesserCandidate,
            PostureArg::Alternative => Self::Alternative,
            PostureArg::Dismissed => Self::Dismissed,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AssessmentArg {
    Supports,
    Opposes,
    Uncertain,
    Excluded,
}

impl From<AssessmentArg> for ElementAssessment {
    fn from(value: AssessmentArg) -> Self {
        match value {
            AssessmentArg::Supports => Self::Supports,
            AssessmentArg::Opposes => Self::Opposes,
            AssessmentArg::Uncertain => Self::Uncertain,
            AssessmentArg::Excluded => Self::Excluded,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum NodeKindArg {
    Content,
    Source,
    ContentGroup,
    Proposition,
    Event,
    Edge,
    Entity,
    Advocacy,
}

impl From<NodeKindArg> for NodeKind {
    fn from(value: NodeKindArg) -> Self {
        match value {
            NodeKindArg::Content => Self::Content,
            NodeKindArg::Source => Self::Source,
            NodeKindArg::ContentGroup => Self::ContentGroup,
            NodeKindArg::Proposition => Self::Proposition,
            NodeKindArg::Event => Self::Event,
            NodeKindArg::Edge => Self::Edge,
            NodeKindArg::Entity => Self::Entity,
            NodeKindArg::Advocacy => Self::Advocacy,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RelationArg {
    Supports,
    Contradicts,
    ConsistentWith,
    IndependentlyCorroborates,
    Impeaches,
    Qualifies,
    Explains,
    DerivedFrom,
    RefersTo,
    TemporallyOverlaps,
    PossiblySamePerson,
    ExpectedButMissing,
    RequiresFollowUp,
    RelevantTo,
    Quotes,
    Reports,
    Summarizes,
    Transcribes,
    Depicts,
    RecordsUtterance,
    Measures,
    BasedOn,
    AccountOf,
    CreatedAfter,
    RecordedDuring,
    CandidateSameOccurrence,
    SpeakerCandidate,
}

impl From<RelationArg> for EdgeKind {
    fn from(value: RelationArg) -> Self {
        match value {
            RelationArg::Supports => Self::Supports,
            RelationArg::Contradicts => Self::Contradicts,
            RelationArg::ConsistentWith => Self::ConsistentWith,
            RelationArg::IndependentlyCorroborates => Self::IndependentlyCorroborates,
            RelationArg::Impeaches => Self::Impeaches,
            RelationArg::Qualifies => Self::Qualifies,
            RelationArg::Explains => Self::Explains,
            RelationArg::DerivedFrom => Self::DerivedFrom,
            RelationArg::RefersTo => Self::RefersTo,
            RelationArg::TemporallyOverlaps => Self::TemporallyOverlaps,
            RelationArg::PossiblySamePerson => Self::PossiblySamePerson,
            RelationArg::ExpectedButMissing => Self::ExpectedButMissing,
            RelationArg::RequiresFollowUp => Self::RequiresFollowUp,
            RelationArg::RelevantTo => Self::RelevantTo,
            RelationArg::Quotes => Self::Quotes,
            RelationArg::Reports => Self::Reports,
            RelationArg::Summarizes => Self::Summarizes,
            RelationArg::Transcribes => Self::Transcribes,
            RelationArg::Depicts => Self::Depicts,
            RelationArg::RecordsUtterance => Self::RecordsUtterance,
            RelationArg::Measures => Self::Measures,
            RelationArg::BasedOn => Self::BasedOn,
            RelationArg::AccountOf => Self::AccountOf,
            RelationArg::CreatedAfter => Self::CreatedAfter,
            RelationArg::RecordedDuring => Self::RecordedDuring,
            RelationArg::CandidateSameOccurrence => Self::CandidateSameOccurrence,
            RelationArg::SpeakerCandidate => Self::SpeakerCandidate,
        }
    }
}

#[derive(Debug, Subcommand)]
enum ReviewAction {
    /// List records still awaiting a person, machine suggestions first.
    Queue,
    /// Record one review decision.
    Apply {
        /// Record type being reviewed.
        #[arg(long, value_enum)]
        target: TargetArg,
        /// Identifier of the record being reviewed.
        #[arg(long)]
        id: String,
        /// State to move the record into.
        #[arg(long, value_enum)]
        state: StateArg,
        /// Named person accountable for the decision.
        #[arg(long)]
        actor: String,
        /// Written reason; required to reject.
        #[arg(long)]
        basis: Option<String>,
        /// Exact original locator opened while verifying.
        #[arg(long)]
        locator: Option<String>,
    },
    /// Print the append-only review history.
    History {
        /// Limit the history to one record.
        #[arg(long)]
        id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TargetArg {
    Content,
    Source,
    Edge,
    Proposition,
    Event,
}

impl From<TargetArg> for ReviewTarget {
    fn from(value: TargetArg) -> Self {
        match value {
            TargetArg::Content => Self::Content,
            TargetArg::Source => Self::Source,
            TargetArg::Edge => Self::Edge,
            TargetArg::Proposition => Self::Proposition,
            TargetArg::Event => Self::Event,
        }
    }
}

/// Only the three states a person can produce are offered on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum StateArg {
    Reviewed,
    Verified,
    Rejected,
}

impl From<StateArg> for ReviewState {
    fn from(value: StateArg) -> Self {
        match value {
            StateArg::Reviewed => Self::Reviewed,
            StateArg::Verified => Self::Verified,
            StateArg::Rejected => Self::Rejected,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FixtureName {
    VehicleStop,
    HitAndRun,
}

#[derive(Debug, Subcommand)]
enum View {
    /// Counts and workflow state.
    Overview,
    /// Where the case stands: what each element rests on, and where it is thin.
    Standing,
    /// Production and completeness ledger.
    Discovery,
    /// Charge-element evidence matrix.
    Elements,
    /// Statements by or attributed to an entity.
    Witness {
        /// Entity identifier, e.g. `person-patel`.
        entity_id: String,
    },
    /// Lane-preserving timeline.
    Timeline,
    /// Source-grounded evidence grouped by normalized date and exact location.
    Collation,
    /// Suppression, identification, discovery, and other issue workspaces.
    Issues,
    /// Latest posture-specific client decision brief.
    Brief {
        /// release, motions, negotiation, trial, sentencing, or appeal
        posture: String,
    },
    /// Exact evidence supporting, contradicting, or qualifying a proposition.
    Proposition {
        /// Stable proposition identifier.
        proposition_id: String,
    },
    /// Compare charged offenses and lesser candidates by element.
    Offenses,
    /// Privileged notes currently attached to one record.
    Notes {
        /// Node type that was annotated.
        #[arg(value_enum)]
        target_kind: NodeKindArg,
        /// Identifier of the annotated record.
        target: String,
    },
    /// Every version of one work-product item, oldest first.
    WorkHistory {
        /// Any version's identifier.
        item_id: String,
    },
}

/// What the office layer can be asked to do from a terminal.
///
/// Every one of these runs against `office.sqlite`. Only `docket` and
/// `matter show` open the evidence database at all, and then only to read the
/// standing of a linked case: an office with no discovery yet still has a
/// docket, and that is the point of the boundary.
#[derive(Debug, Subcommand)]
enum OfficeAction {
    /// Create or migrate the local office database.
    Init,
    /// Load a hand-authored office fixture.
    Seed {
        /// Which caseload to seed.
        #[arg(value_enum, default_value_t = OfficeFixtureName::MisdemeanorDocket)]
        fixture: OfficeFixtureName,
        /// Anchor the calendar on this date rather than on today.
        #[arg(long)]
        anchor: Option<String>,
    },
    /// The people who can author office records.
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// The people the office represents.
    Client {
        #[command(subcommand)]
        action: ClientAction,
    },
    /// Cases as the office carries them.
    Matter {
        #[command(subcommand)]
        action: MatterAction,
    },
    /// Staff somebody onto a matter.
    Assign {
        /// The matter being staffed.
        #[arg(long)]
        matter: String,
        /// The user being staffed onto it.
        #[arg(long)]
        user: String,
        /// What they are doing on it.
        #[arg(long, value_enum)]
        role: AssignmentRoleArg,
        /// The user making the assignment.
        #[arg(long)]
        by: String,
    },
    /// Courts the office appears in.
    Court {
        #[command(subcommand)]
        action: CourtAction,
    },
    /// Court settings, each spanning every matter of one client it covers.
    Appearance {
        #[command(subcommand)]
        action: AppearanceAction,
    },
    /// Things owed by a date.
    Deadline {
        #[command(subcommand)]
        action: DeadlineAction,
    },
    /// Notes on a client, a matter, or a setting.
    Note {
        #[command(subcommand)]
        action: NoteAction,
    },
    /// Decide a possible identity match. Nothing is ever merged automatically.
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },
    /// Search clients, matters, contacts and numbers across the office.
    Search {
        /// Plain text. Punctuation is not query syntax here.
        query: String,
        /// Most hits to return.
        #[arg(long, default_value_t = 25)]
        limit: u32,
    },
    /// What the office has to be in court for, with the evidence posture of
    /// every linked case.
    Docket {
        /// `today`, or a day in `YYYY-MM-DD`.
        #[arg(long, default_value = "today")]
        date: String,
        /// The whole week containing that day, Monday through Sunday.
        #[arg(long)]
        week: bool,
    },
    /// What is owed, soonest first.
    Deadlines {
        /// `today`, or the day to count from in `YYYY-MM-DD`.
        #[arg(long, default_value = "today")]
        as_of: String,
        /// How many days ahead to look.
        #[arg(long, default_value_t = 14)]
        within: u32,
    },
}

/// Office users, who are the authors every office write names.
#[derive(Debug, Subcommand)]
enum UserAction {
    /// Record somebody who can author office records.
    Add {
        /// The name colleagues know them by.
        #[arg(long)]
        name: String,
        /// What they do in the office.
        #[arg(long)]
        role: String,
        /// Work email, when there is one.
        #[arg(long)]
        email: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// List everybody who can author.
    List,
}

/// The people the office represents.
#[derive(Debug, Subcommand)]
enum ClientAction {
    /// Open a client record.
    Add {
        /// The person's name as the office records it.
        #[arg(long)]
        name: String,
        /// Date of birth in `YYYY-MM-DD`.
        #[arg(long)]
        dob: Option<String>,
        /// How the office records the person's sex.
        #[arg(long)]
        sex: Option<SexArg>,
        /// The language they ask to be spoken to in.
        #[arg(long = "language")]
        language: Option<String>,
        /// Anything worth recording about the person rather than a case.
        #[arg(long)]
        notes: Option<String>,
        /// Another name they go by; repeatable.
        #[arg(long = "alias")]
        aliases: Vec<String>,
        /// A telephone number; repeatable, the first is primary.
        #[arg(long = "phone")]
        phones: Vec<String>,
        /// An email address; repeatable, the first is primary.
        #[arg(long = "email")]
        emails: Vec<String>,
        /// A postal address; repeatable, the first is primary.
        #[arg(long = "address")]
        addresses: Vec<String>,
        /// The user opening the record.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// List every client.
    List,
    /// Everything the office knows about one person, across every matter.
    Show {
        /// Stable client identifier.
        client_id: String,
    },
    /// Candidates a person should look at before a second record is opened.
    ///
    /// Returns possibilities and merges nothing. Only a named person's decision
    /// is ever written, through `office identity decide`.
    Duplicates {
        /// The name about to be entered.
        #[arg(long)]
        name: String,
        /// A contact value to compare on; repeatable.
        #[arg(long = "contact")]
        contacts: Vec<String>,
        /// A client to leave out, when checking an existing record.
        #[arg(long)]
        exclude: Option<String>,
    },
}

/// Cases as the office carries them.
#[derive(Debug, Subcommand)]
enum MatterAction {
    /// Open a matter for a client.
    Open {
        /// The client this matter belongs to.
        #[arg(long)]
        client: String,
        /// How the matter is captioned on the docket.
        #[arg(long)]
        caption: String,
        /// The court's own number for it.
        #[arg(long)]
        court_number: Option<String>,
        /// The court hearing it.
        #[arg(long)]
        court: Option<String>,
        /// Where the matter stands in the office.
        #[arg(long, value_enum)]
        status: Option<MatterStatusArg>,
        /// Where the client is.
        #[arg(long, value_enum)]
        custody: Option<CustodyArg>,
        /// Where negotiation stands.
        #[arg(long, value_enum)]
        offer: Option<OfferArg>,
        /// The offer in the defender's own words.
        #[arg(long)]
        offer_summary: Option<String>,
        /// The charges, summarized for a docket line.
        #[arg(long)]
        charges: Option<String>,
        /// When the office took it, in `YYYY-MM-DD`.
        #[arg(long)]
        opened: Option<String>,
        /// When the client was last spoken to, in `YYYY-MM-DD`.
        #[arg(long)]
        last_contact: Option<String>,
        /// The kernel case holding this matter's discovery.
        #[arg(long)]
        evidence_case: Option<String>,
        /// The user opening the matter.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// List every matter.
    List,
    /// One matter, with the standing of its evidence case when it has one.
    Show {
        /// Stable matter identifier.
        matter_id: String,
    },
    /// Move a matter's status.
    Status {
        /// Stable matter identifier.
        matter_id: String,
        /// Where the matter now stands.
        #[arg(long, value_enum)]
        status: MatterStatusArg,
    },
    /// Record where the client is.
    Custody {
        /// Stable matter identifier.
        matter_id: String,
        /// Where the client now is.
        #[arg(long, value_enum)]
        custody: CustodyArg,
    },
    /// Record where negotiation stands.
    Offer {
        /// Stable matter identifier.
        matter_id: String,
        /// Where the offer now stands.
        #[arg(long, value_enum)]
        offer: OfferArg,
        /// The offer in the defender's own words.
        #[arg(long)]
        summary: Option<String>,
    },
    /// Record that the client was spoken to.
    Contact {
        /// Stable matter identifier.
        matter_id: String,
        /// The day, in `YYYY-MM-DD`. Defaults to today.
        #[arg(long, default_value = "today")]
        on: String,
    },
    /// Point a matter at the kernel case holding its discovery.
    LinkEvidence {
        /// Stable matter identifier.
        matter_id: String,
        /// Stable evidence case identifier. Stored as given, never resolved
        /// here.
        #[arg(long = "case")]
        evidence_case: String,
    },
    /// Tie two matters of one client together.
    Relate {
        /// Stable matter identifier.
        matter_id: String,
        /// The related matter, which must belong to the same client.
        #[arg(long = "to")]
        related: String,
        /// How they stand to one another, in the office's own words.
        #[arg(long, default_value = "related")]
        relation: String,
    },
}

/// Courts and the judges who sit in them.
#[derive(Debug, Subcommand)]
enum CourtAction {
    /// Record a court.
    Add {
        /// The court's name.
        #[arg(long)]
        name: String,
        /// Division or department.
        #[arg(long)]
        division: Option<String>,
        /// Street address.
        #[arg(long)]
        address: Option<String>,
        /// Courtroom.
        #[arg(long)]
        room: Option<String>,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// List every court.
    List,
    /// Record a judge.
    Judge {
        /// The judge's name.
        #[arg(long)]
        name: String,
        /// The court they sit in.
        #[arg(long)]
        court: Option<String>,
    },
}

/// Court settings. One setting spans every matter of one client it covers.
#[derive(Debug, Subcommand)]
enum AppearanceAction {
    /// Put one setting on the calendar, spanning every matter it covers.
    Schedule {
        /// Whose setting it is.
        #[arg(long)]
        client: String,
        /// A matter this setting covers; repeat once per matter.
        #[arg(long = "matter", required = true)]
        matters: Vec<String>,
        /// The court sitting.
        #[arg(long)]
        court: Option<String>,
        /// The judge, when the office knows which one.
        #[arg(long)]
        judge: Option<String>,
        /// The day, in `YYYY-MM-DD`.
        #[arg(long)]
        date: String,
        /// The time, in 24-hour `HH:MM`.
        #[arg(long)]
        time: Option<String>,
        /// What the setting is for.
        #[arg(long = "type", value_enum)]
        appearance_type: AppearanceTypeArg,
        /// Anything worth recording about the setting itself.
        #[arg(long)]
        notes: Option<String>,
        /// The user scheduling it.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Add one more matter to an existing setting.
    Link {
        /// Stable appearance identifier.
        appearance_id: String,
        /// The matter to add, which must belong to the same client.
        #[arg(long)]
        matter: String,
        /// Why this matter stands differently from the rest of the setting.
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove one matter from a setting, leaving the setting standing.
    Unlink {
        /// Stable appearance identifier.
        appearance_id: String,
        /// The matter to remove.
        #[arg(long)]
        matter: String,
    },
    /// Strike a setting from the calendar. The record stays.
    Cancel {
        /// Stable appearance identifier.
        appearance_id: String,
        /// The day it was struck, in `YYYY-MM-DD`. Defaults to today.
        #[arg(long, default_value = "today")]
        on: String,
    },
    /// Record what happened at a setting.
    Outcome {
        /// Stable appearance identifier.
        appearance_id: String,
        /// What happened, in the office's own words.
        #[arg(long)]
        outcome: String,
    },
}

/// Things owed by a date.
#[derive(Debug, Subcommand)]
enum DeadlineAction {
    /// Record something owed by a date.
    Add {
        /// The matter it falls on.
        #[arg(long)]
        matter: String,
        /// What is owed.
        #[arg(long)]
        description: String,
        /// When, in `YYYY-MM-DD`.
        #[arg(long)]
        due: String,
        /// Where it comes from, which decides whether the date can move.
        #[arg(long, value_enum)]
        origin: DeadlineOriginArg,
        /// The user recording it.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Mark a deadline met.
    Satisfy {
        /// Stable deadline identifier.
        deadline_id: String,
        /// The day it was met, in `YYYY-MM-DD`. Defaults to today.
        #[arg(long, default_value = "today")]
        on: String,
    },
}

/// Notes, which are append-only and revised by superseding.
#[derive(Debug, Subcommand)]
enum NoteAction {
    /// Write a note. Exactly one scope may be given.
    Write {
        /// The person this note follows, across every matter they have.
        #[arg(long)]
        client: Option<String>,
        /// The matter this note stays with.
        #[arg(long)]
        matter: Option<String>,
        /// The setting this note belongs to.
        #[arg(long)]
        appearance: Option<String>,
        /// What the note says. Mentions are read out of this text.
        #[arg(long)]
        body: String,
        /// The user writing it. Immutable once written.
        #[arg(long)]
        author: String,
        /// Stable identifier; generated when omitted.
        #[arg(long)]
        id: Option<String>,
    },
    /// Write the next version of a note. The old one is never altered.
    Revise {
        /// The note being revised.
        note_id: String,
        /// What the note now says.
        #[arg(long)]
        body: String,
        /// The user writing this version.
        #[arg(long)]
        author: String,
    },
    /// The current notes on one client, matter, or setting.
    List {
        /// Which kind of subject the identifier names.
        #[arg(long, value_enum)]
        scope: NoteScopeArg,
        /// The client, matter, or appearance identifier.
        #[arg(long)]
        subject: String,
    },
    /// Every version of one note, oldest first.
    History {
        /// Any version of the note.
        note_id: String,
    },
    /// Current notes calling on one part of the office.
    Mentioning {
        /// Which mention to look for.
        #[arg(long, value_enum)]
        tag: MentionTagArg,
    },
}

/// Decisions about whether an office client and a kernel entity are one person.
#[derive(Debug, Subcommand)]
enum IdentityAction {
    /// Record a named person's decision about one candidate.
    Decide {
        /// The office's record of the person.
        #[arg(long)]
        client: String,
        /// The kernel case the entity lives in.
        #[arg(long = "case")]
        evidence_case: String,
        /// The kernel entity being considered.
        #[arg(long)]
        entity: String,
        /// Linked, or dismissed so it is not offered again.
        #[arg(long, value_enum)]
        state: IdentityStateArg,
        /// Why the prompt was raised, in the words it was offered in.
        #[arg(long)]
        matched_on: Option<String>,
        /// The user deciding.
        #[arg(long)]
        author: String,
    },
}

/// Which office fixture to seed.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OfficeFixtureName {
    /// A misdemeanor caseload at the scale the docket gate is measured against.
    MisdemeanorDocket,
}

/// How the office records a client's sex.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SexArg {
    /// Recorded as female.
    Female,
    /// Recorded as male.
    Male,
    /// Recorded as something the two-value form does not name.
    Another,
}

impl From<SexArg> for Sex {
    fn from(value: SexArg) -> Self {
        match value {
            SexArg::Female => Self::Female,
            SexArg::Male => Self::Male,
            SexArg::Another => Self::Another,
        }
    }
}

/// Where a matter stands in the office.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum MatterStatusArg {
    /// The office is carrying it.
    Open,
    /// Taken in, waiting on the appointment order.
    PendingAppointment,
    /// Finished.
    Closed,
    /// Gone to another office.
    Transferred,
    /// The office is off the case.
    Withdrawn,
}

impl From<MatterStatusArg> for MatterStatus {
    fn from(value: MatterStatusArg) -> Self {
        match value {
            MatterStatusArg::Open => Self::Open,
            MatterStatusArg::PendingAppointment => Self::PendingAppointment,
            MatterStatusArg::Closed => Self::Closed,
            MatterStatusArg::Transferred => Self::Transferred,
            MatterStatusArg::Withdrawn => Self::Withdrawn,
        }
    }
}

/// Where the client is.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum CustodyArg {
    /// Nobody has recorded it, which is not the same as out.
    Unknown,
    /// At liberty.
    Out,
    /// Held.
    InCustody,
    /// Out on bond.
    ReleasedOnBond,
    /// Held on another jurisdiction's hold.
    DetainedHold,
}

impl From<CustodyArg> for CustodyState {
    fn from(value: CustodyArg) -> Self {
        match value {
            CustodyArg::Unknown => Self::Unknown,
            CustodyArg::Out => Self::Out,
            CustodyArg::InCustody => Self::InCustody,
            CustodyArg::ReleasedOnBond => Self::ReleasedOnBond,
            CustodyArg::DetainedHold => Self::DetainedHold,
        }
    }
}

/// Where negotiation stands.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum OfferArg {
    /// Nothing has been offered.
    None,
    /// An offer is on the table.
    Extended,
    /// The client is considering it.
    UnderAdvisement,
    /// Turned down.
    Rejected,
    /// Taken.
    Accepted,
    /// Withdrawn by time.
    Expired,
}

impl From<OfferArg> for OfferState {
    fn from(value: OfferArg) -> Self {
        match value {
            OfferArg::None => Self::None,
            OfferArg::Extended => Self::Extended,
            OfferArg::UnderAdvisement => Self::UnderAdvisement,
            OfferArg::Rejected => Self::Rejected,
            OfferArg::Accepted => Self::Accepted,
            OfferArg::Expired => Self::Expired,
        }
    }
}

/// What somebody staffed onto a matter is doing on it.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum AssignmentRoleArg {
    /// Counsel of record.
    Attorney,
    /// Second chair.
    SecondChair,
    /// Investigator.
    Investigator,
    /// Paralegal.
    Paralegal,
    /// Social worker.
    SocialWorker,
    /// Supervising attorney.
    Supervisor,
}

impl From<AssignmentRoleArg> for AssignmentRole {
    fn from(value: AssignmentRoleArg) -> Self {
        match value {
            AssignmentRoleArg::Attorney => Self::Attorney,
            AssignmentRoleArg::SecondChair => Self::SecondChair,
            AssignmentRoleArg::Investigator => Self::Investigator,
            AssignmentRoleArg::Paralegal => Self::Paralegal,
            AssignmentRoleArg::SocialWorker => Self::SocialWorker,
            AssignmentRoleArg::Supervisor => Self::Supervisor,
        }
    }
}

/// What a setting is for.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum AppearanceTypeArg {
    /// First appearance on the charge.
    Arraignment,
    /// Status conference.
    Status,
    /// Pretrial conference.
    Pretrial,
    /// Motion hearing.
    Motion,
    /// Plea.
    Plea,
    /// Trial.
    Trial,
    /// Sentencing.
    Sentencing,
    /// Review hearing.
    Review,
    /// Probation or supervision violation.
    Violation,
    /// Anything the office has to be there for that is none of the above.
    Other,
}

impl From<AppearanceTypeArg> for AppearanceType {
    fn from(value: AppearanceTypeArg) -> Self {
        match value {
            AppearanceTypeArg::Arraignment => Self::Arraignment,
            AppearanceTypeArg::Status => Self::Status,
            AppearanceTypeArg::Pretrial => Self::Pretrial,
            AppearanceTypeArg::Motion => Self::Motion,
            AppearanceTypeArg::Plea => Self::Plea,
            AppearanceTypeArg::Trial => Self::Trial,
            AppearanceTypeArg::Sentencing => Self::Sentencing,
            AppearanceTypeArg::Review => Self::Review,
            AppearanceTypeArg::Violation => Self::Violation,
            AppearanceTypeArg::Other => Self::Other,
        }
    }
}

/// Where a deadline comes from, which decides whether the date can move.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum DeadlineOriginArg {
    /// Fixed by statute or rule.
    Statutory,
    /// Set by the court.
    CourtOrdered,
    /// The office's own working date.
    SelfImposed,
}

impl From<DeadlineOriginArg> for DeadlineOrigin {
    fn from(value: DeadlineOriginArg) -> Self {
        match value {
            DeadlineOriginArg::Statutory => Self::Statutory,
            DeadlineOriginArg::CourtOrdered => Self::CourtOrdered,
            DeadlineOriginArg::SelfImposed => Self::SelfImposed,
        }
    }
}

/// Which kind of subject a note hangs off.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum NoteScopeArg {
    /// The person, across every matter they have.
    Client,
    /// One matter.
    Matter,
    /// One setting.
    Appearance,
}

impl From<NoteScopeArg> for NoteScope {
    fn from(value: NoteScopeArg) -> Self {
        match value {
            NoteScopeArg::Client => Self::Client,
            NoteScopeArg::Matter => Self::Matter,
            NoteScopeArg::Appearance => Self::Appearance,
        }
    }
}

/// Which part of the office a note calls on.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum MentionTagArg {
    /// Investigation.
    Investigator,
    /// Social work.
    Socialwork,
    /// Immigration consequences.
    Immigration,
    /// Supervision.
    Supervisor,
}

impl From<MentionTagArg> for MentionTag {
    fn from(value: MentionTagArg) -> Self {
        match value {
            MentionTagArg::Investigator => Self::Investigator,
            MentionTagArg::Socialwork => Self::SocialWork,
            MentionTagArg::Immigration => Self::Immigration,
            MentionTagArg::Supervisor => Self::Supervisor,
        }
    }
}

/// What a person decided about a possible identity match.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum IdentityStateArg {
    /// The candidate is the same person.
    Linked,
    /// The candidate is somebody else, and is not to be offered again.
    Dismissed,
}

impl From<IdentityStateArg> for IdentityLinkState {
    fn from(value: IdentityStateArg) -> Self {
        match value {
            IdentityStateArg::Linked => Self::Linked,
            IdentityStateArg::Dismissed => Self::Dismissed,
        }
    }
}

/// One row of a plain office listing, rendered as JSON.
#[derive(Debug, Serialize)]
struct OfficeRow {
    /// Stable identifier.
    id: String,
    /// The name it is known by.
    name: String,
    /// What the person does in the office, on a user listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
}

/// Runs one office command.
///
/// The evidence database is opened only where a linked case's standing is
/// actually being reported, so the office layer is exercisable on its own.
fn run_office(
    evidence_database: &Path,
    office_database: &Path,
    action: &OfficeAction,
) -> Result<()> {
    let mut desk = OfficeDesk::open(office_database)?;
    match action {
        OfficeAction::Init => {
            println!("initialized {}", office_database.display());
        }
        OfficeAction::Seed { fixture, anchor } => {
            let OfficeFixtureName::MisdemeanorDocket = fixture;
            let anchor = match anchor {
                Some(given) => given.clone(),
                None => desk.today()?,
            };
            let seeded = OfficeFixture::MisdemeanorDocket.seed_from(desk.store_mut(), &anchor)?;
            println!("{seeded}");
        }
        OfficeAction::User { action } => run_office_user(&desk, action)?,
        OfficeAction::Client { action } => run_office_client(&mut desk, action)?,
        OfficeAction::Matter { action } => {
            run_office_matter(&mut desk, evidence_database, action)?;
        }
        OfficeAction::Assign {
            matter,
            user,
            role,
            by,
        } => {
            let id = desk.store().assign_to_matter(&ProposedAssignment {
                matter_id: matter.clone(),
                user_id: user.clone(),
                role: (*role).into(),
                assigned_by_user_id: by.clone(),
            })?;
            println!("{id}");
        }
        OfficeAction::Court { action } => run_office_court(&desk, action)?,
        OfficeAction::Appearance { action } => run_office_appearance(&mut desk, action)?,
        OfficeAction::Deadline { action } => run_office_deadline(&desk, action)?,
        OfficeAction::Note { action } => run_office_note(&mut desk, action)?,
        OfficeAction::Identity {
            action:
                IdentityAction::Decide {
                    client,
                    evidence_case,
                    entity,
                    state,
                    matched_on,
                    author,
                },
        } => {
            desk.store()
                .decide_identity_link(&ProposedIdentityLinkDecision {
                    client_id: client.clone(),
                    evidence_case_id: evidence_case.clone(),
                    evidence_entity_id: entity.clone(),
                    state: (*state).into(),
                    matched_on: matched_on.clone(),
                    author_user_id: author.clone(),
                })?;
            println!("{}", state_word(*state));
        }
        OfficeAction::Search { query, limit } => {
            print_json(&desk.store().search(query, *limit)?)?;
        }
        OfficeAction::Docket { date, week } => {
            let date = office_date(&desk, date)?;
            let evidence = Store::open(evidence_database)?;
            if *week {
                print_json(&desk.court_week(&evidence, &date)?)?;
            } else {
                print_json(&desk.court_docket(&evidence, &date)?)?;
            }
        }
        OfficeAction::Deadlines { as_of, within } => {
            let as_of = office_date(&desk, as_of)?;
            print_json(&desk.upcoming_deadlines(&as_of, *within)?)?;
        }
    }
    Ok(())
}

/// Resolves the literal `today` against the office's one local clock.
fn office_date(desk: &OfficeDesk, given: &str) -> Result<String> {
    if given.eq_ignore_ascii_case("today") {
        desk.today()
    } else {
        Ok(given.to_owned())
    }
}

/// The word a decision is reported by, so a script can read the outcome back.
const fn state_word(state: IdentityStateArg) -> &'static str {
    match state {
        IdentityStateArg::Linked => "linked",
        IdentityStateArg::Dismissed => "dismissed",
    }
}

fn run_office_user(desk: &OfficeDesk, action: &UserAction) -> Result<()> {
    match action {
        UserAction::Add {
            name,
            role,
            email,
            id,
        } => {
            let created = desk.store().create_user(&ProposedUser {
                id: id.clone(),
                display_name: name.clone(),
                role: role.clone(),
                email: email.clone(),
            })?;
            println!("{created}");
        }
        UserAction::List => {
            let rows = desk
                .store()
                .users()?
                .into_iter()
                .map(|(id, name, role)| OfficeRow {
                    id,
                    name,
                    role: Some(role),
                })
                .collect::<Vec<_>>();
            print_json(&rows)?;
        }
    }
    Ok(())
}

fn run_office_client(desk: &mut OfficeDesk, action: &ClientAction) -> Result<()> {
    match action {
        ClientAction::Add {
            name,
            dob,
            sex,
            language,
            notes,
            aliases,
            phones,
            emails,
            addresses,
            author,
            id,
        } => {
            let mut contacts = Vec::new();
            push_contacts(&mut contacts, ContactKind::Phone, phones);
            push_contacts(&mut contacts, ContactKind::Email, emails);
            push_contacts(&mut contacts, ContactKind::Address, addresses);
            let profile = desk.store_mut().create_client(&ProposedClient {
                id: id.clone(),
                display_name: name.clone(),
                date_of_birth: dob.clone(),
                sex: sex.map(Sex::from),
                preferred_language: language.clone(),
                notes: notes.clone(),
                aliases: aliases.clone(),
                contacts,
                author_user_id: author.clone(),
            })?;
            print_json(&profile)?;
        }
        ClientAction::List => {
            let rows = desk
                .store()
                .clients()?
                .into_iter()
                .map(|(id, name)| OfficeRow {
                    id,
                    name,
                    role: None,
                })
                .collect::<Vec<_>>();
            print_json(&rows)?;
        }
        ClientAction::Show { client_id } => {
            print_json(&desk.store().client_profile(client_id)?)?;
        }
        ClientAction::Duplicates {
            name,
            contacts,
            exclude,
        } => {
            let candidates =
                desk.store()
                    .possible_client_duplicates(name, contacts, exclude.as_deref())?;
            print_json(&candidates)?;
        }
    }
    Ok(())
}

/// Turns repeated command-line values into contacts, the first of each kind
/// being the one to try first.
fn push_contacts(contacts: &mut Vec<ProposedClientContact>, kind: ContactKind, values: &[String]) {
    for (index, value) in values.iter().enumerate() {
        contacts.push(ProposedClientContact {
            kind,
            value: value.clone(),
            label: None,
            is_primary: index == 0,
        });
    }
}

fn run_office_matter(
    desk: &mut OfficeDesk,
    evidence_database: &Path,
    action: &MatterAction,
) -> Result<()> {
    match action {
        MatterAction::Open {
            client,
            caption,
            court_number,
            court,
            status,
            custody,
            offer,
            offer_summary,
            charges,
            opened,
            last_contact,
            evidence_case,
            author,
            id,
        } => {
            let created = desk.store().open_matter(&ProposedMatter {
                id: id.clone(),
                client_id: client.clone(),
                caption: caption.clone(),
                court_number: court_number.clone(),
                court_id: court.clone(),
                status: status.map(Into::into),
                custody_state: custody.map(Into::into),
                offer_state: offer.map(Into::into),
                offer_summary: offer_summary.clone(),
                charge_summary: charges.clone(),
                opened_on: opened.clone(),
                last_contact_on: last_contact.clone(),
                evidence_case_id: evidence_case.clone(),
                author_user_id: author.clone(),
            })?;
            println!("{created}");
        }
        MatterAction::List => print_json(&desk.store().matters()?)?,
        MatterAction::Show { matter_id } => {
            let evidence = Store::open(evidence_database)?;
            print_json(&desk.matter_view(&evidence, matter_id)?)?;
        }
        MatterAction::Status { matter_id, status } => {
            desk.store()
                .update_matter_status(matter_id, (*status).into())?;
        }
        MatterAction::Custody { matter_id, custody } => {
            desk.store()
                .update_custody_state(matter_id, (*custody).into())?;
        }
        MatterAction::Offer {
            matter_id,
            offer,
            summary,
        } => {
            desk.store()
                .update_offer_state(matter_id, (*offer).into(), summary.as_deref())?;
        }
        MatterAction::Contact { matter_id, on } => {
            let on = office_date(desk, on)?;
            desk.store().record_client_contact(matter_id, &on)?;
        }
        MatterAction::LinkEvidence {
            matter_id,
            evidence_case,
        } => {
            desk.store().link_evidence_case(matter_id, evidence_case)?;
        }
        MatterAction::Relate {
            matter_id,
            related,
            relation,
        } => {
            let id = desk.store().link_matters(matter_id, related, relation)?;
            println!("{id}");
        }
    }
    Ok(())
}

fn run_office_court(desk: &OfficeDesk, action: &CourtAction) -> Result<()> {
    match action {
        CourtAction::Add {
            name,
            division,
            address,
            room,
            id,
        } => {
            let created = desk.store().create_court(&ProposedCourt {
                id: id.clone(),
                name: name.clone(),
                division: division.clone(),
                address: address.clone(),
                room: room.clone(),
            })?;
            println!("{created}");
        }
        CourtAction::List => {
            let rows = desk
                .store()
                .courts()?
                .into_iter()
                .map(|(id, name)| OfficeRow {
                    id,
                    name,
                    role: None,
                })
                .collect::<Vec<_>>();
            print_json(&rows)?;
        }
        CourtAction::Judge { name, court } => {
            let created = desk.store().create_judge(court.as_deref(), name)?;
            println!("{created}");
        }
    }
    Ok(())
}

fn run_office_appearance(desk: &mut OfficeDesk, action: &AppearanceAction) -> Result<()> {
    match action {
        AppearanceAction::Schedule {
            client,
            matters,
            court,
            judge,
            date,
            time,
            appearance_type,
            notes,
            author,
            id,
        } => {
            let created = desk.store_mut().schedule_appearance(&ProposedAppearance {
                id: id.clone(),
                client_id: client.clone(),
                matter_ids: matters.clone(),
                court_id: court.clone(),
                judge_id: judge.clone(),
                appearance_date: date.clone(),
                appearance_time: time.clone(),
                appearance_type: (*appearance_type).into(),
                notes: notes.clone(),
                author_user_id: author.clone(),
            })?;
            println!("{created}");
        }
        AppearanceAction::Link {
            appearance_id,
            matter,
            note,
        } => {
            desk.store()
                .link_matter_to_appearance(appearance_id, matter, note.as_deref())?;
        }
        AppearanceAction::Unlink {
            appearance_id,
            matter,
        } => {
            desk.store()
                .unlink_matter_from_appearance(appearance_id, matter)?;
        }
        AppearanceAction::Cancel { appearance_id, on } => {
            let on = office_date(desk, on)?;
            desk.store().cancel_appearance(appearance_id, &on)?;
        }
        AppearanceAction::Outcome {
            appearance_id,
            outcome,
        } => {
            desk.store()
                .record_appearance_outcome(appearance_id, outcome)?;
        }
    }
    Ok(())
}

fn run_office_deadline(desk: &OfficeDesk, action: &DeadlineAction) -> Result<()> {
    match action {
        DeadlineAction::Add {
            matter,
            description,
            due,
            origin,
            author,
            id,
        } => {
            let created = desk.store().record_deadline(&ProposedDeadline {
                id: id.clone(),
                matter_id: matter.clone(),
                description: description.clone(),
                due_date: due.clone(),
                origin: (*origin).into(),
                author_user_id: author.clone(),
            })?;
            println!("{created}");
        }
        DeadlineAction::Satisfy { deadline_id, on } => {
            let on = office_date(desk, on)?;
            desk.store().satisfy_deadline(deadline_id, &on)?;
        }
    }
    Ok(())
}

fn run_office_note(desk: &mut OfficeDesk, action: &NoteAction) -> Result<()> {
    match action {
        NoteAction::Write {
            client,
            matter,
            appearance,
            body,
            author,
            id,
        } => {
            let written = desk.store_mut().write_note(&ProposedNote {
                id: id.clone(),
                client_id: client.clone(),
                matter_id: matter.clone(),
                appearance_id: appearance.clone(),
                body: body.clone(),
                author_user_id: author.clone(),
            })?;
            print_json(&written)?;
        }
        NoteAction::Revise {
            note_id,
            body,
            author,
        } => {
            let revised = desk.store_mut().revise_note(
                note_id,
                &ProposedNote {
                    id: None,
                    client_id: None,
                    matter_id: None,
                    appearance_id: None,
                    body: body.clone(),
                    author_user_id: author.clone(),
                },
            )?;
            print_json(&revised)?;
        }
        NoteAction::List { scope, subject } => {
            print_json(&desk.store().notes_for((*scope).into(), subject)?)?;
        }
        NoteAction::History { note_id } => {
            print_json(&desk.store().note_history(note_id)?)?;
        }
        NoteAction::Mentioning { tag } => {
            print_json(&desk.store().notes_mentioning((*tag).into())?)?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        // `evidence export … | head` closes the pipe partway through, which is
        // a reader that has seen enough rather than a failure to report.
        if reader_stopped_listening(&error) {
            return;
        }
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

/// Returns whether the command failed only because its output had nowhere to go.
fn reader_stopped_listening(error: &evidence_intake::Error) -> bool {
    matches!(
        error,
        evidence_intake::Error::Serialization(failure)
            if failure.io_error_kind() == Some(io::ErrorKind::BrokenPipe)
    )
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    // The office layer is a sibling of the kernel, so an office command is
    // dispatched before the evidence database is opened at all. A defender who
    // has clients and a calendar but no discovery yet still has a docket.
    if let Command::Office { action } = &cli.command {
        let office_database = cli
            .office_database
            .clone()
            .unwrap_or_else(|| OfficeDesk::beside(&cli.database));
        return run_office(&cli.database, &office_database, action);
    }

    let mut store = Store::open(&cli.database)?;

    match cli.command {
        // Handled above, before the evidence database was opened.
        Command::Office { .. } => {}
        Command::Init => {
            println!("initialized {}", cli.database.display());
        }
        Command::Seed { fixture } => {
            let fixture = match fixture {
                FixtureName::VehicleStop => DemoFixture::VehicleStop,
                FixtureName::HitAndRun => DemoFixture::HitAndRun,
            };
            let id = fixture.seed(&mut store)?;
            println!("{id}");
        }
        Command::Cases => print_json(&store.cases()?)?,
        Command::NewCase {
            name,
            id,
            reference,
            jurisdiction,
            production,
        } => {
            let opened = store.open_case(&ProposedCase {
                id,
                name,
                reference,
                jurisdiction,
                production,
            })?;
            print_json(&opened)?;
        }
        Command::NewProduction {
            case_id,
            label,
            id,
            received,
            from,
            notes,
        } => {
            let opened = store.open_production(
                &CaseId(case_id),
                &ProposedProduction {
                    id,
                    label,
                    received_at: received,
                    producing_party: from,
                    notes,
                },
            )?;
            print_json(&opened)?;
        }
        Command::Productions { case_id } => {
            print_json(&store.productions(&CaseId(case_id))?)?;
        }
        Command::View { case_id, view } => {
            let case_id = CaseId(case_id);
            match view {
                View::Overview => print_json(&store.overview(&case_id)?)?,
                View::Standing => print_json(&store.case_standing(&case_id)?)?,
                View::Discovery => print_json(&store.discovery_ledger(&case_id)?)?,
                View::Elements => print_json(&store.element_matrix(&case_id)?)?,
                View::Witness { entity_id } => {
                    print_json(&store.witness_dossier(&case_id, &entity_id)?)?;
                }
                View::Timeline => print_json(&store.contested_timeline(&case_id)?)?,
                View::Collation => print_json(&store.collation_index(&case_id)?)?,
                View::Issues => print_json(&store.issue_workspaces(&case_id)?)?,
                View::Brief { posture } => {
                    print_json(&store.decision_brief(&case_id, &posture)?)?;
                }
                View::Proposition { proposition_id } => {
                    print_json(&store.proposition_evidence(&case_id, &proposition_id)?)?;
                }
                View::Offenses => print_json(&store.offense_comparison(&case_id)?)?,
                View::Notes {
                    target_kind,
                    target,
                } => {
                    let target = NodeRef::new(NodeKind::from(target_kind), target);
                    print_json(&store.annotations(&case_id, &target)?)?;
                }
                View::WorkHistory { item_id } => {
                    print_json(&store.advocacy_history(&case_id, &item_id)?)?;
                }
            }
        }
        Command::Review { case_id, action } => {
            let case_id = CaseId(case_id);
            match action {
                ReviewAction::Queue => print_json(&store.review_queue(&case_id)?)?,
                ReviewAction::Apply {
                    target,
                    id,
                    state,
                    actor,
                    basis,
                    locator,
                } => {
                    let decision = ReviewDecision {
                        target: ReviewTarget::from(target),
                        target_id: id,
                        to_state: ReviewState::from(state),
                        actor,
                        basis,
                        verified_against_locator: locator,
                    };
                    print_json(&store.apply_review(&case_id, &decision)?)?;
                }
                ReviewAction::History { id } => {
                    print_json(&store.review_history(&case_id, id.as_deref())?)?;
                }
            }
        }
        Command::Suggest { case_id, analyzers } => {
            let case_id = CaseId(case_id);
            let kinds: Vec<SuggestionKind> = if analyzers.is_empty() {
                SuggestionKind::ALL.to_vec()
            } else {
                analyzers.into_iter().map(SuggestionKind::from).collect()
            };
            print_json(&store.suggest(&case_id, &kinds)?)?;
        }
        Command::Search {
            case_id,
            query,
            limit,
        } => {
            let case_id = CaseId(case_id);
            print_json(&store.search(&case_id, &query, limit)?)?;
        }
        Command::Export { case_id, audience } => {
            let case_id = CaseId(case_id);
            print_json(&store.export_case(&case_id, ExportAudience::from(audience))?)?;
        }
        Command::Author { case_id, item } => {
            let case_id = CaseId(case_id);
            match item {
                AuthorItem::Proposition { text, author, id } => {
                    let proposal = ProposedProposition { id, text, author };
                    print_json(&store.author_proposition(&case_id, &proposal)?)?;
                }
                AuthorItem::Link {
                    from_kind,
                    from,
                    relation,
                    to_kind,
                    to,
                    rationale,
                    author,
                    id,
                } => {
                    let proposal = ProposedLink {
                        id,
                        from: NodeRef::new(NodeKind::from(from_kind), from),
                        relation: EdgeKind::from(relation),
                        to: NodeRef::new(NodeKind::from(to_kind), to),
                        rationale,
                        author,
                    };
                    print_json(&store.link_evidence(&case_id, &proposal)?)?;
                }
                AuthorItem::Entity {
                    kind,
                    name,
                    client,
                    notes,
                    id,
                } => {
                    let proposal = ProposedEntity {
                        id,
                        kind: EntityKind::from(kind),
                        display_name: name,
                        is_client: client,
                        notes,
                    };
                    print_json(&store.record_entity(&case_id, &proposal)?)?;
                }
                AuthorItem::Charge {
                    label,
                    elements,
                    posture,
                    citation,
                    grade,
                    id,
                } => {
                    let proposal = ProposedCharge {
                        id,
                        label,
                        citation,
                        posture: ChargePosture::from(posture),
                        grade,
                        elements: elements
                            .into_iter()
                            .map(|text| ProposedElement { id: None, text })
                            .collect(),
                    };
                    print_json(&store.record_charge(&case_id, &proposal)?)?;
                }
                AuthorItem::Work {
                    kind,
                    title,
                    body,
                    status,
                    author,
                    revises,
                    id,
                } => {
                    let proposal = ProposedAdvocacyItem {
                        id,
                        kind: AdvocacyKind::from(kind),
                        title,
                        body,
                        status,
                        author,
                    };
                    let written = match revises {
                        Some(previous) => {
                            store.revise_advocacy_item(&case_id, &previous, &proposal)?
                        }
                        None => store.author_advocacy_item(&case_id, &proposal)?,
                    };
                    print_json(&written)?;
                }
                AuthorItem::Note {
                    target_kind,
                    target,
                    body,
                    author,
                    revises,
                    id,
                } => {
                    let proposal = ProposedAnnotation {
                        id,
                        target: NodeRef::new(NodeKind::from(target_kind), target),
                        body,
                        author,
                    };
                    let written = match revises {
                        Some(previous) => {
                            store.revise_annotation(&case_id, &previous, &proposal)?
                        }
                        None => store.annotate(&case_id, &proposal)?,
                    };
                    print_json(&written)?;
                }
                AuthorItem::Brief {
                    posture,
                    summary,
                    strengths,
                    risks,
                    unresolved,
                    client_topics,
                    author,
                    id,
                } => {
                    let proposal = ProposedBrief {
                        id,
                        posture,
                        summary,
                        strengths,
                        risks,
                        unresolved_questions: unresolved,
                        client_topics,
                        author,
                    };
                    print_json(&store.record_brief(&case_id, &proposal)?)?;
                }
                AuthorItem::Mapping {
                    element,
                    proposition,
                    assessment,
                    notes,
                    author,
                    id,
                } => {
                    let proposal = ProposedElementMapping {
                        id,
                        element_id: element,
                        proposition_id: proposition,
                        assessment: ElementAssessment::from(assessment),
                        notes,
                        author,
                    };
                    print_json(&store.map_element(&case_id, &proposal)?)?;
                }
            }
        }
        Command::Ingest { case_id, path } => {
            let json = read_batch_json(&path)?;
            let batch: NormalizedBatch = serde_json::from_str(&json)?;
            if batch.case_id.0 != case_id {
                return Err(evidence_intake::Error::InvalidFixture(format!(
                    "batch is for case `{}`, not `{case_id}`",
                    batch.case_id
                )));
            }
            store.import_normalized(&batch)?;
            print_json(&store.overview(&batch.case_id)?)?;
        }
        Command::IndexFrames { case_id, path } => {
            let json = read_batch_json(&path)?;
            let index: KeyframeIndex = serde_json::from_str(&json)?;
            if index.case_id.0 != case_id {
                return Err(evidence_intake::Error::InvalidFixture(format!(
                    "index is for case `{}`, not `{case_id}`",
                    index.case_id
                )));
            }
            store.index_keyframes(&index)?;
            print_json(&store.overview(&index.case_id)?)?;
        }
        Command::FindFrames {
            case_id,
            model,
            vector,
            limit,
        } => {
            let json = if vector == "-" {
                read_batch_json(std::path::Path::new("-"))?
            } else if let Some(path) = vector.strip_prefix('@') {
                read_batch_json(std::path::Path::new(path))?
            } else {
                vector
            };
            let query: Vec<f32> = serde_json::from_str(&json)?;
            print_json(&store.search_keyframes(&CaseId(case_id), &model, &query, limit)?)?;
        }
    }
    Ok(())
}

/// Reads a normalized batch from a file or from stdin when `path` is `-`.
fn read_batch_json(path: &std::path::Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut json = String::new();
        io::stdin().read_to_string(&mut json).map_err(|error| {
            evidence_intake::Error::InvalidFixture(format!("could not read stdin: {error}"))
        })?;
        return Ok(json);
    }
    fs::read_to_string(path).map_err(|error| {
        evidence_intake::Error::InvalidFixture(format!(
            "could not read {}: {error}",
            path.display()
        ))
    })
}

/// Writes a read model to standard output as pretty JSON.
///
/// Serialized straight into a buffered writer rather than into a `String` that
/// is then printed: a full case export is large, and rendering it twice in
/// memory to gain nothing is the kind of cost that only shows up on the cases
/// that matter most.
fn print_json(value: &impl Serialize) -> Result<()> {
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    serde_json::to_writer_pretty(&mut out, value)?;
    // A failed write reaches the caller the same way it would have if it had
    // happened one byte earlier, inside the serializer.
    out.write_all(b"\n").map_err(serde_json::Error::io)?;
    out.flush().map_err(serde_json::Error::io)?;
    Ok(())
}
