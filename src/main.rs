//! Command-line shell for the collation kernel.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use evidence_intake::{
    CaseId, DemoFixture, EdgeKind, NodeKind, NodeRef, ProposedLink, ProposedProposition, Result,
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
            }
        }
    }
    Ok(())
}

fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
