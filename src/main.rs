//! Command-line shell for the collation kernel.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_intake::{
    AdvocacyKind, CaseId, ChargePosture, DemoFixture, EdgeKind, ElementAssessment, NodeKind,
    NodeRef, ProposedAdvocacyItem, ProposedAnnotation, ProposedBrief, ProposedCharge,
    ProposedElement, ProposedElementMapping, ProposedLink, ProposedProposition, Result,
    ReviewDecision, ReviewState, ReviewTarget, Store,
};
use serde::Serialize;

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    /// SQLite case database.
    #[arg(long, default_value = "evidence.sqlite", global = true)]
    database: PathBuf,
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
    /// List cases.
    Cases,
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
    /// Write a person's own reading of the case into it.
    Author {
        /// Stable case identifier.
        case_id: String,
        #[command(subcommand)]
        item: AuthorItem,
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
    Corroborates,
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
}

impl From<RelationArg> for EdgeKind {
    fn from(value: RelationArg) -> Self {
        match value {
            RelationArg::Supports => Self::Supports,
            RelationArg::Contradicts => Self::Contradicts,
            RelationArg::Corroborates => Self::Corroborates,
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

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut store = Store::open(&cli.database)?;

    match cli.command {
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
        Command::View { case_id, view } => {
            let case_id = CaseId(case_id);
            match view {
                View::Overview => print_json(&store.overview(&case_id)?)?,
                View::Discovery => print_json(&store.discovery_ledger(&case_id)?)?,
                View::Elements => print_json(&store.element_matrix(&case_id)?)?,
                View::Witness { entity_id } => {
                    print_json(&store.witness_dossier(&case_id, &entity_id)?)?;
                }
                View::Timeline => print_json(&store.contested_timeline(&case_id)?)?,
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
    }
    Ok(())
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
