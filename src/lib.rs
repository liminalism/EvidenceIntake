//! Defender-oriented evidence collation.
//!
//! The kernel preserves disagreement rather than calculating a single account
//! of events. Every evidentiary claim remains traceable to an immutable source
//! segment, and attorney work product occupies a distinct advocacy layer.

mod authoring;
mod error;
mod fixture;
mod ingest;
mod model;
mod review;
mod store;
mod views;

pub use authoring::{AuthoredLink, AuthoredProposition, ProposedLink, ProposedProposition};
pub use error::{Error, Result};
pub use fixture::DemoFixture;
pub use ingest::{
    ExtractionProvenance, NormalizedBatch, NormalizedContent, NormalizedSegment, NormalizedSource,
};
pub use model::{
    AdvocacyKind, CaseId, ContentKind, EdgeKind, NodeKind, NodeRef, ReviewState, SourceKind,
    TemporalRelation, TimelineLane,
};
pub use review::{ReviewDecision, ReviewEvent, ReviewQueueItem, ReviewTarget};
pub use store::Store;
pub use views::{
    DecisionBrief, DiscoveryItem, ElementCoverage, ElementRow, IssueWorkspace, OffenseComparison,
    Overview, PropositionEvidence, TimelineEntry, WitnessStatement,
};
