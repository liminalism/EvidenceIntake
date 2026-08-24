//! Defender-oriented evidence collation.
//!
//! The kernel preserves disagreement rather than calculating a single account
//! of events. Every evidentiary claim remains traceable to an immutable source
//! segment, and attorney work product occupies a distinct advocacy layer.

mod assembly;
mod authoring;
mod coordinator;
mod enrichment;
mod error;
mod export;
mod fixture;
pub mod gui;
mod ingest;
mod intake;
mod interpretation;
mod model;
mod review;
mod store;
mod suggest;
mod views;

pub use assembly::{
    AuthoredOccurrence, CaseDigest, DigestLocator, DigestSection, DigestSentence, Lineage,
    OmittedDigestTemplate, PacketItem, PacketSection, ProposedOccurrence, PropositionPacket,
    StructuralCount,
};
pub use authoring::{
    AuthoredCharge, AuthoredElement, AuthoredElementMapping, AuthoredEntity, AuthoredLink,
    AuthoredProposition, BriefParagraphKind, OpenedCase, OpenedProduction, ProposedAdvocacyItem,
    ProposedAnnotation, ProposedBrief, ProposedBriefParagraph, ProposedCase, ProposedCharge,
    ProposedElement, ProposedElementMapping, ProposedEntity, ProposedLink, ProposedProduction,
    ProposedProposition, WorkProductVersion,
};
pub use coordinator::IntakeCoordinator;
pub use enrichment::{
    EnrichmentField, EnrichmentPassage, EnrichmentSession, EnrichmentSource, EnrichmentValue,
    EntityCandidate, PendingInput, PreviewDescriptor, TimeEntry,
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
pub use intake::{IntakeArtifact, IntakeJob, IntakeJobState, NewIntakeJob, SourceLocation};
pub use interpretation::{
    ContentForm, ContentInterpretation, EffectiveInterpretation, InterpretationBatch,
    InterpretationTarget, Materiality, PerceptionBasis, ProposedContentGroup,
    ProposedInterpretation, ProposedSourceProfile, SourceProfile, SourceRole, TemporalStance,
};
pub use model::{
    AdvocacyKind, CaseId, ChargePosture, ContentKind, EdgeKind, ElementAssessment, EntityKind,
    NodeKind, NodeRef, ReviewState, SourceKind, TemporalRelation, TimelineLane,
};
pub use review::{ReviewDecision, ReviewEvent, ReviewQueueItem, ReviewTarget};
pub use store::Store;
pub use suggest::{AnalyzerReport, Finding, SuggestionKind, SuggestionRun};
pub use views::{
    CaseStanding, CaseSummary, ChargeStanding, CollationEntry, CollationGroup, CollationIndex,
    DecisionBrief, DiscoveryItem, ElementCoverage, ElementRow, ElementStanding, IssueWorkspace,
    KeyframeHit, LiveDispute, LoadBearingSource, OffenseComparison, OpenGap, Overview,
    PlacementGap, PropositionEvidence, SearchHit, SourceAnchorCoverage, TimelineEntry,
    WitnessStatement,
};
