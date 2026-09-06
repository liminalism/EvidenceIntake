//! Platform-neutral application layer for native evidence workspaces.
//!
//! Frontends own widgets and event loops; this module owns case selection and
//! translates user operations into the kernel's typed API. A Windows frontend
//! is available behind `gui-winsafe`; a future Linux frontend can call the same
//! [`Workspace`] methods without depending on Win32.

use std::fmt::{self, Write as _};
use std::fs;
use std::path::{Path, PathBuf};

use evidence_adapter_protocol::{AdapterJobRequest, AdapterResultManifest};
use office_core::{
    AppearanceType, COMMON_LANGUAGES, CivilDate, ClientProfile, ContactKind, CustodyState,
    DeadlineOrigin, DeadlineRow, MatterProfile, MatterStatus, NoteScope, OfferState, OfficeFixture,
    PossiblePerson, ProposedAppearance, ProposedClient, ProposedClientContact, ProposedCourt,
    ProposedDeadline, ProposedMatter, ProposedNote, Sex, UpcomingDeadlines,
};
use serde::Serialize;

use crate::{
    AuthoredEntity, CaseDigest, CaseId, CaseStanding, CollationEntry, CollationIndex, ContentForm,
    ContentInterpretation, CourtDocket, DemoFixture, DocketRow, EnrichmentField, EnrichmentPassage,
    EnrichmentSession, EnrichmentSource, EnrichmentValue, EntityCandidate, EntityKind,
    ExportAudience, IntakeArtifact, IntakeJob, InterpretationBatch, InterpretationTarget,
    KeyframeHit, Materiality, NewIntakeJob, NodeKind, NodeRef, NormalizedBatch, OfficeDesk,
    OpenedProduction, PendingInput, PerceptionBasis, PreviewDescriptor, ProposedAdvocacyItem,
    ProposedAnnotation, ProposedBrief, ProposedCase, ProposedCharge, ProposedContentGroup,
    ProposedElementMapping, ProposedEntity, ProposedInterpretation, ProposedLink,
    ProposedOccurrence, ProposedProduction, ProposedProposition, ProposedSourceProfile,
    PropositionPacket, ReviewDecision, ReviewState, ReviewTarget, SourceLocation, SourceProfile,
    Store, SuggestionKind, TemporalStance, TimeEntry, TimelineEntry, WitnessStatement,
};

#[cfg(all(feature = "gui-winsafe", target_os = "windows"))]
pub mod winsafe;

/// Result returned by the frontend-neutral application layer.
pub type GuiResult<T> = std::result::Result<T, GuiError>;

/// A user-facing GUI operation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuiError {
    message: String,
}

impl GuiError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for GuiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for GuiError {}

impl From<crate::Error> for GuiError {
    fn from(error: crate::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<office_core::Error> for GuiError {
    fn from(error: office_core::Error) -> Self {
        Self::new(error.to_string())
    }
}

impl From<serde_json::Error> for GuiError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON error: {error}"))
    }
}

impl From<std::io::Error> for GuiError {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("file error: {error}"))
    }
}

/// Case read models available from every native frontend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceView {
    /// Counts and workflow state.
    Overview,
    /// Element-by-element structural standing.
    Standing,
    /// Production and completeness ledger.
    Discovery,
    /// Charge-element evidence matrix.
    Elements,
    /// Lane-preserving contested timeline.
    Timeline,
    /// Source-grounded time and location collation index.
    Collation,
    /// Privileged issue workspaces.
    Issues,
    /// Charged and alternative offense comparison.
    Offenses,
    /// Records waiting for a named reviewer.
    ReviewQueue,
    /// Append-only review decisions.
    ReviewHistory,
    /// Fixed source-linked workspace around each contested proposition.
    Packets,
    /// Deterministic, template-rendered, source-linked layered digest.
    Digest,
}

impl WorkspaceView {
    /// Every view, in the order a case is usually read.
    pub const ALL: [Self; 12] = [
        Self::Overview,
        Self::Standing,
        Self::Discovery,
        Self::Elements,
        Self::Timeline,
        Self::Collation,
        Self::Issues,
        Self::Offenses,
        Self::ReviewQueue,
        Self::ReviewHistory,
        Self::Packets,
        Self::Digest,
    ];

    /// Stable label used by native navigation controls.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Standing => "Case standing",
            Self::Discovery => "Discovery ledger",
            Self::Elements => "Element matrix",
            Self::Timeline => "Contested timeline",
            Self::Collation => "Time & Place Index",
            Self::Issues => "Issue workspaces",
            Self::Offenses => "Offense comparison",
            Self::ReviewQueue => "Review queue",
            Self::ReviewHistory => "Review history",
            Self::Packets => "Proposition packets",
            Self::Digest => "Case digest",
        }
    }
}

/// Typed authoring operations exposed by the compact JSON authoring panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    /// Contested factual proposition.
    Proposition,
    /// Typed relationship between two case nodes.
    Link,
    /// Person, organization, object, or location.
    Entity,
    /// Charge with ordered statutory elements.
    Charge,
    /// Privileged advocacy/work-product item.
    WorkProduct,
    /// Privileged annotation on a case node.
    Note,
    /// Posture-specific client decision brief.
    Brief,
    /// Proposition-to-element assessment.
    ElementMapping,
    /// Accounts of one occurrence, assembled into a single event.
    Occurrence,
}

impl AuthorKind {
    /// All authoring kinds in the order shown by native frontends.
    pub const ALL: [Self; 9] = [
        Self::Proposition,
        Self::Link,
        Self::Entity,
        Self::Charge,
        Self::WorkProduct,
        Self::Note,
        Self::Brief,
        Self::ElementMapping,
        Self::Occurrence,
    ];

    /// Stable label used by the authoring-kind selector.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Proposition => "proposition",
            Self::Link => "link",
            Self::Entity => "entity",
            Self::Charge => "charge",
            Self::WorkProduct => "work product",
            Self::Note => "note",
            Self::Brief => "brief",
            Self::ElementMapping => "element mapping",
            Self::Occurrence => "occurrence",
        }
    }

    /// Finds a kind from its stable frontend label.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }

    /// Returns an editable JSON template for this operation.
    pub fn template(self) -> &'static str {
        match self {
            Self::Proposition => {
                r#"{
  "text": "State the contested proposition",
  "author": "Reviewer name"
}"#
            }
            Self::Link => {
                r#"{
  "from": { "kind": "content", "id": "content-id" },
  "relation": "supports",
  "to": { "kind": "proposition", "id": "proposition-id" },
  "rationale": "Why this relationship holds",
  "author": "Reviewer name"
}"#
            }
            Self::Entity => {
                r#"{
  "kind": "person",
  "display_name": "Full name",
  "is_client": false,
  "notes": null
}"#
            }
            Self::Charge => {
                r#"{
  "label": "Offense label",
  "citation": "citation",
  "posture": "charged",
  "grade": null,
  "elements": [
    { "text": "First statutory element" }
  ]
}"#
            }
            Self::WorkProduct => {
                r#"{
  "kind": "legal_issue",
  "title": "Issue title",
  "body": "Privileged analysis",
  "status": "open",
  "author": "Attorney name"
}"#
            }
            Self::Note => {
                r#"{
  "target": { "kind": "content", "id": "content-id" },
  "body": "Privileged note",
  "author": "Attorney name"
}"#
            }
            Self::Brief => {
                r#"{
  "posture": "motions",
  "summary": "Summary",
  "strengths": "Structural support to discuss",
  "risks": "Material risks",
  "unresolved_questions": "Open questions",
  "client_topics": "Topics for the client",
  "author": "Attorney name"
}"#
            }
            Self::ElementMapping => {
                r#"{
  "element_id": "element-id",
  "proposition_id": "proposition-id",
  "assessment": "uncertain",
  "notes": "Why it bears this way",
  "author": "Attorney name"
}"#
            }
            Self::Occurrence => {
                r#"{
  "label": "What happened, neutrally",
  "lane": "recorded",
  "raw_time": null,
  "normalized_start": null,
  "time_basis": null,
  "location_text": null,
  "passage_ids": ["content-id-one", "content-id-two"],
  "rationale": "Why these accounts describe one occurrence",
  "author": "Reviewer name"
}"#
            }
        }
    }
}

/// Platform-neutral state and operations for one database-backed workspace.
///
/// A workspace holds both halves of the product: the evidence kernel, and the
/// office beside it. They never share a transaction and there is no foreign key
/// between them — this struct is simply the place that has both open, the same
/// way [`OfficeDesk`](crate::OfficeDesk) is one level down.
pub struct Workspace {
    database: Option<PathBuf>,
    store: Store,
    cases: Vec<(CaseId, String)>,
    active_case: Option<usize>,
    enrichment: Option<EnrichmentSession>,
    office: Option<OfficeDesk>,
    acting_user: Option<ActingUser>,
    pane: Pane,
    docket_date: String,
    docket: Option<CourtDocket>,
    deadlines: Option<UpcomingDeadlines>,
}

impl Workspace {
    /// Opens (and, when absent, initializes) a case database.
    pub fn open(path: impl AsRef<Path>) -> GuiResult<Self> {
        let path = path.as_ref();
        let store = Store::open(path)?;
        Self::with_store(Some(path.to_path_buf()), store)
    }

    /// Creates an in-memory workspace, primarily for adapters and tests.
    pub fn in_memory() -> GuiResult<Self> {
        Self::with_store(None, Store::in_memory()?)
    }

    fn with_store(database: Option<PathBuf>, store: Store) -> GuiResult<Self> {
        let cases = docket_pairs(&store)?;
        let active_case = (!cases.is_empty()).then_some(0);
        // The office is a separate file beside the case database, and its
        // absence is not fatal: every evidence surface still reads, and the
        // Court pane says why it is empty rather than refusing to open.
        let office = match &database {
            Some(path) => OfficeDesk::open(OfficeDesk::beside(path)).ok(),
            None => OfficeDesk::in_memory().ok(),
        };
        let docket_date = office
            .as_ref()
            .and_then(|desk| desk.today().ok())
            .unwrap_or_default();
        Ok(Self {
            database,
            store,
            cases,
            active_case,
            enrichment: None,
            office,
            acting_user: None,
            pane: Pane::Court,
            docket_date,
            docket: None,
            deadlines: None,
        })
    }

    /// Database path, absent only for an in-memory workspace.
    pub fn database(&self) -> Option<&Path> {
        self.database.as_deref()
    }

    /// Cases as `(stable identifier, display name)` pairs.
    pub fn cases(&self) -> &[(CaseId, String)] {
        &self.cases
    }

    /// Index of the active case.
    pub const fn active_case_index(&self) -> Option<usize> {
        self.active_case
    }

    /// Active case as its stable identifier and display name.
    pub fn active_case(&self) -> Option<&(CaseId, String)> {
        self.active_case.and_then(|index| self.cases.get(index))
    }

    /// Selects a case by its index in [`Self::cases`].
    pub fn select_case(&mut self, index: usize) -> GuiResult<()> {
        if index >= self.cases.len() {
            return Err(GuiError::new(format!("case index {index} does not exist")));
        }
        self.active_case = Some(index);
        self.enrichment = None;
        Ok(())
    }

    /// Opens a new empty case from a JSON payload and selects it.
    ///
    /// An empty payload opens an untitled case with an initial production so
    /// intake can attach originals without a fixture.
    pub fn open_case_json(&mut self, payload: &str) -> GuiResult<String> {
        let proposal = if payload.trim().is_empty() {
            ProposedCase {
                id: None,
                name: "Untitled case".to_owned(),
                reference: None,
                jurisdiction: None,
                production: None,
            }
        } else {
            serde_json::from_str(payload)?
        };
        let opened = self.store.open_case(&proposal)?;
        self.refresh_cases()?;
        if let Some(index) = self
            .cases
            .iter()
            .position(|(candidate, _)| candidate.0 == opened.id)
        {
            self.active_case = Some(index);
        }
        self.enrichment = None;
        formatted(&opened)
    }

    /// Seeds one curated demonstration case and selects it.
    pub fn seed(&mut self, fixture: DemoFixture) -> GuiResult<CaseId> {
        let id = fixture.seed(&mut self.store)?;
        self.refresh_cases()?;
        self.active_case = self
            .cases
            .iter()
            .position(|(candidate, _)| candidate == &id);
        self.enrichment = None;
        Ok(id)
    }

    /// Imports one already-normalized adapter batch from JSON.
    pub fn import_json(&mut self, json: &str) -> GuiResult<String> {
        let batch: NormalizedBatch = serde_json::from_str(json)?;
        self.store.import_normalized(&batch)?;
        self.refresh_cases()?;
        if let Some(index) = self
            .cases
            .iter()
            .position(|(candidate, _)| candidate == &batch.case_id)
        {
            self.active_case = Some(index);
        }
        Ok(format!(
            "Imported {} source(s) into {}.",
            batch.sources.len(),
            batch.case_id
        ))
    }

    /// Imports a human-authored interpretation batch from JSON.
    pub fn import_interpretations_json(&mut self, json: &str) -> GuiResult<String> {
        let batch: InterpretationBatch = serde_json::from_str(json)?;
        if &batch.case_id != self.active_case_id()? {
            return Err(GuiError::new(
                "interpretation batch belongs to a different case",
            ));
        }
        self.store.import_interpretations(&batch)?;
        Ok(format!(
            "Imported {} source profile(s), {} group(s), and {} interpretation(s).",
            batch.source_profiles.len(),
            batch.content_groups.len(),
            batch.interpretations.len()
        ))
    }

    /// Lists the case's originals with how much of each has been read.
    pub fn enrichment_sources(&self) -> GuiResult<Vec<EnrichmentSource>> {
        Ok(self.store.enrichment_sources(self.active_case_id()?)?)
    }

    /// Reads the current source profile, when one has been written.
    pub fn source_profile(&self, source_id: &str) -> GuiResult<Option<SourceProfile>> {
        Ok(self
            .store
            .current_source_profile(self.active_case_id()?, source_id)?)
    }

    /// Writes one source-profile version and refreshes any open sweep.
    ///
    /// This is the decide-once screen: the role, author, creation date and
    /// default reading entered here are inherited by every passage in the
    /// source, so a sweep only has to touch the exceptions.
    ///
    /// Profiles are versioned by superseding rather than overwriting, and the
    /// screen edits "the profile of this source" rather than one version of
    /// it, so a save that names no predecessor supersedes whichever version is
    /// current. The earlier version stays readable.
    pub fn save_source_profile(&mut self, profile: &ProposedSourceProfile) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        if profile.created_by.trim().is_empty() || profile.created_by.starts_with("suggest:") {
            return Err(GuiError::new(
                "a source profile must name the person who decided it",
            ));
        }
        let mut profile = profile.clone();
        if profile.supersedes_profile_id.is_none() {
            profile.supersedes_profile_id = self
                .store
                .current_source_profile(&case_id, &profile.source_id)?
                .map(|current| current.id);
        }
        let profile = &profile;
        let written = self.store.append_source_profile(&case_id, profile)?;
        if let Some(session) = self.enrichment.as_mut()
            && session.source_id == profile.source_id
        {
            session.rows = self
                .store
                .enrichment_passages(&case_id, &session.source_id)?;
            session.cursor = session.cursor.min(session.rows.len().saturating_sub(1));
        }
        json(&written)
    }

    /// Offers the case's entities for `@` autocomplete, nearest match first.
    pub fn entity_candidates(&self, prefix: &str, limit: u32) -> GuiResult<Vec<EntityCandidate>> {
        Ok(self
            .store
            .entity_candidates(self.active_case_id()?, prefix, limit)?)
    }

    /// Creates one entity from inside `@` autocomplete.
    ///
    /// `possibly_same_as` records the reviewer's doubt rather than resolving
    /// it: the two records stay two records, joined by an unreviewed
    /// `possibly_same_person` edge that says a person raised the question.
    pub fn create_entity(
        &mut self,
        display_name: &str,
        kind: EntityKind,
        actor: &str,
        possibly_same_as: Option<&str>,
    ) -> GuiResult<AuthoredEntity> {
        let case_id = self.active_case_id()?.clone();
        let actor = actor.trim();
        if actor.is_empty() || actor.starts_with("suggest:") {
            return Err(GuiError::new(
                "creating an entity requires the name of the person making the decision",
            ));
        }
        let entity = self.store.record_entity(
            &case_id,
            &ProposedEntity {
                id: None,
                kind,
                display_name: display_name.to_owned(),
                is_client: false,
                notes: None,
            },
        )?;
        if let Some(other) = possibly_same_as.map(str::trim).filter(|id| !id.is_empty()) {
            self.store.link_evidence(
                &case_id,
                &ProposedLink {
                    id: None,
                    from: NodeRef {
                        kind: NodeKind::Entity,
                        id: entity.id.clone(),
                    },
                    relation: crate::EdgeKind::PossiblySamePerson,
                    to: NodeRef {
                        kind: NodeKind::Entity,
                        id: other.to_owned(),
                    },
                    rationale: format!(
                        "Entered as a separate record during enrichment; `{}` may be the same \
                         person and the two are not merged.",
                        entity.display_name
                    ),
                    author: actor.to_owned(),
                },
            )?;
        }
        Ok(entity)
    }

    /// Runs only the deterministic enrichment rules over the active case.
    ///
    /// Candidates cost one key to accept and one to refuse, so they are worth
    /// producing before a sweep rather than during it.
    pub fn suggest_enrichment(&mut self) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        let kinds = SuggestionKind::ALL
            .into_iter()
            .filter(|kind| kind.is_enrichment())
            .collect::<Vec<_>>();
        json(&self.store.suggest(&case_id, &kinds)?)
    }

    /// Opens one source in the platform-neutral keyboard enrichment workspace.
    pub fn open_enrichment(&mut self, source_id: &str, actor: &str) -> GuiResult<()> {
        let actor = actor.trim();
        if actor.is_empty() || actor.starts_with("suggest:") {
            return Err(GuiError::new(
                "enrichment requires the name of the person making decisions",
            ));
        }
        let case_id = self.active_case_id()?.clone();
        let rows = self.store.enrichment_passages(&case_id, source_id)?;
        if rows.is_empty() {
            return Err(GuiError::new(format!(
                "source `{source_id}` has no active passages"
            )));
        }
        self.enrichment = Some(EnrichmentSession {
            source_id: source_id.to_owned(),
            actor: actor.to_owned(),
            field: EnrichmentField::ContentForm,
            rows,
            cursor: 0,
            visual_mark: None,
            last_value: None,
            sticky_time_basis: None,
            pending_input: None,
            command_mode: false,
            keystrokes: 0,
        });
        Ok(())
    }

    /// Current enrichment session for native frontends and test harnesses.
    pub const fn enrichment_session(&self) -> Option<&EnrichmentSession> {
        self.enrichment.as_ref()
    }

    /// Locator-synchronized preview for the selected enrichment passage.
    pub fn enrichment_preview(&self) -> GuiResult<PreviewDescriptor> {
        let session = self
            .enrichment
            .as_ref()
            .ok_or_else(|| GuiError::new("No enrichment source is open."))?;
        let passage = session
            .selected()
            .ok_or_else(|| GuiError::new("The enrichment source has no selected passage."))?;
        Ok(self
            .store
            .preview_descriptor(self.active_case_id()?, passage)?)
    }

    /// Applies one physical key from the enrichment grammar.
    pub fn enrichment_key(&mut self, key: &str) -> GuiResult<String> {
        self.with_enrichment(|store, case_id, session| {
            session.keystrokes = session.keystrokes.saturating_add(1);
            session.pending_input = None;
            match key {
                "j" | "Down" => session.move_by(1),
                "k" | "Up" => session.move_by(-1),
                "J" => session.move_to_work(true),
                "K" => session.move_to_work(false),
                "F2" => session.field = EnrichmentField::ContentForm,
                "F3" => session.field = EnrichmentField::Speaker,
                "F4" => session.field = EnrichmentField::AttributedPerson,
                "F5" => session.field = EnrichmentField::TemporalStance,
                "F6" => session.field = EnrichmentField::Time,
                "F7" => session.field = EnrichmentField::Location,
                "F8" => session.field = EnrichmentField::PerceptionBasis,
                "Esc" => session.command_mode = true,
                "v" => session.visual_mark = Some(session.cursor),
                "Enter" => accept_candidate(store, case_id, session)?,
                "x" => reject_candidate(store, case_id, session)?,
                "." => {
                    let value = session.last_value.clone().ok_or_else(|| {
                        GuiError::new("No enrichment value has been entered yet.")
                    })?;
                    apply_value(store, case_id, session, value, ApplyRange::One)?;
                }
                "n" => apply_value(
                    store,
                    case_id,
                    session,
                    EnrichmentValue::Materiality(Materiality::Boilerplate),
                    ApplyRange::One,
                )?,
                "@" => session.pending_input = Some(PendingInput::Entity),
                "t" => session.pending_input = Some(PendingInput::Time),
                "s" => session.pending_input = Some(PendingInput::Span),
                "/" => session.pending_input = Some(PendingInput::Search),
                "g" => group_with_next(store, case_id, session)?,
                "p" => set_nearest_reporting_parent(store, case_id, session)?,
                "u" if session.command_mode => undo_last(store, case_id, session)?,
                "?" => {}
                candidate => {
                    let is_shifted = candidate.chars().count() == 1
                        && candidate.chars().next().is_some_and(char::is_uppercase);
                    let lowered = candidate.to_ascii_lowercase();
                    let value = value_for_key(session.field, &lowered).ok_or_else(|| {
                        GuiError::new(format!(
                            "Key `{candidate}` has no value in the {:?} sweep.",
                            session.field
                        ))
                    })?;
                    let range = if let Some(mark) = session.visual_mark.take() {
                        ApplyRange::Between(mark, session.cursor)
                    } else if is_shifted {
                        ApplyRange::PageToEnd
                    } else {
                        ApplyRange::One
                    };
                    apply_value(store, case_id, session, value, range)?;
                }
            }
            json(session)
        })
    }

    /// Completes an `@` entity selection for the current speaker/attribution/location field.
    pub fn enrichment_submit_entity(&mut self, entity_id: &str) -> GuiResult<String> {
        self.with_enrichment(|store, case_id, session| {
            session.keystrokes = session.keystrokes.saturating_add(1);
            if session.pending_input != Some(PendingInput::Entity) {
                return Err(GuiError::new("The enrichment workspace is not awaiting an entity."));
            }
            let value = match session.field {
                EnrichmentField::Speaker => EnrichmentValue::Speaker(entity_id.to_owned()),
                EnrichmentField::AttributedPerson => {
                    EnrichmentValue::AttributedPerson(entity_id.to_owned())
                }
                EnrichmentField::Location => EnrichmentValue::LocationEntity(entity_id.to_owned()),
                _ => {
                    return Err(GuiError::new(
                        "Entity selection is available only for speaker, attributed person, or location.",
                    ));
                }
            };
            session.pending_input = None;
            apply_value(store, case_id, session, value, ApplyRange::One)?;
            json(session)
        })
    }

    /// Completes a `t` time entry, reusing the session's sticky basis when omitted.
    pub fn enrichment_submit_time(&mut self, entry: &TimeEntry) -> GuiResult<String> {
        self.with_enrichment(|store, case_id, session| {
            session.keystrokes = session.keystrokes.saturating_add(1);
            if session.pending_input != Some(PendingInput::Time) {
                return Err(GuiError::new(
                    "The enrichment workspace is not awaiting a time.",
                ));
            }
            let value = entry.value.trim();
            if value.is_empty() {
                return Err(GuiError::new("Time entry is empty."));
            }
            let selected = session
                .selected()
                .ok_or_else(|| GuiError::new("No passage is selected."))?;
            let mut proposal = snapshot(selected, &session.actor);
            let value = if entry.approximate {
                format!("~{value}")
            } else {
                value.to_owned()
            };
            if entry.asserted {
                proposal.asserted_start = Some(value);
                proposal
                    .field_provenance
                    .insert("asserted_start".to_owned(), "entered".to_owned());
            } else {
                let basis = entry
                    .basis
                    .as_deref()
                    .map(str::trim)
                    .filter(|basis| !basis.is_empty())
                    .map(str::to_owned)
                    .or_else(|| session.sticky_time_basis.clone())
                    .ok_or_else(|| {
                        GuiError::new("Normalized time requires an alignment basis for this sweep.")
                    })?;
                proposal.normalized_start = Some(value);
                proposal.time_alignment_basis = Some(basis.clone());
                proposal
                    .field_provenance
                    .insert("normalized_start".to_owned(), "entered".to_owned());
                session.sticky_time_basis = Some(basis);
            }
            store.append_interpretation(case_id, &proposal)?;
            refresh_enrichment(store, case_id, session)?;
            session.pending_input = None;
            session.advance();
            json(session)
        })
    }

    /// Completes a location entry with the wording the source used.
    pub fn enrichment_submit_location(&mut self, text: &str) -> GuiResult<String> {
        let value = EnrichmentValue::Location(text.to_owned());
        self.with_enrichment(|store, case_id, session| {
            session.keystrokes = session.keystrokes.saturating_add(1);
            if session.field != EnrichmentField::Location {
                return Err(GuiError::new(
                    "Location text belongs to the location sweep (F7).",
                ));
            }
            session.pending_input = None;
            apply_value(store, case_id, session, value, ApplyRange::One)?;
            json(session)
        })
    }

    /// Moves the sweep cursor to one row, as a click or a find does.
    ///
    /// It costs a keystroke like `j` does: reaching a passage is work whether
    /// the reviewer walked to it or pointed at it, and the throughput budget
    /// would flatter itself if pointing were free.
    pub fn enrichment_select(&mut self, index: usize) -> GuiResult<String> {
        self.with_enrichment(|_, _, session| {
            if index >= session.rows.len() {
                return Err(GuiError::new("That passage is not in this source."));
            }
            session.keystrokes = session.keystrokes.saturating_add(1);
            session.pending_input = None;
            session.cursor = index;
            json(session)
        })
    }

    /// Renders the open sweep as grid rows for a native frontend.
    pub fn enrichment_rows(&self) -> GuiResult<Vec<EnrichmentRow>> {
        let session = self
            .enrichment
            .as_ref()
            .ok_or_else(|| GuiError::new("No enrichment source is open."))?;
        Ok(session
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| EnrichmentRow::of(index, row, session.field))
            .collect())
    }

    /// Completes an `s` sub-span selection using UTF-8 byte offsets.
    pub fn enrichment_submit_span(&mut self, start: u32, end: u32) -> GuiResult<String> {
        self.with_enrichment(|store, case_id, session| {
            session.keystrokes = session.keystrokes.saturating_add(1);
            if session.pending_input != Some(PendingInput::Span) {
                return Err(GuiError::new(
                    "The enrichment workspace is not awaiting a span.",
                ));
            }
            let selected = session
                .selected()
                .ok_or_else(|| GuiError::new("No passage is selected."))?;
            let target = InterpretationTarget::Content {
                id: selected.content_id.clone(),
            };
            let current = store.current_interpretation(case_id, &target, Some(start), Some(end))?;
            let mut proposal = snapshot(selected, &session.actor);
            proposal.char_start = Some(start);
            proposal.char_end = Some(end);
            proposal.supersedes_interpretation_id = current.map(|item| item.id);
            store.append_interpretation(case_id, &proposal)?;
            refresh_enrichment(store, case_id, session)?;
            session.pending_input = None;
            json(session)
        })
    }

    fn with_enrichment<T>(
        &mut self,
        operation: impl FnOnce(&mut Store, &CaseId, &mut EnrichmentSession) -> GuiResult<T>,
    ) -> GuiResult<T> {
        let case_id = self.active_case_id()?.clone();
        let mut session = self
            .enrichment
            .take()
            .ok_or_else(|| GuiError::new("No enrichment source is open."))?;
        let result = operation(&mut self.store, &case_id, &mut session);
        self.enrichment = Some(session);
        result
    }

    /// Productions available to receive intake for the selected case.
    pub fn productions(&self) -> GuiResult<Vec<OpenedProduction>> {
        Ok(self.store.productions(self.active_case_id()?)?)
    }

    /// Open a production on the selected case.
    pub fn open_production(&mut self, label: &str) -> GuiResult<OpenedProduction> {
        let case_id = self.active_case_id()?.clone();
        Ok(self.store.open_production(
            &case_id,
            &ProposedProduction {
                id: None,
                label: label.to_owned(),
                received_at: None,
                producing_party: None,
                notes: None,
            },
        )?)
    }

    /// Persist one fully resolved adapter request.
    pub fn queue_intake(&mut self, request: &AdapterJobRequest) -> GuiResult<IntakeJob> {
        if request.case_id != self.active_case_id()?.0 {
            return Err(GuiError::new("intake request belongs to a different case"));
        }
        let request_json = serde_json::to_string_pretty(request)?;
        Ok(self
            .store
            .enqueue_intake_job(&NewIntakeJob { request_json })?)
    }

    /// Queue rows for the selected case.
    pub fn intake_jobs(&self) -> GuiResult<Vec<IntakeJob>> {
        Ok(self.store.intake_jobs(self.active_case_id()?)?)
    }

    /// Retained artifacts for one queue row.
    pub fn intake_artifacts(&self, job_id: &str) -> GuiResult<Vec<IntakeArtifact>> {
        Ok(self.store.intake_artifacts(job_id)?)
    }

    /// Retry a failed/interrupted row with its new immutable attempt request.
    pub fn retry_intake(
        &mut self,
        job_id: &str,
        request: &AdapterJobRequest,
    ) -> GuiResult<IntakeJob> {
        Ok(self
            .store
            .retry_intake_job(job_id, &serde_json::to_string_pretty(request)?)?)
    }

    /// Whether switching away would hide an actively running process/import.
    pub fn has_active_intake(&self) -> GuiResult<bool> {
        Ok(self.store.has_active_intake()?)
    }

    /// Commit an adapter manifest, exposed for platform coordinators/tests.
    pub fn commit_intake(
        &mut self,
        job_id: &str,
        manifest: &AdapterResultManifest,
    ) -> GuiResult<()> {
        self.store.commit_intake_result(job_id, manifest)?;
        Ok(())
    }

    /// Score-free chronological finder hits selected with a caller-provided
    /// text embedding in the exact stored model space.
    pub fn search_frames(
        &self,
        model: &str,
        vector: &[f32],
        limit: u32,
    ) -> GuiResult<Vec<KeyframeHit>> {
        Ok(self
            .store
            .search_keyframes(self.active_case_id()?, model, vector, limit)?)
    }

    /// Stored visual finder model spaces for the selected case.
    pub fn keyframe_models(&self) -> GuiResult<Vec<String>> {
        Ok(self.store.keyframe_models(self.active_case_id()?)?)
    }

    /// Current hash-verified path record for one source.
    pub fn source_location(&self, source_id: &str) -> GuiResult<SourceLocation> {
        Ok(self.store.source_location(source_id)?)
    }

    /// Renders one case read model as presentation-ready text.
    pub fn render(&self, view: WorkspaceView) -> GuiResult<String> {
        let case_id = self.active_case_id()?;
        match view {
            WorkspaceView::Overview => formatted(&self.store.overview(case_id)?),
            WorkspaceView::Standing => formatted(&self.store.case_standing(case_id)?),
            WorkspaceView::Discovery => formatted(&self.store.discovery_ledger(case_id)?),
            WorkspaceView::Elements => formatted(&self.store.element_matrix(case_id)?),
            WorkspaceView::Timeline => formatted(&self.store.contested_timeline(case_id)?),
            WorkspaceView::Collation => Ok(collation_text(&self.store.collation_index(case_id)?)),
            WorkspaceView::Issues => formatted(&self.store.issue_workspaces(case_id)?),
            WorkspaceView::Offenses => formatted(&self.store.offense_comparison(case_id)?),
            WorkspaceView::ReviewQueue => formatted(&self.store.review_queue(case_id)?),
            WorkspaceView::ReviewHistory => formatted(&self.store.review_history(case_id, None)?),
            WorkspaceView::Packets => Ok(packets_text(&self.store.proposition_packets(case_id)?)),
            WorkspaceView::Digest => Ok(digest_text(
                &self.store.case_digest(case_id, ExportAudience::WorkFile)?,
            )),
        }
    }

    /// Searches source-grounded case content.
    pub fn search(&self, query: &str, limit: u32) -> GuiResult<String> {
        formatted(&self.store.search(self.active_case_id()?, query, limit)?)
    }

    /// Runs every deterministic analyzer and returns proposals plus findings.
    pub fn suggest_all(&mut self) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        formatted(&self.store.suggest(&case_id, &SuggestionKind::ALL)?)
    }

    /// Applies one named human review decision.
    pub fn review(
        &mut self,
        target: ReviewTarget,
        target_id: String,
        to_state: ReviewState,
        actor: String,
        basis: Option<String>,
        locator: Option<String>,
    ) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        let decision = ReviewDecision {
            target,
            target_id,
            to_state,
            actor,
            basis,
            verified_against_locator: locator,
        };
        formatted(&self.store.apply_review(&case_id, &decision)?)
    }

    /// Parses a typed authoring payload and writes it through the kernel API.
    pub fn author_json(&mut self, kind: AuthorKind, payload: &str) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        match kind {
            AuthorKind::Proposition => {
                let value: ProposedProposition = serde_json::from_str(payload)?;
                formatted(&self.store.author_proposition(&case_id, &value)?)
            }
            AuthorKind::Link => {
                let value: ProposedLink = serde_json::from_str(payload)?;
                formatted(&self.store.link_evidence(&case_id, &value)?)
            }
            AuthorKind::Entity => {
                let value: ProposedEntity = serde_json::from_str(payload)?;
                formatted(&self.store.record_entity(&case_id, &value)?)
            }
            AuthorKind::Charge => {
                let value: ProposedCharge = serde_json::from_str(payload)?;
                formatted(&self.store.record_charge(&case_id, &value)?)
            }
            AuthorKind::WorkProduct => {
                let value: ProposedAdvocacyItem = serde_json::from_str(payload)?;
                formatted(&self.store.author_advocacy_item(&case_id, &value)?)
            }
            AuthorKind::Note => {
                let value: ProposedAnnotation = serde_json::from_str(payload)?;
                formatted(&self.store.annotate(&case_id, &value)?)
            }
            AuthorKind::Brief => {
                let value: ProposedBrief = serde_json::from_str(payload)?;
                formatted(&self.store.record_brief(&case_id, &value)?)
            }
            AuthorKind::ElementMapping => {
                let value: ProposedElementMapping = serde_json::from_str(payload)?;
                formatted(&self.store.map_element(&case_id, &value)?)
            }
            AuthorKind::Occurrence => {
                let value: ProposedOccurrence = serde_json::from_str(payload)?;
                formatted(&self.store.author_occurrence(&case_id, &value)?)
            }
        }
    }

    /// Produces a source-linked export as JSON.
    pub fn export(&self, audience: ExportAudience) -> GuiResult<String> {
        json(&self.store.export_case(self.active_case_id()?, audience)?)
    }

    /// Renders an export as readable text for an on-screen preview.
    ///
    /// The artifact written to disk stays JSON — [`Self::export`] and
    /// [`Self::save_export`] are the contract; this is only how a screen shows
    /// what that file will hold.
    pub fn export_preview(&self, audience: ExportAudience) -> GuiResult<String> {
        formatted(&self.store.export_case(self.active_case_id()?, audience)?)
    }

    /// Writes an export to a caller-selected path.
    pub fn save_export(&self, path: impl AsRef<Path>, audience: ExportAudience) -> GuiResult<()> {
        fs::write(path, self.export(audience)?)?;
        Ok(())
    }

    fn refresh_cases(&mut self) -> GuiResult<()> {
        let selected_id = self
            .active_case
            .and_then(|index| self.cases.get(index))
            .map(|(id, _)| id.clone());
        self.cases = docket_pairs(&self.store)?;
        self.active_case = selected_id
            .as_ref()
            .and_then(|selected| self.cases.iter().position(|(id, _)| id == selected))
            .or_else(|| (!self.cases.is_empty()).then_some(0));
        Ok(())
    }

    fn active_case_id(&self) -> GuiResult<&CaseId> {
        self.active_case
            .and_then(|index| self.cases.get(index))
            .map(|(id, _)| id)
            .ok_or_else(|| {
                GuiError::new("No case is selected. Open a case, or seed a demonstration case.")
            })
    }

    // ----- the Court pane ---------------------------------------------------

    /// Which half of the workspace is on screen.
    pub const fn pane(&self) -> Pane {
        self.pane
    }

    /// Switches panes. Nothing is reloaded here; the frontend asks for what the
    /// pane it is showing needs.
    pub const fn set_pane(&mut self, pane: Pane) {
        self.pane = pane;
    }

    /// Whether an office database is open beside the case database.
    pub const fn has_office(&self) -> bool {
        self.office.is_some()
    }

    /// The day the Court pane is showing, in `YYYY-MM-DD`.
    pub fn docket_date(&self) -> &str {
        &self.docket_date
    }

    /// The day's name, so a reader knows a Monday from a Friday at a glance.
    pub fn docket_weekday(&self) -> String {
        self.docket
            .as_ref()
            .map_or_else(String::new, |docket| docket.day.weekday.clone())
    }

    /// Reloads the day on screen: its settings, and what is owed from it.
    pub fn refresh_court(&mut self) -> GuiResult<()> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        if self.docket_date.is_empty() {
            self.docket_date = office.today()?;
        }
        self.docket = Some(office.court_docket(&self.store, &self.docket_date)?);
        self.deadlines = Some(office.upcoming_deadlines(&self.docket_date, DEADLINE_WINDOW_DAYS)?);
        Ok(())
    }

    /// Moves the Court pane to today.
    pub fn docket_today(&mut self) -> GuiResult<()> {
        self.docket_date = self.office.as_ref().ok_or_else(no_office)?.today()?;
        self.refresh_court()
    }

    /// Moves the Court pane a number of days, forward or back.
    pub fn docket_shift_days(&mut self, days: i64) -> GuiResult<()> {
        self.docket_date = self.civil_date()?.add_days(days).to_text();
        self.refresh_court()
    }

    /// Moves the Court pane a number of weeks, forward or back.
    pub fn docket_shift_weeks(&mut self, weeks: i64) -> GuiResult<()> {
        self.docket_date = self.civil_date()?.add_weeks(weeks).to_text();
        self.refresh_court()
    }

    /// Moves the Court pane to a typed day.
    ///
    /// A day that is not a real date is refused here rather than reaching the
    /// office database, which validates the shape of a date string but cannot
    /// tell February the thirtieth from a day that exists.
    pub fn set_docket_date(&mut self, date: &str) -> GuiResult<()> {
        let date = date.trim();
        let parsed = if date.is_empty() || date.eq_ignore_ascii_case("today") {
            return self.docket_today();
        } else {
            CivilDate::parse(date).ok_or_else(|| {
                GuiError::new(format!(
                    "{date:?} is not a day. Dates are written 2026-08-31."
                ))
            })?
        };
        self.docket_date = parsed.to_text();
        self.refresh_court()
    }

    fn civil_date(&self) -> GuiResult<CivilDate> {
        let held = if self.docket_date.is_empty() {
            self.office.as_ref().ok_or_else(no_office)?.today()?
        } else {
            self.docket_date.clone()
        };
        CivilDate::parse(&held)
            .ok_or_else(|| GuiError::new(format!("{held:?} is not a day the calendar can move.")))
    }

    /// The docket, flattened to what a grid paints.
    pub fn docket_rows(&self) -> Vec<DocketGridRow> {
        self.docket
            .as_ref()
            .map(|docket| docket.rows.iter().map(DocketGridRow::of).collect())
            .unwrap_or_default()
    }

    /// What is owed from the day on screen, overdue first.
    pub fn deadline_rows(&self) -> Vec<DeadlineGridRow> {
        let Some(deadlines) = self.deadlines.as_ref() else {
            return Vec::new();
        };
        deadlines
            .overdue
            .iter()
            .chain(deadlines.upcoming.iter())
            .map(DeadlineGridRow::of)
            .collect()
    }

    /// One line summarizing the day, for the Court pane's status.
    pub fn docket_summary(&self) -> String {
        let Some(docket) = self.docket.as_ref() else {
            return "No day is loaded.".to_owned();
        };
        let settings = docket.rows.len();
        let matters: usize = docket.rows.iter().map(|row| row.entry.matters.len()).sum();
        let deadlines = self
            .deadlines
            .as_ref()
            .map_or(0, |owed| owed.overdue.len() + owed.upcoming.len());
        format!(
            "{} {} · {settings} settings · {matters} matters · {deadlines} owed within {DEADLINE_WINDOW_DAYS} days",
            docket.day.date, docket.day.weekday
        )
    }

    /// Everything behind one docket row, for the detail pane.
    ///
    /// This is the "show context, edit in place" rule made concrete: selecting
    /// a setting expands what is already on screen rather than navigating away
    /// from it. Privileged analysis is not here — advocacy items, annotations
    /// and decision briefs are never read by this path.
    pub fn court_detail(&self, index: usize) -> GuiResult<String> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        let docket = self
            .docket
            .as_ref()
            .ok_or_else(|| GuiError::new("No day is loaded."))?;
        let row = docket
            .rows
            .get(index)
            .ok_or_else(|| GuiError::new(format!("row {index} is not on this docket")))?;

        let client = office.store().client_profile(&row.entry.client_id)?;
        let linked = office.store().linked_entities(&row.entry.client_id)?;
        let mut matters = Vec::with_capacity(row.entry.matters.len());
        for line in &row.entry.matters {
            let profile = office.store().matter_profile(&line.id)?;
            let evidence = match profile.evidence_case_id.as_deref() {
                Some(case_id) => self.matter_evidence(case_id, &linked)?,
                None => None,
            };
            matters.push(MatterDetail {
                matter: profile,
                evidence,
            });
        }

        formatted(&CourtDetail {
            setting: SettingHead {
                date: row.entry.date.clone(),
                weekday: docket.day.weekday.clone(),
                time: row.entry.time.clone(),
                what_for: row.entry.appearance_type.clone(),
                court: row.entry.court.clone(),
                room: row.entry.room.clone(),
                judge: row.entry.judge.clone(),
                outcome: row.entry.outcome.clone(),
                evidence_posture: row.posture_line(),
            },
            client,
            matters,
        })
    }

    /// The kernel's own reading of one matter's case: what it rests on, the
    /// competing accounts of when, and what the client's linked witnesses said.
    ///
    /// `None` when the kernel does not hold the case, which is reported as the
    /// broken link it is rather than as an empty case.
    fn matter_evidence(
        &self,
        case_id: &str,
        linked: &[(String, String)],
    ) -> GuiResult<Option<MatterEvidence>> {
        let case = CaseId(case_id.to_owned());
        let standing = match self.store.case_standing(&case) {
            Ok(standing) => standing,
            Err(crate::Error::NotFound { .. }) => return Ok(None),
            Err(other) => return Err(other.into()),
        };
        let mut witnesses = Vec::new();
        for entity_id in linked
            .iter()
            .filter(|(linked_case, _)| linked_case == case_id)
            .map(|(_, entity_id)| entity_id)
        {
            witnesses.extend(self.store.witness_dossier(&case, entity_id)?);
        }
        Ok(Some(MatterEvidence {
            standing,
            timeline: self.store.contested_timeline(&case)?,
            witnesses,
        }))
    }

    // ----- reaching evidence through a matter -------------------------------

    /// Matters as `(identifier, display label)`, for the Office pane's chooser.
    ///
    /// The Office pane reaches a case *through* a matter rather than through a
    /// bare case list, because a matter is what an office actually carries and
    /// what a court number belongs to.
    pub fn office_matters(&self) -> GuiResult<Vec<(String, String)>> {
        let Some(office) = self.office.as_ref() else {
            return Ok(Vec::new());
        };
        Ok(office
            .store()
            .matters()?
            .into_iter()
            .map(|matter| {
                let label = match (&matter.court_number, &matter.evidence_case_id) {
                    (Some(number), Some(_)) => format!("{number} — {}", matter.caption),
                    (Some(number), None) => format!("{number} — {} (no case)", matter.caption),
                    (None, Some(_)) => matter.caption.clone(),
                    (None, None) => format!("{} (no case)", matter.caption),
                };
                (matter.id, label)
            })
            .collect())
    }

    /// Selects the evidence case a matter points at.
    ///
    /// Fails loudly rather than silently doing nothing: a matter with no case
    /// and a matter naming a case the kernel does not hold are different
    /// problems, and both are worth saying out loud.
    pub fn select_matter(&mut self, matter_id: &str) -> GuiResult<CaseId> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        let matter = office.store().matter_profile(matter_id)?;
        let case_id = matter.evidence_case_id.ok_or_else(|| {
            GuiError::new(format!(
                "{} has no evidence case linked. Link one before reading its discovery.",
                matter.caption
            ))
        })?;
        let index = self
            .cases
            .iter()
            .position(|(id, _)| id.0 == case_id)
            .ok_or_else(|| {
                GuiError::new(format!(
                    "{} names case {case_id}, which this evidence database does not hold.",
                    matter.caption
                ))
            })?;
        self.select_case(index)?;
        Ok(CaseId(case_id))
    }

    /// Seeds the office fixture, anchored on today so it is worth looking at.
    pub fn seed_office(&mut self) -> GuiResult<String> {
        let office = self.office.as_mut().ok_or_else(no_office)?;
        let anchor = office.today()?;
        let seeded = OfficeFixture::MisdemeanorDocket.seed_from(office.store_mut(), &anchor)?;
        self.refresh_court()?;
        Ok(seeded)
    }

    // ----- office data entry ------------------------------------------------

    /// Users the office knows, as `(id, name, role)`, for the acting-as picker.
    pub fn office_users(&self) -> GuiResult<Vec<(String, String, String)>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office.store().users()?)
    }

    /// Who office writes in this session are attributed to, once chosen.
    pub fn acting_user(&self) -> Option<&ActingUser> {
        self.acting_user.as_ref()
    }

    /// Names the person entering records, finding or creating the user row.
    ///
    /// Asked once and reused: an office record with no author is not a record,
    /// and a per-dialog author box is a per-dialog chance to typo a new one.
    pub fn set_acting_user(&mut self, display_name: &str, role: &str) -> GuiResult<String> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        let name = display_name.trim();
        if name.is_empty() {
            return Err(GuiError::new("Somebody needs a name to act as."));
        }
        let id = office.store().user_named(name, role)?;
        self.acting_user = Some(ActingUser {
            id,
            display_name: name.to_owned(),
            role: role.to_owned(),
        });
        Ok(format!("Acting as {name} ({role})."))
    }

    fn acting_user_id(&self) -> GuiResult<String> {
        self.acting_user
            .as_ref()
            .map(|user| user.id.clone())
            .ok_or_else(no_acting_user)
    }

    /// Clients as `(id, name)`, for a chooser.
    pub fn office_clients(&self) -> GuiResult<Vec<(String, String)>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office.store().clients()?)
    }

    /// Courts as `(id, name)`, for a chooser.
    pub fn office_courts(&self) -> GuiResult<Vec<(String, String)>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office.store().courts()?)
    }

    /// One client's matters as `(id, label)`, for a setting's matter list.
    pub fn office_matters_of_client(&self, client_id: &str) -> GuiResult<Vec<(String, String)>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office
            .store()
            .client_profile(client_id)?
            .matters
            .into_iter()
            .map(|matter| {
                let label = match &matter.court_number {
                    Some(number) => format!("{number} — {}", matter.caption),
                    None => matter.caption.clone(),
                };
                (matter.id, label)
            })
            .collect())
    }

    /// Languages for the client form's chooser: the office's own answers
    /// first, then the common ones it has not met yet. A list to pick from,
    /// never a limit — any typed language is accepted.
    pub fn office_languages(&self) -> GuiResult<Vec<String>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        let mut languages = office.store().client_languages()?;
        for candidate in COMMON_LANGUAGES {
            if !languages.iter().any(|known| known == candidate) {
                languages.push(candidate.to_owned());
            }
        }
        Ok(languages)
    }

    /// Settings on one day as `(id, label)`, for filing a note under one.
    pub fn office_settings_on(&self, date: &str) -> GuiResult<Vec<(String, String)>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office
            .store()
            .docket_day(date)?
            .settings
            .into_iter()
            .map(|entry| {
                let time = entry.time.as_deref().unwrap_or("time not set");
                let label = format!("{time} · {} · {}", entry.appearance_type, entry.client);
                (entry.id, label)
            })
            .collect())
    }

    /// People the office may already know under this name or these contacts.
    ///
    /// Advisory only: nothing is merged and nothing is written, whatever this
    /// returns. The caller shows the candidates and a named person decides.
    pub fn client_duplicates(
        &self,
        name: &str,
        contacts: &[String],
    ) -> GuiResult<Vec<PossiblePerson>> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office
            .store()
            .possible_client_duplicates(name, contacts, None)?)
    }

    /// Opens a client record from the entry form.
    pub fn create_client(&mut self, draft: &ClientDraft) -> GuiResult<String> {
        let author = self.acting_user_id()?;
        let office = self.office.as_mut().ok_or_else(no_office)?;
        let mut contacts = Vec::new();
        for (kind, value) in [
            (ContactKind::Phone, &draft.phone),
            (ContactKind::Email, &draft.email),
            (ContactKind::Address, &draft.address),
        ] {
            if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                contacts.push(ProposedClientContact {
                    kind,
                    value: value.to_owned(),
                    label: None,
                    is_primary: true,
                });
            }
        }
        let profile = office.store_mut().create_client(&ProposedClient {
            id: None,
            display_name: draft.display_name.clone(),
            date_of_birth: draft.date_of_birth.clone(),
            sex: draft.sex,
            preferred_language: draft.preferred_language.clone(),
            notes: draft.notes.clone(),
            aliases: draft.aliases.clone(),
            contacts,
            author_user_id: author,
        })?;
        formatted(&profile)
    }

    /// Opens a matter, and its evidence case in the same flow.
    ///
    /// The kernel case is opened first, prefilled from the matter itself —
    /// caption as name, court number as reference, court as jurisdiction — so
    /// nothing is typed twice. A failure between the two writes leaves a case
    /// with no matter, which is visible on the case list and linkable by hand,
    /// rather than a matter naming a case that was never opened.
    pub fn open_matter(&mut self, draft: &MatterDraft) -> GuiResult<String> {
        let author = self.acting_user_id()?;
        if self.office.is_none() {
            return Err(no_office());
        }
        let court_name = match draft.court_id.as_deref() {
            Some(court_id) => {
                let office = self.office.as_ref().ok_or_else(no_office)?;
                office
                    .store()
                    .courts()?
                    .into_iter()
                    .find(|(id, _)| id == court_id)
                    .map(|(_, name)| name)
            }
            None => None,
        };
        let evidence_case_id = match &draft.evidence {
            EvidenceLink::None => None,
            EvidenceLink::Existing(case) => Some(case.0.clone()),
            EvidenceLink::OpenNewCase => Some(
                self.store
                    .open_case(&ProposedCase {
                        id: None,
                        name: draft.caption.clone(),
                        reference: draft.court_number.clone(),
                        jurisdiction: court_name.clone(),
                        production: None,
                    })?
                    .id,
            ),
        };
        let opened_new_case = matches!(draft.evidence, EvidenceLink::OpenNewCase);
        let office = self.office.as_ref().ok_or_else(no_office)?;
        let today = office.today()?;
        let matter_id = office
            .store()
            .open_matter(&ProposedMatter {
                id: None,
                client_id: draft.client_id.clone(),
                caption: draft.caption.clone(),
                court_number: draft.court_number.clone(),
                court_id: draft.court_id.clone(),
                status: Some(draft.status),
                custody_state: Some(draft.custody_state),
                offer_state: Some(draft.offer_state),
                offer_summary: None,
                charge_summary: draft.charge_summary.clone(),
                opened_on: Some(today),
                last_contact_on: None,
                evidence_case_id: evidence_case_id.clone(),
                author_user_id: author,
            })
            .map_err(|error| {
                if opened_new_case {
                    GuiError::new(format!(
                        "The evidence case opened but the matter did not: {error}. \
                         The case is on the case list; link it from a matter by hand."
                    ))
                } else {
                    error.into()
                }
            })?;
        let profile = office.store().matter_profile(&matter_id)?;
        self.refresh_cases()?;
        if let Some(case_id) = &evidence_case_id
            && let Some(index) = self
                .cases
                .iter()
                .position(|(candidate, _)| &candidate.0 == case_id)
        {
            self.select_case(index)?;
        }
        self.refresh_court().ok();
        formatted(&profile)
    }

    /// Records a court by name, for the matter form's "New court" row.
    pub fn create_court_named(&mut self, name: &str) -> GuiResult<String> {
        let office = self.office.as_ref().ok_or_else(no_office)?;
        Ok(office.store().create_court(&ProposedCourt {
            id: None,
            name: name.to_owned(),
            division: None,
            address: None,
            room: None,
        })?)
    }

    /// Schedules one setting covering every named matter of one client.
    pub fn schedule_setting(&mut self, draft: &SettingDraft) -> GuiResult<String> {
        let author = self.acting_user_id()?;
        let office = self.office.as_mut().ok_or_else(no_office)?;
        let judge_id = match draft.judge.as_deref().map(str::trim) {
            Some(name) if !name.is_empty() => Some(
                office
                    .store()
                    .judge_named(draft.court_id.as_deref(), name)?,
            ),
            _ => None,
        };
        let scheduled = office
            .store_mut()
            .schedule_appearance(&ProposedAppearance {
                id: None,
                client_id: draft.client_id.clone(),
                matter_ids: draft.matter_ids.clone(),
                court_id: draft.court_id.clone(),
                judge_id,
                appearance_date: draft.date.clone(),
                appearance_time: draft.time.clone(),
                appearance_type: draft.appearance_type,
                notes: draft.notes.clone(),
                author_user_id: author,
            })?;
        let day = office.store().docket_day(&draft.date)?;
        let entry = day.settings.into_iter().find(|entry| entry.id == scheduled);
        self.refresh_court().ok();
        match entry {
            Some(entry) => formatted(&entry),
            None => Ok(format!("Setting scheduled for {}.", draft.date)),
        }
    }

    /// Records what is owed on a matter and when.
    pub fn record_office_deadline(&mut self, draft: &DeadlineDraft) -> GuiResult<String> {
        let author = self.acting_user_id()?;
        let office = self.office.as_ref().ok_or_else(no_office)?;
        office.store().record_deadline(&ProposedDeadline {
            id: None,
            matter_id: draft.matter_id.clone(),
            description: draft.description.clone(),
            due_date: draft.due_date.clone(),
            origin: draft.origin,
            author_user_id: author,
        })?;
        self.refresh_court().ok();
        Ok(format!(
            "Deadline recorded — {} due {}.",
            draft.description, draft.due_date
        ))
    }

    /// Files a note under exactly one client, matter, or setting.
    pub fn write_office_note(&mut self, draft: &NoteDraft) -> GuiResult<String> {
        let author = self.acting_user_id()?;
        let office = self.office.as_mut().ok_or_else(no_office)?;
        let subject = draft.subject_id.clone();
        let (client_id, matter_id, appearance_id) = match draft.scope {
            NoteScope::Client => (Some(subject), None, None),
            NoteScope::Matter => (None, Some(subject), None),
            NoteScope::Appearance => (None, None, Some(subject)),
        };
        let note = office.store_mut().write_note(&ProposedNote {
            id: None,
            client_id,
            matter_id,
            appearance_id,
            body: draft.body.clone(),
            author_user_id: author,
        })?;
        formatted(&note)
    }
}

/// How far ahead the Court pane looks for what is owed.
///
/// A fortnight: far enough that a continuance set for next week is already
/// visible, near enough that the list stays something a person reads rather
/// than scrolls past.
const DEADLINE_WINDOW_DAYS: u32 = 14;

fn no_office() -> GuiError {
    GuiError::new(
        "No office database is open beside this case database. \
         Open a database on disk, or seed the office caseload.",
    )
}

fn no_acting_user() -> GuiError {
    GuiError::new("Nobody is acting. Choose who is entering records before writing.")
}

/// The named person every office write in this session is attributed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActingUser {
    /// User identifier.
    pub id: String,
    /// The name they act under.
    pub display_name: String,
    /// The role the office knows them by.
    pub role: String,
}

/// What the client entry form collected. Carries no author: the workspace
/// supplies the acting person, so a dialog cannot write anonymously.
#[derive(Debug, Clone, Default)]
pub struct ClientDraft {
    /// The person's name as the office records it.
    pub display_name: String,
    /// Date of birth in `YYYY-MM-DD`, when known.
    pub date_of_birth: Option<String>,
    /// How the office records the person's sex. Absent means not recorded.
    pub sex: Option<Sex>,
    /// The language they ask to be spoken to in.
    pub preferred_language: Option<String>,
    /// Other names they go by.
    pub aliases: Vec<String>,
    /// A telephone number, when given.
    pub phone: Option<String>,
    /// An email address, when given.
    pub email: Option<String>,
    /// A postal address, when given.
    pub address: Option<String>,
    /// Anything worth recording about the person rather than a case.
    pub notes: Option<String>,
}

impl ClientDraft {
    /// The values the duplicate check compares on.
    pub fn contact_values(&self) -> Vec<String> {
        [&self.phone, &self.email, &self.address]
            .into_iter()
            .filter_map(|value| value.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// What a new matter does about discovery. One flow, three honest answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceLink {
    /// Open a kernel case prefilled from the matter, and link it.
    OpenNewCase,
    /// Link a case the kernel already holds.
    Existing(CaseId),
    /// No discovery yet; the matter says "(no case)" until one is linked.
    None,
}

/// What the matter entry form collected.
#[derive(Debug, Clone)]
pub struct MatterDraft {
    /// The client the matter belongs to.
    pub client_id: String,
    /// How the matter is captioned.
    pub caption: String,
    /// The court's own number for it, when known.
    pub court_number: Option<String>,
    /// The court hearing it, when known.
    pub court_id: Option<String>,
    /// Where it stands in the office.
    pub status: MatterStatus,
    /// Where the client is.
    pub custody_state: CustodyState,
    /// Where negotiation stands.
    pub offer_state: OfferState,
    /// The charges, summarized.
    pub charge_summary: Option<String>,
    /// What the matter does about discovery.
    pub evidence: EvidenceLink,
}

/// What the setting entry form collected.
#[derive(Debug, Clone)]
pub struct SettingDraft {
    /// The client called.
    pub client_id: String,
    /// Every matter the one setting covers; at least one.
    pub matter_ids: Vec<String>,
    /// The court sitting, when known.
    pub court_id: Option<String>,
    /// The judge's name, found or recorded on the way in.
    pub judge: Option<String>,
    /// The day, in `YYYY-MM-DD`.
    pub date: String,
    /// The time in 24-hour `HH:MM`, when the docket gives one.
    pub time: Option<String>,
    /// What the setting is for.
    pub appearance_type: AppearanceType,
    /// Anything the calendar should carry.
    pub notes: Option<String>,
}

/// What the deadline entry form collected.
#[derive(Debug, Clone)]
pub struct DeadlineDraft {
    /// The matter it is owed on.
    pub matter_id: String,
    /// What is owed.
    pub description: String,
    /// When, in `YYYY-MM-DD`.
    pub due_date: String,
    /// Where the obligation comes from.
    pub origin: DeadlineOrigin,
}

/// What the note entry form collected.
#[derive(Debug, Clone)]
pub struct NoteDraft {
    /// What the note is filed under: a client, a matter, or a setting.
    pub scope: NoteScope,
    /// The one subject's identifier.
    pub subject_id: String,
    /// The note itself.
    pub body: String,
}

/// Which half of the workspace is on screen.
///
/// Court opens first. The first question of a defender's day is where they
/// have to be and for whom, not what is in one case file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Pane {
    /// Today's settings, what is owed, and the posture of the linked cases.
    Court,
    /// The case file itself: intake, review, collation, authoring, export.
    Office,
}

impl Pane {
    /// Both panes, in the order the switch shows them.
    pub const ALL: [Self; 2] = [Self::Court, Self::Office];

    /// The caption on the pane switch.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Court => "Court",
            Self::Office => "Office",
        }
    }
}

/// Which Court list is being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CourtView {
    /// Settings on the day on screen.
    Docket,
    /// What is owed, counted from the day on screen.
    Deadlines,
}

/// One docket row, reduced to what a grid paints.
///
/// One row is one setting, however many matters it covers — the whole reason
/// the office layer models a setting separately from a matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocketGridRow {
    /// When it is called, or `—` when the docket gives no time.
    pub time: String,
    /// What the setting is for.
    pub what_for: String,
    /// Court and room.
    pub court: String,
    /// Whose setting it is.
    pub client: String,
    /// Court numbers of every matter it covers, in the order they are called.
    pub matters: String,
    /// The charges, summarized.
    pub charges: String,
    /// Where the client is.
    pub custody: String,
    /// Where negotiation stands.
    pub offer: String,
    /// When the client was last spoken to.
    pub last_contact: String,
    /// What the linked cases rest on. Counts and absences, never a score.
    pub posture: String,
    /// What is open on the covered matters.
    pub open_work: String,
    /// The row as one copyable line, so a number never has to be retyped.
    pub copy_text: String,
}

impl DocketGridRow {
    fn of(row: &DocketRow) -> Self {
        let entry = &row.entry;
        let matters = joined(entry.matters.iter().map(|line| {
            line.court_number
                .clone()
                .unwrap_or_else(|| line.caption.clone())
        }));
        let charges = joined(
            entry
                .matters
                .iter()
                .filter_map(|line| line.charge_summary.clone()),
        );
        let custody = distinct(
            entry
                .matters
                .iter()
                .map(|line| readable_key(&line.custody_state)),
        );
        let offer = distinct(
            entry
                .matters
                .iter()
                .map(|line| readable_key(&line.offer_state)),
        );
        let last_contact = entry
            .last_contact
            .clone()
            .unwrap_or_else(|| "not recorded".to_owned());
        let posture = row.posture_line();
        let open_work = format!("{} owed · {} notes", entry.open_deadlines, entry.notes);
        let time = entry.time.clone().unwrap_or_else(|| "—".to_owned());
        let court = match (&entry.court, &entry.room) {
            (Some(court), Some(room)) => format!("{court} · {room}"),
            (Some(court), None) => court.clone(),
            (None, _) => "not recorded".to_owned(),
        };
        let copy_text = format!(
            "{time} {}  {}  {matters}  {charges}  {custody}  {court}",
            readable_key(&entry.appearance_type),
            entry.client
        );
        Self {
            time,
            what_for: readable_key(&entry.appearance_type),
            court,
            client: entry.client.clone(),
            matters,
            charges,
            custody,
            offer,
            last_contact,
            posture,
            open_work,
            copy_text,
        }
    }
}

/// One owed thing, reduced to what a grid paints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeadlineGridRow {
    /// When it is owed, in `YYYY-MM-DD`.
    pub due: String,
    /// How long there is, or how long it has been overdue.
    pub within: String,
    /// The matter it falls on, by its court number where there is one.
    pub matter: String,
    /// Whose matter it is.
    pub client: String,
    /// What is owed.
    pub what: String,
    /// Where it comes from, which decides whether the date can move.
    pub origin: String,
    /// The row as one copyable line.
    pub copy_text: String,
}

impl DeadlineGridRow {
    fn of(row: &DeadlineRow) -> Self {
        let within = match row.days_remaining {
            0 => "today".to_owned(),
            1 => "tomorrow".to_owned(),
            -1 => "1 day overdue".to_owned(),
            days if days < 0 => format!("{} days overdue", -days),
            days => format!("in {days} days"),
        };
        let matter = row
            .court_number
            .clone()
            .unwrap_or_else(|| row.matter.clone());
        Self {
            copy_text: format!(
                "{} {matter} {} — {} ({})",
                row.due_date,
                row.client,
                row.description,
                readable_key(&row.origin)
            ),
            due: row.due_date.clone(),
            within,
            matter,
            client: row.client.clone(),
            what: row.description.clone(),
            origin: readable_key(&row.origin),
        }
    }
}

/// Everything behind one docket row.
#[derive(Debug, Clone, Serialize)]
struct CourtDetail {
    /// The setting itself.
    setting: SettingHead,
    /// The person, across every matter they have.
    client: ClientProfile,
    /// Each covered matter, and what its case rests on.
    matters: Vec<MatterDetail>,
}

/// The setting, as the head of its own detail.
#[derive(Debug, Clone, Serialize)]
struct SettingHead {
    date: String,
    weekday: String,
    time: Option<String>,
    what_for: String,
    court: Option<String>,
    room: Option<String>,
    judge: Option<String>,
    outcome: Option<String>,
    evidence_posture: String,
}

/// One matter under a setting, with its case when it has one.
#[derive(Debug, Clone, Serialize)]
struct MatterDetail {
    matter: MatterProfile,
    evidence: Option<MatterEvidence>,
}

/// The kernel's reading of one matter's case.
#[derive(Debug, Clone, Serialize)]
struct MatterEvidence {
    standing: CaseStanding,
    timeline: Vec<TimelineEntry>,
    witnesses: Vec<WitnessStatement>,
}

/// Joins values with a middle dot, or says nothing is recorded.
fn joined(values: impl Iterator<Item = String>) -> String {
    let collected = values.collect::<Vec<_>>();
    if collected.is_empty() {
        "not recorded".to_owned()
    } else {
        collected.join(" · ")
    }
}

/// Joins the distinct values, in the order they first appear.
fn distinct(values: impl Iterator<Item = String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for value in values {
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    joined(seen.into_iter())
}

#[derive(Debug, Clone, Copy)]
enum ApplyRange {
    One,
    PageToEnd,
    Between(usize, usize),
}

fn value_for_key(field: EnrichmentField, key: &str) -> Option<EnrichmentValue> {
    match (field, key) {
        (EnrichmentField::ContentForm, "u") => {
            Some(EnrichmentValue::ContentForm(ContentForm::RecordedUtterance))
        }
        (EnrichmentField::ContentForm, "a") => {
            Some(EnrichmentValue::ContentForm(ContentForm::AuthoredAssertion))
        }
        (EnrichmentField::ContentForm, "q") => {
            Some(EnrichmentValue::ContentForm(ContentForm::QuotedStatement))
        }
        (EnrichmentField::ContentForm, "r") => {
            Some(EnrichmentValue::ContentForm(ContentForm::ReportedStatement))
        }
        (EnrichmentField::ContentForm, "o") => {
            Some(EnrichmentValue::ContentForm(ContentForm::VisualObservation))
        }
        (EnrichmentField::ContentForm, "m") => {
            Some(EnrichmentValue::ContentForm(ContentForm::MeasuredResult))
        }
        (EnrichmentField::ContentForm, "e") => {
            Some(EnrichmentValue::ContentForm(ContentForm::EvidenceReference))
        }
        (EnrichmentField::ContentForm, "c") => Some(EnrichmentValue::ContentForm(
            ContentForm::OfficialCharacterization,
        )),
        (EnrichmentField::ContentForm, "b") => {
            Some(EnrichmentValue::ContentForm(ContentForm::Boilerplate))
        }
        (EnrichmentField::TemporalStance, "c") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::ContemporaneousCapture,
        )),
        (EnrichmentField::TemporalStance, "a") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::ContemporaneousAccount,
        )),
        (EnrichmentField::TemporalStance, "r") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::RetrospectiveRecollection,
        )),
        (EnrichmentField::TemporalStance, "h") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::ReportOfPriorStatement,
        )),
        (EnrichmentField::TemporalStance, "m") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::LaterMeasurement,
        )),
        (EnrichmentField::TemporalStance, "l") => Some(EnrichmentValue::TemporalStance(
            TemporalStance::LaterAnalysis,
        )),
        (EnrichmentField::TemporalStance, "u") => {
            Some(EnrichmentValue::TemporalStance(TemporalStance::Unknown))
        }
        (EnrichmentField::PerceptionBasis, "s") => {
            Some(EnrichmentValue::PerceptionBasis(PerceptionBasis::Saw))
        }
        (EnrichmentField::PerceptionBasis, "h") => {
            Some(EnrichmentValue::PerceptionBasis(PerceptionBasis::Heard))
        }
        (EnrichmentField::PerceptionBasis, "m") => {
            Some(EnrichmentValue::PerceptionBasis(PerceptionBasis::Measured))
        }
        (EnrichmentField::PerceptionBasis, "r") => {
            Some(EnrichmentValue::PerceptionBasis(PerceptionBasis::Recorded))
        }
        (EnrichmentField::PerceptionBasis, "d") => Some(EnrichmentValue::PerceptionBasis(
            PerceptionBasis::ReadInSource,
        )),
        (EnrichmentField::PerceptionBasis, "t") => Some(EnrichmentValue::PerceptionBasis(
            PerceptionBasis::ToldByPerson,
        )),
        (EnrichmentField::PerceptionBasis, "i") => Some(EnrichmentValue::PerceptionBasis(
            PerceptionBasis::InferredOrCharacterized,
        )),
        (EnrichmentField::PerceptionBasis, "u") => {
            Some(EnrichmentValue::PerceptionBasis(PerceptionBasis::Unknown))
        }
        _ => None,
    }
}

fn apply_value(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
    value: EnrichmentValue,
    range: ApplyRange,
) -> GuiResult<()> {
    let indices = range_indices(session, range)?;
    let mut interpretations = Vec::with_capacity(indices.len());
    for index in &indices {
        let row = session
            .rows
            .get(*index)
            .ok_or_else(|| GuiError::new("Enrichment range left the source."))?;
        let mut proposal = snapshot(row, &session.actor);
        apply_to_snapshot(&mut proposal, &value)?;
        interpretations.push(proposal);
    }
    if interpretations.len() == 1 {
        store.append_interpretation(case_id, &interpretations[0])?;
    } else {
        store.import_interpretations(&InterpretationBatch {
            case_id: case_id.clone(),
            source_profiles: Vec::new(),
            content_groups: Vec::new(),
            interpretations,
        })?;
    }
    session.last_value = Some(value);
    let last = indices.last().copied().unwrap_or(session.cursor);
    refresh_enrichment(store, case_id, session)?;
    session.cursor = last.min(session.rows.len().saturating_sub(1));
    session.advance();
    Ok(())
}

fn range_indices(session: &EnrichmentSession, range: ApplyRange) -> GuiResult<Vec<usize>> {
    if session.rows.is_empty() {
        return Err(GuiError::new("The enrichment source has no passages."));
    }
    match range {
        ApplyRange::One => Ok(vec![session.cursor]),
        ApplyRange::Between(left, right) => {
            let start = left.min(right);
            let end = left.max(right).min(session.rows.len() - 1);
            Ok((start..=end).collect())
        }
        ApplyRange::PageToEnd => {
            let page = session.rows[session.cursor].page;
            let end = session.rows[session.cursor..]
                .iter()
                .position(|row| row.page != page)
                .map_or(session.rows.len(), |offset| session.cursor + offset);
            Ok((session.cursor..end).collect())
        }
    }
}

fn apply_to_snapshot(
    proposal: &mut ProposedInterpretation,
    value: &EnrichmentValue,
) -> GuiResult<()> {
    let (field, provenance) = match value {
        EnrichmentValue::ContentForm(value) => {
            proposal.content_form = Some(*value);
            ("content_form", "entered")
        }
        EnrichmentValue::Speaker(value) => {
            proposal.speaker_entity_id = Some(value.clone());
            ("speaker_entity_id", "entered")
        }
        EnrichmentValue::AttributedPerson(value) => {
            proposal.attributed_entity_id = Some(value.clone());
            ("attributed_entity_id", "entered")
        }
        EnrichmentValue::TemporalStance(value) => {
            proposal.temporal_stance = Some(*value);
            ("temporal_stance", "entered")
        }
        EnrichmentValue::PerceptionBasis(value) => {
            proposal.perception_basis = Some(*value);
            ("perception_basis", "entered")
        }
        EnrichmentValue::Location(value) => {
            if value.trim().is_empty() {
                return Err(GuiError::new("Location text is empty."));
            }
            proposal.location_text = Some(value.trim().to_owned());
            ("location_text", "entered")
        }
        EnrichmentValue::LocationEntity(value) => {
            proposal.location_entity_id = Some(value.clone());
            ("location_entity_id", "entered")
        }
        EnrichmentValue::Materiality(value) => {
            proposal.materiality = *value;
            ("materiality", "entered")
        }
    };
    proposal
        .field_provenance
        .insert(field.to_owned(), provenance.to_owned());
    Ok(())
}

fn snapshot(row: &EnrichmentPassage, actor: &str) -> ProposedInterpretation {
    if let Some(current) = &row.current {
        return proposal_from(current, actor, Some(current.id.clone()));
    }
    ProposedInterpretation {
        id: None,
        target: InterpretationTarget::Content {
            id: row.content_id.clone(),
        },
        char_start: None,
        char_end: None,
        content_form: row.effective.content_form,
        perception_basis: row.effective.perception_basis,
        temporal_stance: row.effective.temporal_stance,
        speaker_entity_id: row.effective.speaker_entity_id.clone(),
        attributed_entity_id: None,
        reporting_parent_interpretation_id: None,
        content_created_at: row.effective.content_created_at.clone(),
        asserted_start: None,
        asserted_end: None,
        normalized_start: None,
        normalized_end: None,
        time_alignment_basis: None,
        location_text: None,
        location_entity_id: None,
        materiality: Materiality::Unknown,
        field_provenance: row.effective.field_provenance.clone(),
        basis: None,
        review_state: ReviewState::Reviewed,
        created_by: actor.to_owned(),
        supersedes_interpretation_id: None,
    }
}

fn proposal_from(
    current: &ContentInterpretation,
    actor: &str,
    supersedes: Option<String>,
) -> ProposedInterpretation {
    ProposedInterpretation {
        id: None,
        target: current.target.clone(),
        char_start: current.char_start,
        char_end: current.char_end,
        content_form: current.content_form,
        perception_basis: current.perception_basis,
        temporal_stance: current.temporal_stance,
        speaker_entity_id: current.speaker_entity_id.clone(),
        attributed_entity_id: current.attributed_entity_id.clone(),
        reporting_parent_interpretation_id: current.reporting_parent_interpretation_id.clone(),
        content_created_at: current.content_created_at.clone(),
        asserted_start: current.asserted_start.clone(),
        asserted_end: current.asserted_end.clone(),
        normalized_start: current.normalized_start.clone(),
        normalized_end: current.normalized_end.clone(),
        time_alignment_basis: current.time_alignment_basis.clone(),
        location_text: current.location_text.clone(),
        location_entity_id: current.location_entity_id.clone(),
        materiality: current.materiality,
        field_provenance: current.field_provenance.clone(),
        basis: current.basis.clone(),
        review_state: ReviewState::Reviewed,
        created_by: actor.to_owned(),
        supersedes_interpretation_id: supersedes,
    }
}

fn accept_candidate(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    let row = session
        .selected()
        .ok_or_else(|| GuiError::new("No passage is selected."))?;
    let candidate = row
        .current
        .as_ref()
        .filter(|item| item.review_state == ReviewState::Suggested)
        .ok_or_else(|| GuiError::new("The selected passage has no candidate to accept."))?;
    let mut proposal = proposal_from(candidate, &session.actor, Some(candidate.id.clone()));
    let accepted = format!(
        "accepted:{}",
        candidate.created_by.trim_start_matches("suggest:")
    );
    for field in populated_fields(candidate) {
        proposal
            .field_provenance
            .insert(field.to_owned(), accepted.clone());
    }
    proposal.basis = Some(format!("Accepted {}", candidate.created_by));
    store.append_interpretation(case_id, &proposal)?;
    refresh_enrichment(store, case_id, session)?;
    session.advance();
    Ok(())
}

fn reject_candidate(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    let row = session
        .selected()
        .ok_or_else(|| GuiError::new("No passage is selected."))?;
    let candidate = row
        .current
        .as_ref()
        .filter(|item| item.review_state == ReviewState::Suggested)
        .ok_or_else(|| GuiError::new("The selected passage has no candidate to reject."))?;
    let mut proposal = proposal_from(candidate, &session.actor, Some(candidate.id.clone()));
    proposal.review_state = ReviewState::Rejected;
    proposal.basis = Some(format!("Rejected {}", candidate.created_by));
    store.append_interpretation(case_id, &proposal)?;
    refresh_enrichment(store, case_id, session)?;
    session.advance();
    Ok(())
}

fn populated_fields(item: &ContentInterpretation) -> Vec<&'static str> {
    let mut fields = Vec::new();
    if item.content_form.is_some() {
        fields.push("content_form");
    }
    if item.perception_basis.is_some() {
        fields.push("perception_basis");
    }
    if item.temporal_stance.is_some() {
        fields.push("temporal_stance");
    }
    if item.speaker_entity_id.is_some() {
        fields.push("speaker_entity_id");
    }
    if item.attributed_entity_id.is_some() {
        fields.push("attributed_entity_id");
    }
    if item.asserted_start.is_some() {
        fields.push("asserted_start");
    }
    if item.normalized_start.is_some() {
        fields.push("normalized_start");
    }
    if item.location_text.is_some() {
        fields.push("location_text");
    }
    fields
}

fn group_with_next(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    let Some(next) = session.cursor.checked_add(1) else {
        return Err(GuiError::new(
            "The selected passage has no following passage.",
        ));
    };
    if next >= session.rows.len() {
        return Err(GuiError::new(
            "The selected passage has no following passage.",
        ));
    }
    store.append_content_group(
        case_id,
        &ProposedContentGroup {
            id: None,
            label: None,
            content_ids: vec![
                session.rows[session.cursor].content_id.clone(),
                session.rows[next].content_id.clone(),
            ],
            review_state: ReviewState::Reviewed,
            created_by: session.actor.clone(),
            supersedes_group_id: None,
        },
    )?;
    Ok(())
}

fn set_nearest_reporting_parent(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    let selected = session
        .selected()
        .ok_or_else(|| GuiError::new("No passage is selected."))?;
    let speaker = selected
        .effective
        .speaker_entity_id
        .as_deref()
        .ok_or_else(|| GuiError::new("Set the passage speaker before its reporting parent."))?;
    let parent_index = (0..session.cursor)
        .rev()
        .find(|index| session.rows[*index].effective.speaker_entity_id.as_deref() == Some(speaker))
        .ok_or_else(|| GuiError::new("No preceding passage has the same author."))?;
    let parent_row = &session.rows[parent_index];
    let mut batch_rows = Vec::new();
    let parent_id = match parent_row.current.as_ref() {
        Some(current)
            if matches!(
                current.review_state,
                ReviewState::Reviewed | ReviewState::Verified
            ) =>
        {
            current.id.clone()
        }
        _ => {
            let id = uuid::Uuid::now_v7().to_string();
            let mut parent = snapshot(parent_row, &session.actor);
            parent.id = Some(id.clone());
            batch_rows.push(parent);
            id
        }
    };
    let mut child = snapshot(selected, &session.actor);
    child.reporting_parent_interpretation_id = Some(parent_id);
    child.field_provenance.insert(
        "reporting_parent_interpretation_id".to_owned(),
        "entered".to_owned(),
    );
    batch_rows.push(child);
    store.import_interpretations(&InterpretationBatch {
        case_id: case_id.clone(),
        source_profiles: Vec::new(),
        content_groups: Vec::new(),
        interpretations: batch_rows,
    })?;
    refresh_enrichment(store, case_id, session)?;
    session.advance();
    Ok(())
}

fn undo_last(
    store: &mut Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    let selected = session
        .selected()
        .ok_or_else(|| GuiError::new("No passage is selected."))?;
    let current = selected
        .current
        .as_ref()
        .ok_or_else(|| GuiError::new("The selected passage has no interpretation to undo."))?;
    let history = store.interpretation_history(
        case_id,
        &current.target,
        current.char_start,
        current.char_end,
    )?;
    let predecessor = history
        .iter()
        .rev()
        .nth(1)
        .ok_or_else(|| GuiError::new("The selected interpretation has no predecessor."))?;
    let mut proposal = proposal_from(predecessor, &session.actor, Some(current.id.clone()));
    proposal.basis = Some(format!("Undo of {}", current.id));
    for provenance in proposal.field_provenance.values_mut() {
        "undo".clone_into(provenance);
    }
    store.append_interpretation(case_id, &proposal)?;
    refresh_enrichment(store, case_id, session)?;
    session.command_mode = false;
    Ok(())
}

fn refresh_enrichment(
    store: &Store,
    case_id: &CaseId,
    session: &mut EnrichmentSession,
) -> GuiResult<()> {
    session.rows = store.enrichment_passages(case_id, &session.source_id)?;
    session.cursor = session.cursor.min(session.rows.len().saturating_sub(1));
    Ok(())
}

fn docket_pairs(store: &Store) -> GuiResult<Vec<(CaseId, String)>> {
    Ok(store
        .cases()?
        .into_iter()
        .map(|summary| (CaseId(summary.id), summary.name))
        .collect())
}

fn json(value: &impl Serialize) -> GuiResult<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

/// Renders one read model as indented text a reader can scan, rather than as
/// serialized JSON. The machine contract is unchanged — the CLI and the saved
/// export files still speak JSON; this is only how a screen presents it.
fn formatted(value: &impl Serialize) -> GuiResult<String> {
    Ok(readable_text(&serde_json::to_value(value)?))
}

/// Turns presentation JSON into indented, human-readable text.
///
/// Keys become sentence-cased labels; values are kept verbatim, because they
/// are the stable database vocabulary and a reader may quote them back into a
/// review decision. Lists render as bullets, nested records indent, and an
/// empty or null slot says `none` rather than disappearing — an absent value
/// is information in this workspace.
pub fn readable_text(value: &serde_json::Value) -> String {
    let mut output = String::new();
    match value {
        serde_json::Value::Array(items) if items.is_empty() => {
            output.push_str("Nothing to show.\n");
        }
        _ => write_readable(&mut output, value, 0, false),
    }
    output
}

fn write_readable(output: &mut String, value: &serde_json::Value, indent: usize, bullet: bool) {
    match value {
        serde_json::Value::Object(map) => {
            let mut first = true;
            for (key, item) in map {
                let pad = readable_pad(indent, bullet && first);
                first = false;
                match item {
                    serde_json::Value::Object(inner) if inner.is_empty() => {
                        let _ = writeln!(output, "{pad}{}: none", readable_key(key));
                    }
                    serde_json::Value::Array(inner) if inner.is_empty() => {
                        let _ = writeln!(output, "{pad}{}: none", readable_key(key));
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        let _ = writeln!(output, "{pad}{}:", readable_key(key));
                        write_readable(output, item, indent + 1, false);
                    }
                    scalar => {
                        let _ = writeln!(
                            output,
                            "{pad}{}: {}",
                            readable_key(key),
                            readable_scalar(scalar)
                        );
                    }
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                match item {
                    serde_json::Value::Object(_) => {
                        write_readable(output, item, indent + 1, true);
                    }
                    serde_json::Value::Array(_) => {
                        let _ = writeln!(output, "{}-", readable_pad(indent, false));
                        write_readable(output, item, indent + 1, false);
                    }
                    scalar => {
                        let _ = writeln!(
                            output,
                            "{}- {}",
                            readable_pad(indent, false),
                            readable_scalar(scalar)
                        );
                    }
                }
            }
        }
        scalar => {
            let _ = writeln!(
                output,
                "{}{}",
                readable_pad(indent, false),
                readable_scalar(scalar)
            );
        }
    }
}

/// A list element's first line carries the bullet in place of two pad spaces,
/// so its fields line up under each other.
fn readable_pad(indent: usize, bullet: bool) -> String {
    if bullet {
        format!("{}- ", "  ".repeat(indent.saturating_sub(1)))
    } else {
        "  ".repeat(indent)
    }
}

fn readable_key(key: &str) -> String {
    let spaced = key.replace('_', " ");
    let mut characters = spaced.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + characters.as_str()
    })
}

fn readable_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "none".to_owned(),
        serde_json::Value::Bool(true) => "yes".to_owned(),
        serde_json::Value::Bool(false) => "no".to_owned(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn collation_text(index: &CollationIndex) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "TIME & PLACE INDEX");
    let _ = writeln!(output, "Case: {}", index.case_id);
    let _ = writeln!(
        output,
        "{} date groups | {} exact-location groups | {} shared-anchor groups",
        index.by_date.len(),
        index.by_location.len(),
        index.shared_anchor_unconfirmed.len()
    );
    let _ = writeln!(
        output,
        "{} passages lack a normalized date | {} lack location\n",
        index.without_normalized_date, index.without_location
    );

    let _ = writeln!(output, "SOURCE ANCHOR COVERAGE");
    for source in &index.source_coverage {
        let _ = writeln!(
            output,
            "- {} [{}] — {} passages; raw {}, created {}, asserted {}, normalized-date {}, location {}",
            source.source,
            source.source_kind,
            source.passages,
            source.with_raw_time,
            source.with_content_created_at,
            source.with_asserted_time,
            source.with_normalized_date,
            source.with_location
        );
    }

    let _ = writeln!(
        output,
        "\nSHARED REVIEWED DATE/LOCATION ANCHOR — RELATIONSHIP NOT ESTABLISHED"
    );
    if index.shared_anchor_unconfirmed.is_empty() {
        let _ = writeln!(output, "- None. No relationship was inferred.");
    }
    for group in &index.shared_anchor_unconfirmed {
        let _ = writeln!(output, "- {}", group.rationale);
        for entry in &group.entries {
            write_collation_entry(&mut output, entry, "  ");
        }
    }

    let _ = writeln!(output, "\nNEEDS PLACEMENT");
    if index.needs_placement.is_empty() {
        let _ = writeln!(output, "- None.");
    }
    for gap in &index.needs_placement {
        let _ = writeln!(
            output,
            "- Missing {}",
            gap.missing_anchors.join(" and ").replace('_', " ")
        );
        write_collation_entry(&mut output, &gap.entry, "  ");
    }

    let _ = writeln!(output, "\nBY NORMALIZED DATE");
    for group in &index.by_date {
        let _ = writeln!(
            output,
            "{} — {} passages across {} original files",
            group.normalized_date.as_deref().unwrap_or("unplaced"),
            group.entries.len(),
            group.distinct_originals
        );
        for entry in &group.entries {
            write_collation_entry(&mut output, entry, "  ");
        }
    }

    let _ = writeln!(output, "\nEXACT LOCATION GROUPS");
    for group in &index.by_location {
        let _ = writeln!(
            output,
            "- {} — {} passages across {} original files",
            group.location.as_deref().unwrap_or("unplaced"),
            group.entries.len(),
            group.distinct_originals
        );
    }
    output
}

fn write_collation_entry(output: &mut String, entry: &CollationEntry, indent: &str) {
    let text = entry.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let _ = writeln!(
        output,
        "{indent}[{}] {} @ {} | {}",
        badge(entry),
        entry.source,
        entry.locator,
        entry.review_state
    );
    for label in time_labels(entry) {
        let _ = writeln!(output, "{indent}{label}");
    }
    let _ = writeln!(output, "{indent}{text}");
}

fn time_labels(entry: &CollationEntry) -> Vec<String> {
    let mut labels = Vec::new();
    if let Some(start) = entry.normalized_start.as_deref() {
        let interval = entry
            .normalized_end
            .as_deref()
            .map_or_else(|| start.to_owned(), |end| format!("{start} – {end}"));
        labels.push(format!("Normalized case time: {interval}"));
        labels.push(format!(
            "Alignment: {}",
            entry.time_basis.as_deref().unwrap_or("basis not recorded")
        ));
    } else {
        labels.push("Alignment: not set".to_owned());
    }
    if let Some(value) = entry.asserted_time.as_deref() {
        labels.push(format!("Alleged event time: {value}"));
    }
    if let Some(value) = entry.content_created_at.as_deref() {
        labels.push(format!("Report created: {value}"));
    }
    if let Some(value) = entry.raw_time.as_deref() {
        labels.push(format!("Recording time: {value}"));
    }
    labels
}

fn badge(entry: &CollationEntry) -> &'static str {
    if entry.machine_generated && entry.review_state == "suggested" {
        return "MACHINE SUGGESTION";
    }
    if entry
        .extractor
        .as_deref()
        .is_some_and(|name| name.contains("whisper") || name.contains("asr"))
    {
        return if entry.review_state == "verified" {
            "HUMAN-VERIFIED TRANSCRIPT"
        } else {
            "RAW TRANSCRIPT"
        };
    }
    if entry.content_kind == "observation"
        && matches!(entry.review_state.as_str(), "reviewed" | "verified")
    {
        return "REVIEWED OBSERVATION";
    }
    if entry.content_kind == "statement" && entry.source_kind == "document" {
        return "DOCUMENT ASSERTION";
    }
    if entry.content_kind == "recording_gap" {
        return "RECORDING LOSS";
    }
    "SOURCE PASSAGE"
}

/// One enrichment grid row, already reduced to what a frontend paints.
///
/// The sweep field decides the value column, which is the whole point of the
/// grid: attention stays on one question down the page instead of on one
/// passage across a form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnrichmentRow {
    /// One-based row number as shown.
    pub number: u32,
    /// Immutable content identifier.
    pub content_id: String,
    /// Exact original locator.
    pub locator: String,
    /// Passage text on a single line.
    pub passage: String,
    /// Current value of the sweep field.
    pub value: String,
    /// Where that value came from.
    pub provenance: String,
    /// Fixed provenance badge for the underlying extraction.
    pub badge: String,
    /// Whether the sweep field still needs a person.
    pub needs: bool,
    /// Whether a deterministic rule is waiting for an answer.
    pub candidate: bool,
}

impl EnrichmentRow {
    fn of(index: usize, row: &EnrichmentPassage, field: EnrichmentField) -> Self {
        Self {
            number: u32::try_from(index + 1).unwrap_or(u32::MAX),
            content_id: row.content_id.clone(),
            locator: row.locator.clone(),
            passage: one_line(&row.text),
            value: field_value(row, field),
            provenance: field_provenance(row, field),
            badge: passage_badge(row).to_owned(),
            needs: row.needs(field),
            candidate: row.has_candidate(),
        }
    }
}

/// Single-key values available in one sweep, for the `?` key sheet.
pub fn key_sheet(field: EnrichmentField) -> Vec<(&'static str, &'static str)> {
    match field {
        EnrichmentField::ContentForm => vec![
            ("u", "recorded utterance"),
            ("a", "authored assertion"),
            ("q", "quoted statement"),
            ("r", "reported statement"),
            ("o", "visual observation"),
            ("m", "measured result"),
            ("e", "evidence reference"),
            ("c", "official characterization"),
            ("b", "boilerplate"),
        ],
        EnrichmentField::TemporalStance => vec![
            ("c", "contemporaneous capture"),
            ("a", "contemporaneous account"),
            ("r", "retrospective recollection"),
            ("h", "report of a prior statement"),
            ("m", "later measurement"),
            ("l", "later analysis"),
            ("u", "unknown"),
        ],
        EnrichmentField::PerceptionBasis => vec![
            ("s", "saw"),
            ("h", "heard"),
            ("m", "measured"),
            ("r", "recorded"),
            ("d", "read in the source"),
            ("t", "told by a person"),
            ("i", "inferred or characterized"),
            ("u", "unknown"),
        ],
        EnrichmentField::Speaker | EnrichmentField::AttributedPerson => {
            vec![("@", "choose or create the person")]
        }
        EnrichmentField::Time => vec![("t", "enter a time, then a basis")],
        EnrichmentField::Location => vec![
            ("t", "type the wording the source used"),
            ("@", "choose a location entity"),
        ],
    }
}

/// Commands that mean the same thing in every sweep.
pub const COMMAND_SHEET: [(&str, &str); 12] = [
    ("j / k", "next / previous passage"),
    ("J / K", "next / previous passage still needing this field"),
    ("F2-F8", "switch the sweep field"),
    ("Enter", "accept the candidate under the cursor"),
    ("x", "reject the candidate, recording the rule as the basis"),
    (".", "repeat the last entered value"),
    ("Shift+letter", "apply the value to the rest of the page"),
    ("v then letter", "apply the value to the marked range"),
    ("s", "split a sub-span out of this passage"),
    ("g", "group this passage with the next"),
    (
        "p",
        "reporting parent = nearest preceding same-author passage",
    ),
    ("n", "boilerplate: leave this passage out of later sweeps"),
];

fn one_line(text: &str) -> String {
    let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= 160 {
        return flattened;
    }
    let mut clipped = flattened.chars().take(157).collect::<String>();
    clipped.push_str("...");
    clipped
}

fn field_value(row: &EnrichmentPassage, field: EnrichmentField) -> String {
    let current = row.current.as_ref();
    match field {
        EnrichmentField::ContentForm => row
            .effective
            .content_form
            .map(|value| value.as_str().to_owned()),
        EnrichmentField::Speaker => row.effective.speaker_entity_id.clone(),
        EnrichmentField::AttributedPerson => {
            current.and_then(|item| item.attributed_entity_id.clone())
        }
        EnrichmentField::TemporalStance => row
            .effective
            .temporal_stance
            .map(|value| value.as_str().to_owned()),
        EnrichmentField::Time => current.and_then(|item| {
            item.asserted_start
                .clone()
                .or_else(|| item.normalized_start.clone())
        }),
        EnrichmentField::Location => current.and_then(|item| {
            item.location_text
                .clone()
                .or_else(|| item.location_entity_id.clone())
        }),
        EnrichmentField::PerceptionBasis => row
            .effective
            .perception_basis
            .map(|value| value.as_str().to_owned()),
    }
    .unwrap_or_else(|| "not set".to_owned())
}

fn field_provenance(row: &EnrichmentPassage, field: EnrichmentField) -> String {
    let key = match field {
        EnrichmentField::ContentForm => "content_form",
        EnrichmentField::Speaker => "speaker_entity_id",
        EnrichmentField::AttributedPerson => "attributed_entity_id",
        EnrichmentField::TemporalStance => "temporal_stance",
        EnrichmentField::Time => "asserted_start",
        EnrichmentField::Location => "location_text",
        EnrichmentField::PerceptionBasis => "perception_basis",
    };
    if row.has_candidate() {
        return row
            .current
            .as_ref()
            .map_or_else(|| "candidate".to_owned(), |item| item.created_by.clone());
    }
    row.effective
        .field_provenance
        .get(key)
        .cloned()
        .unwrap_or_else(|| "-".to_owned())
}

/// The badge an enrichment row carries, in the vocabulary the views use.
fn passage_badge(row: &EnrichmentPassage) -> &'static str {
    if row.has_candidate() {
        return "MACHINE SUGGESTION";
    }
    if row
        .extractor
        .as_deref()
        .is_some_and(|name| name.contains("whisper") || name.contains("asr"))
    {
        return if row.content_review_state == "verified" {
            "HUMAN-VERIFIED TRANSCRIPT"
        } else {
            "RAW TRANSCRIPT"
        };
    }
    match row.effective.content_form {
        Some(ContentForm::ReportedStatement) => "REPORT OF STATEMENT",
        Some(ContentForm::AuthoredAssertion) if row.source_kind == "document" => {
            "DOCUMENT ASSERTION"
        }
        Some(ContentForm::VisualObservation)
            if matches!(row.content_review_state.as_str(), "reviewed" | "verified") =>
        {
            "REVIEWED OBSERVATION"
        }
        _ if row.machine_generated => "MACHINE EXTRACTION",
        _ => "SOURCE PASSAGE",
    }
}

fn packets_text(packets: &[PropositionPacket]) -> String {
    let mut output = String::new();
    if packets.is_empty() {
        return "No contested proposition has a packet yet.\r\n".to_owned();
    }
    for packet in packets {
        let _ = writeln!(output, "{}", packet.proposition);
        let _ = writeln!(output, "  status: {}", packet.status);
        let _ = writeln!(output, "  {}", packet.count);
        for section in &packet.sections {
            let _ = writeln!(output, "\r\n  {} — {}", section.title, section.count);
            for item in &section.items {
                let _ = writeln!(
                    output,
                    "    [{}] {} ({})",
                    item.relation, item.original, item.locator
                );
                let _ = writeln!(output, "      {}", one_line(&item.text));
                let form = item.content_form.as_deref().unwrap_or("form not entered");
                let stance = item
                    .temporal_stance
                    .as_deref()
                    .unwrap_or("stance not entered");
                let _ = writeln!(
                    output,
                    "      {form} · {stance} · passage {} · link {}",
                    item.interpretation_state, item.relation_state
                );
            }
            for note in &section.notes {
                let _ = writeln!(output, "    note: {note}");
            }
        }
        output.push_str("\r\n");
    }
    output
}

fn digest_text(digest: &CaseDigest) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "{} — {}", digest.case_name, digest.audience);
    let _ = writeln!(
        output,
        "{} typed sentence(s) withheld for missing or unreviewed slots.",
        digest.omitted_sentences
    );
    for section in &digest.sections {
        let _ = writeln!(output, "\r\n{}", section.title);
        for sentence in &section.sentences {
            let _ = writeln!(output, "  {}", sentence.text);
            for locator in &sentence.locators {
                let _ = writeln!(output, "    {} ({})", locator.original, locator.locator);
            }
        }
        for line in &section.analysis {
            let _ = writeln!(output, "  {line}");
        }
        for omitted in &section.omitted {
            let _ = writeln!(
                output,
                "  withheld {}: {}",
                omitted.template_id, omitted.reason
            );
        }
        if section.sentences.is_empty() && section.analysis.is_empty() && section.omitted.is_empty()
        {
            let _ = writeln!(output, "  nothing reviewed for this layer yet");
        }
    }
    output
}

/// Parses the stable review target label used by native controls.
pub fn review_target(label: &str) -> Option<ReviewTarget> {
    match label {
        "content" => Some(ReviewTarget::Content),
        "source" => Some(ReviewTarget::Source),
        "edge" => Some(ReviewTarget::Edge),
        "proposition" => Some(ReviewTarget::Proposition),
        "event" => Some(ReviewTarget::Event),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_opens_an_empty_case_without_a_fixture() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        let opened = workspace
            .open_case_json(r#"{"name":"State v. Hall","reference":"PD-2026-0900"}"#)
            .expect("open case");
        assert!(opened.contains("State v. Hall"));
        assert!(opened.contains("Initial production"));
        let overview = workspace.render(WorkspaceView::Overview).expect("overview");
        assert!(overview.contains("State v. Hall"));
        assert!(overview.contains("Productions: 1"));
        assert!(
            !overview.contains('{') && !overview.contains('"'),
            "a read model presents as text, not serialized JSON: {overview}"
        );
    }

    #[test]
    fn read_models_present_as_indented_text_rather_than_json() {
        let value = serde_json::json!({
            "case_id": "case-1",
            "counts": { "sources": 2 },
            "items": [{ "kind": "edge", "flagged": true, "notes": null }],
            "gaps": []
        });
        assert_eq!(
            readable_text(&value),
            "Case id: case-1\nCounts:\n  Sources: 2\nItems:\n  - Kind: edge\n    Flagged: yes\n    \
             Notes: none\nGaps: none\n"
        );
        assert_eq!(readable_text(&serde_json::json!([])), "Nothing to show.\n");
    }

    #[test]
    fn workspace_selects_seeded_case_and_renders_views() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        assert!(workspace.render(WorkspaceView::Overview).is_err());

        let case_id = workspace.seed(DemoFixture::VehicleStop).expect("seed");
        assert_eq!(case_id.0, "case-vehicle-stop-001");
        let overview = workspace.render(WorkspaceView::Overview).expect("overview");
        assert!(overview.contains("case-vehicle-stop-001"));
        assert!(workspace.render(WorkspaceView::ReviewQueue).is_ok());
        let collation = workspace
            .render(WorkspaceView::Collation)
            .expect("collation");
        assert!(collation.starts_with("TIME & PLACE INDEX"));
        assert!(collation.contains("SOURCE ANCHOR COVERAGE"));
        assert!(collation.contains("NEEDS PLACEMENT"));
        assert!(collation.contains("Chen BWC 0042.mp4"));
    }

    #[test]
    fn authoring_panel_writes_a_proposition_through_the_kernel() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed(DemoFixture::VehicleStop).expect("seed");
        let written = workspace
            .author_json(
                AuthorKind::Proposition,
                r#"{"text":"The cruiser clock was wrong","author":"A. Defender"}"#,
            )
            .expect("author proposition");
        assert!(written.contains("The cruiser clock was wrong"));
        assert!(written.contains("unreviewed"));
    }

    #[test]
    fn frontend_labels_round_trip_to_typed_operations() {
        for kind in AuthorKind::ALL {
            assert_eq!(AuthorKind::from_label(kind.label()), Some(kind));
            assert!(serde_json::from_str::<serde_json::Value>(kind.template()).is_ok());
        }
        assert_eq!(review_target("edge"), Some(ReviewTarget::Edge));
        assert_eq!(review_target("advocacy"), None);
    }

    /// The Court pane renders a day with no widgets involved, which is what
    /// makes the docket testable at all. One setting covering three matters is
    /// one row here exactly as it is on screen.
    #[test]
    fn the_court_pane_renders_a_docket_without_a_frontend() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        assert_eq!(workspace.pane(), Pane::Court, "Court opens first");
        assert!(workspace.has_office());
        workspace.seed_office().expect("seed the office caseload");

        // The fixture anchors on today's week, so walk to the Tuesday that
        // carries the consolidated setting rather than assuming a weekday.
        let anchor = CivilDate::parse(workspace.docket_date()).expect("a real day");
        workspace
            .set_docket_date(&anchor.week_start().add_days(1).to_text())
            .expect("tuesday");

        let rows = workspace.docket_rows();
        assert!(!rows.is_empty(), "the seeded week has settings on it");
        let consolidated = rows
            .iter()
            .find(|row| row.client == "Alex Rivera")
            .expect("the client called on three related matters");
        assert_eq!(
            consolidated.matters.matches(" · ").count(),
            2,
            "three court numbers on one row: {}",
            consolidated.matters
        );
        assert!(
            rows.iter()
                .filter(|row| row.client == "Alex Rivera")
                .count()
                == 1,
            "one setting is one row, however many matters it covers"
        );
        assert!(
            !consolidated.copy_text.is_empty(),
            "every row can be lifted without retyping"
        );

        // A row says what its cases rest on, and never how they are going.
        for row in &rows {
            let posture = row.posture.to_lowercase();
            for verdict in ["strong", "weak", "likely", "score", "recommend"] {
                assert!(!posture.contains(verdict), "{}", row.posture);
            }
        }

        let index = rows
            .iter()
            .position(|row| row.client == "Alex Rivera")
            .expect("the row is on the docket");
        let detail = workspace.court_detail(index).expect("the detail");
        assert!(detail.contains("Alex Rivera"));
        assert!(detail.contains("CR-2026-491"), "{detail}");
        for privileged in ["advocacy", "annotation", "decision brief"] {
            assert!(
                !detail.to_lowercase().contains(privileged),
                "the Court detail reads no privileged table, but found {privileged:?}"
            );
        }
    }

    /// Date navigation is arithmetic on a civil date, not a string edit, and a
    /// day that does not exist is refused before it reaches the database.
    #[test]
    fn court_date_navigation_moves_by_days_and_weeks() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed");
        workspace.set_docket_date("2026-02-27").expect("a real day");

        workspace.docket_shift_days(1).expect("forward one day");
        assert_eq!(workspace.docket_date(), "2026-02-28");
        workspace.docket_shift_days(1).expect("into March");
        assert_eq!(
            workspace.docket_date(),
            "2026-03-01",
            "2026 is not a leap year"
        );
        workspace.docket_shift_weeks(-1).expect("back a week");
        assert_eq!(workspace.docket_date(), "2026-02-22");

        let refused = workspace
            .set_docket_date("2026-02-30")
            .expect_err("a day that does not exist is refused");
        assert!(refused.to_string().contains("is not a day"));
        assert_eq!(
            workspace.docket_date(),
            "2026-02-22",
            "and the pane does not move"
        );

        workspace.docket_today().expect("today");
        assert!(CivilDate::parse(workspace.docket_date()).is_some());
    }

    /// The Office pane reaches a case through a matter, and says plainly when
    /// a matter cannot get there.
    #[test]
    fn the_office_pane_reaches_a_case_through_its_matter() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");

        // The fixture's first Rivera matter names the hit-and-run case, which
        // this evidence database does not hold yet. That is a broken link, and
        // it reads differently from a matter that was never linked at all.
        let dangling = workspace
            .select_matter("matter-rivera-1")
            .expect_err("the case is not in this database");
        assert!(dangling.to_string().contains("does not hold"), "{dangling}");

        let never_linked = workspace
            .select_matter("matter-rivera-2")
            .expect_err("this one has no case of its own");
        assert!(
            never_linked.to_string().contains("no evidence case linked"),
            "{never_linked}"
        );

        let case = workspace.seed(DemoFixture::HitAndRun).expect("seed a case");
        let reached = workspace
            .select_matter("matter-rivera-1")
            .expect("the matter reaches its case");
        assert_eq!(reached, case);
        assert_eq!(workspace.active_case().map(|(id, _)| id), Some(&case));

        let matters = workspace.office_matters().expect("matters");
        assert!(
            matters
                .iter()
                .any(|(id, label)| id == "matter-rivera-1" && label.contains("CR-2026-491")),
            "a matter is chosen by the number the court calls it by"
        );
        assert!(
            matters.iter().any(|(_, label)| label.contains("(no case)")),
            "and a matter with no discovery says so in the chooser"
        );
    }

    fn acting(workspace: &mut Workspace) {
        workspace
            .set_acting_user("A. Defender", "attorney")
            .expect("name the acting person");
    }

    fn client_draft(name: &str) -> ClientDraft {
        ClientDraft {
            display_name: name.to_owned(),
            ..ClientDraft::default()
        }
    }

    #[test]
    fn office_writes_refuse_to_be_written_by_nobody() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");

        let error = workspace
            .create_client(&client_draft("Nameless Entry"))
            .expect_err("no acting person has been named");
        assert!(error.to_string().contains("Nobody is acting"), "{error}");
    }

    #[test]
    fn the_acting_person_is_found_once_and_reused() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        let before = workspace.office_users().expect("users").len();

        // The fixture already has A. Defender; naming them twice more must
        // not mint two more user rows.
        acting(&mut workspace);
        acting(&mut workspace);

        assert_eq!(workspace.office_users().expect("users").len(), before);
        assert_eq!(
            workspace
                .acting_user()
                .map(|user| user.display_name.as_str()),
            Some("A. Defender")
        );
    }

    #[test]
    fn a_client_form_records_sex_and_the_language_they_speak() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let rendered = workspace
            .create_client(&ClientDraft {
                sex: Some(office_core::Sex::Female),
                preferred_language: Some("Somali".to_owned()),
                phone: Some("(555) 210-8891".to_owned()),
                ..client_draft("Hodan Warsame")
            })
            .expect("open the client");

        assert!(rendered.contains("female"), "{rendered}");
        assert!(rendered.contains("Somali"), "{rendered}");
        assert!(
            workspace
                .office_languages()
                .expect("languages")
                .first()
                .is_some_and(|first| first == "Spanish" || first == "Somali"),
            "the office's own answers head the chooser"
        );
    }

    #[test]
    fn a_second_client_with_the_same_number_is_offered_as_a_question_not_a_merge() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let draft = ClientDraft {
            phone: Some("555-481-2290".to_owned()),
            ..client_draft("Alexander Rivera")
        };
        let candidates = workspace
            .client_duplicates(&draft.display_name, &draft.contact_values())
            .expect("the advisory check");
        assert!(
            candidates
                .iter()
                .any(|person| person.display_name == "Alex Rivera"),
            "the fixture's Rivera shares that number: {candidates:?}"
        );

        // The question is advisory: the record is still written when a named
        // person decides to write it, and nothing is merged either way.
        let before = workspace.office_clients().expect("clients").len();
        workspace.create_client(&draft).expect("create anyway");
        assert_eq!(
            workspace.office_clients().expect("clients").len(),
            before + 1
        );
    }

    #[test]
    fn a_new_matter_opens_its_evidence_case_in_one_step() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let rendered = workspace
            .open_matter(&MatterDraft {
                client_id: "client-okonkwo".to_owned(),
                caption: "State v. Okonkwo".to_owned(),
                court_number: Some("CR-2026-733".to_owned()),
                court_id: None,
                status: office_core::MatterStatus::Open,
                custody_state: office_core::CustodyState::InCustody,
                offer_state: office_core::OfferState::None,
                charge_summary: Some("Obstruction".to_owned()),
                evidence: EvidenceLink::OpenNewCase,
            })
            .expect("one flow opens both");
        assert!(rendered.contains("State v. Okonkwo"), "{rendered}");

        // The kernel case was prefilled from the matter and selected.
        let (case_id, name) = workspace.active_case().expect("a case is selected");
        assert_eq!(name, "State v. Okonkwo");
        let case_id = case_id.clone();

        // And the matter reaches it: the round trip the flow exists for.
        let matters = workspace.office_matters().expect("matters");
        let (matter_id, label) = matters
            .iter()
            .find(|(_, label)| label.contains("CR-2026-733"))
            .expect("the new matter is on the chooser");
        assert!(!label.contains("(no case)"), "{label}");
        let reached = workspace
            .select_matter(matter_id)
            .expect("the matter reaches the case it opened");
        assert_eq!(reached, case_id);
    }

    #[test]
    fn a_new_matter_can_point_at_a_case_the_kernel_already_holds() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);
        let case = workspace.seed(DemoFixture::HitAndRun).expect("seed a case");
        let cases_before = workspace.cases().len();

        workspace
            .open_matter(&MatterDraft {
                client_id: "client-okonkwo".to_owned(),
                caption: "State v. Okonkwo (refiled)".to_owned(),
                court_number: Some("CR-2026-734".to_owned()),
                court_id: None,
                status: office_core::MatterStatus::Open,
                custody_state: office_core::CustodyState::Unknown,
                offer_state: office_core::OfferState::None,
                charge_summary: None,
                evidence: EvidenceLink::Existing(case.clone()),
            })
            .expect("link the case that exists");

        assert_eq!(
            workspace.cases().len(),
            cases_before,
            "no second case appears"
        );
        let matters = workspace.office_matters().expect("matters");
        let (matter_id, _) = matters
            .iter()
            .find(|(_, label)| label.contains("CR-2026-734"))
            .expect("the new matter is on the chooser");
        assert_eq!(
            workspace.select_matter(matter_id).expect("reaches it"),
            case
        );
    }

    #[test]
    fn a_matter_with_no_case_says_so_rather_than_inventing_one() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);
        let cases_before = workspace.cases().len();

        workspace
            .open_matter(&MatterDraft {
                client_id: "client-okonkwo".to_owned(),
                caption: "State v. Okonkwo (no discovery yet)".to_owned(),
                court_number: Some("CR-2026-735".to_owned()),
                court_id: None,
                status: office_core::MatterStatus::PendingAppointment,
                custody_state: office_core::CustodyState::Unknown,
                offer_state: office_core::OfferState::None,
                charge_summary: None,
                evidence: EvidenceLink::None,
            })
            .expect("open the matter alone");

        assert_eq!(
            workspace.cases().len(),
            cases_before,
            "no case was invented"
        );
        let matters = workspace.office_matters().expect("matters");
        let (matter_id, label) = matters
            .iter()
            .find(|(_, label)| label.contains("CR-2026-735"))
            .expect("the new matter is on the chooser");
        assert!(label.contains("(no case)"), "{label}");
        let error = workspace
            .select_matter(matter_id)
            .expect_err("nothing to reach");
        assert!(
            error.to_string().contains("no evidence case linked"),
            "{error}"
        );
    }

    #[test]
    fn a_setting_covers_every_matter_of_one_client_it_names() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let matters = workspace
            .office_matters_of_client("client-rivera")
            .expect("Rivera's matters");
        assert!(matters.len() >= 2, "{matters:?}");
        let named: Vec<String> = matters.iter().take(2).map(|(id, _)| id.clone()).collect();

        let date = workspace.docket_date().to_owned();
        let rendered = workspace
            .schedule_setting(&SettingDraft {
                client_id: "client-rivera".to_owned(),
                matter_ids: named,
                court_id: None,
                judge: Some("Hon. T. Alvarez".to_owned()),
                date: date.clone(),
                time: Some("09:00".to_owned()),
                appearance_type: office_core::AppearanceType::Status,
                notes: None,
            })
            .expect("schedule the setting");
        assert!(rendered.contains("09:00"), "{rendered}");

        // One row on the docket, both matters on it.
        let row = workspace
            .docket_rows()
            .into_iter()
            .find(|row| row.time.contains("09:00") && row.client.contains("Rivera"))
            .expect("the setting is one docket row");
        assert!(
            row.matters.contains("·") || row.matters.contains("CR-"),
            "{}",
            row.matters
        );
    }

    #[test]
    fn a_setting_refuses_a_matter_belonging_to_somebody_else() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let date = workspace.docket_date().to_owned();
        let error = workspace
            .schedule_setting(&SettingDraft {
                client_id: "client-okonkwo".to_owned(),
                matter_ids: vec!["matter-rivera-1".to_owned()],
                court_id: None,
                judge: None,
                date,
                time: None,
                appearance_type: office_core::AppearanceType::Status,
                notes: None,
            })
            .expect_err("that matter is Rivera's");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn what_is_owed_appears_on_the_day_it_falls() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let matters = workspace
            .office_matters_of_client("client-rivera")
            .expect("Rivera's matters");
        let matter_id = matters
            .first()
            .map(|(id, _)| id.clone())
            .expect("one matter");
        let due = workspace.docket_date().to_owned();

        workspace
            .record_office_deadline(&DeadlineDraft {
                matter_id,
                description: "File the suppression motion".to_owned(),
                due_date: due,
                origin: office_core::DeadlineOrigin::CourtOrdered,
            })
            .expect("record what is owed");

        assert!(
            workspace
                .deadline_rows()
                .iter()
                .any(|row| row.what.contains("suppression")),
            "the deadline is on the Court pane"
        );
    }

    #[test]
    fn a_note_is_filed_under_exactly_one_thing() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        workspace.seed_office().expect("seed the office");
        acting(&mut workspace);

        let rendered = workspace
            .write_office_note(&NoteDraft {
                scope: office_core::NoteScope::Client,
                subject_id: "client-rivera".to_owned(),
                body: "Prefers Spanish for anything technical.".to_owned(),
            })
            .expect("file the note");
        assert!(rendered.contains("Spanish"), "{rendered}");

        let error = workspace
            .write_office_note(&NoteDraft {
                scope: office_core::NoteScope::Matter,
                subject_id: "client-rivera".to_owned(),
                body: "Filed under the wrong thing.".to_owned(),
            })
            .expect_err("a client id is not a matter");
        assert!(!error.to_string().is_empty());
    }
}
