//! Defender-oriented evidence collation.
//!
//! The kernel preserves disagreement rather than calculating a single account
//! of events. Every evidentiary claim remains traceable to an immutable source
//! segment, and attorney work product occupies a distinct advocacy layer.

mod authoring;
mod error;
mod export;
mod fixture;
pub mod gui;
mod ingest;
mod model;
mod review;
mod store;
mod suggest;
mod views;

pub use authoring::{
    AuthoredCharge, AuthoredElement, AuthoredElementMapping, AuthoredEntity, AuthoredLink,
    AuthoredProposition, OpenedCase, OpenedProduction, ProposedAdvocacyItem, ProposedAnnotation,
    ProposedBrief, ProposedCase, ProposedCharge, ProposedElement, ProposedElementMapping,
    ProposedEntity, ProposedLink, ProposedProduction, ProposedProposition, WorkProductVersion,
};
pub use error::{Error, Result};
pub use export::{
    CaseExport, ExportAudience, ExportedProposition, ExportedWorkProduct, UnsupportedProposition,
};
pub use fixture::DemoFixture;
pub use ingest::{
    ExtractionProvenance, IndexedKeyframe, KeyframeIndex, NormalizedBatch, NormalizedContent,
    NormalizedEdge, NormalizedSegment, NormalizedSource,
};
pub use model::{
    AdvocacyKind, CaseId, ChargePosture, ContentKind, EdgeKind, ElementAssessment, EntityKind,
    NodeKind, NodeRef, ReviewState, SourceKind, TemporalRelation, TimelineLane,
};
pub use review::{ReviewDecision, ReviewEvent, ReviewQueueItem, ReviewTarget};
pub use store::Store;
pub use suggest::{AnalyzerReport, Finding, SuggestionKind, SuggestionRun};
pub use views::{
    CaseStanding, CaseSummary, ChargeStanding, DecisionBrief, DiscoveryItem, ElementCoverage,
    ElementRow, ElementStanding, IssueWorkspace, KEYFRAME_SIMILARITY_CUT, KeyframeHit, LiveDispute,
    LoadBearingSource, OffenseComparison, OpenGap, Overview, PropositionEvidence, SearchHit,
    TimelineEntry, WitnessStatement,
};
