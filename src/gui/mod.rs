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
use serde::Serialize;

use crate::{
    AuthoredEntity, CaseDigest, CaseId, CollationEntry, CollationIndex, ContentForm,
    ContentInterpretation, DemoFixture, EnrichmentField, EnrichmentPassage, EnrichmentSession,
    EnrichmentSource, EnrichmentValue, EntityCandidate, EntityKind, ExportAudience, IntakeArtifact,
    IntakeJob, InterpretationBatch, InterpretationTarget, KeyframeHit, Materiality, NewIntakeJob,
    NodeKind, NodeRef, NormalizedBatch, OpenedProduction, PendingInput, PerceptionBasis,
    PreviewDescriptor, ProposedAdvocacyItem, ProposedAnnotation, ProposedBrief, ProposedCase,
    ProposedCharge, ProposedContentGroup, ProposedElementMapping, ProposedEntity,
    ProposedInterpretation, ProposedLink, ProposedOccurrence, ProposedProduction,
    ProposedProposition, ProposedSourceProfile, PropositionPacket, ReviewDecision, ReviewState,
    ReviewTarget, SourceLocation, SourceProfile, Store, SuggestionKind, TemporalStance, TimeEntry,
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
pub struct Workspace {
    database: Option<PathBuf>,
    store: Store,
    cases: Vec<(CaseId, String)>,
    active_case: Option<usize>,
    enrichment: Option<EnrichmentSession>,
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
        Ok(Self {
            database,
            store,
            cases,
            active_case,
            enrichment: None,
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
        json(&opened)
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

    /// Renders one case read model as presentation-ready JSON.
    pub fn render(&self, view: WorkspaceView) -> GuiResult<String> {
        let case_id = self.active_case_id()?;
        match view {
            WorkspaceView::Overview => json(&self.store.overview(case_id)?),
            WorkspaceView::Standing => json(&self.store.case_standing(case_id)?),
            WorkspaceView::Discovery => json(&self.store.discovery_ledger(case_id)?),
            WorkspaceView::Elements => json(&self.store.element_matrix(case_id)?),
            WorkspaceView::Timeline => json(&self.store.contested_timeline(case_id)?),
            WorkspaceView::Collation => Ok(collation_text(&self.store.collation_index(case_id)?)),
            WorkspaceView::Issues => json(&self.store.issue_workspaces(case_id)?),
            WorkspaceView::Offenses => json(&self.store.offense_comparison(case_id)?),
            WorkspaceView::ReviewQueue => json(&self.store.review_queue(case_id)?),
            WorkspaceView::ReviewHistory => json(&self.store.review_history(case_id, None)?),
            WorkspaceView::Packets => Ok(packets_text(&self.store.proposition_packets(case_id)?)),
            WorkspaceView::Digest => Ok(digest_text(
                &self.store.case_digest(case_id, ExportAudience::WorkFile)?,
            )),
        }
    }

    /// Searches source-grounded case content.
    pub fn search(&self, query: &str, limit: u32) -> GuiResult<String> {
        json(&self.store.search(self.active_case_id()?, query, limit)?)
    }

    /// Runs every deterministic analyzer and returns proposals plus findings.
    pub fn suggest_all(&mut self) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        json(&self.store.suggest(&case_id, &SuggestionKind::ALL)?)
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
        json(&self.store.apply_review(&case_id, &decision)?)
    }

    /// Parses a typed authoring payload and writes it through the kernel API.
    pub fn author_json(&mut self, kind: AuthorKind, payload: &str) -> GuiResult<String> {
        let case_id = self.active_case_id()?.clone();
        match kind {
            AuthorKind::Proposition => {
                let value: ProposedProposition = serde_json::from_str(payload)?;
                json(&self.store.author_proposition(&case_id, &value)?)
            }
            AuthorKind::Link => {
                let value: ProposedLink = serde_json::from_str(payload)?;
                json(&self.store.link_evidence(&case_id, &value)?)
            }
            AuthorKind::Entity => {
                let value: ProposedEntity = serde_json::from_str(payload)?;
                json(&self.store.record_entity(&case_id, &value)?)
            }
            AuthorKind::Charge => {
                let value: ProposedCharge = serde_json::from_str(payload)?;
                json(&self.store.record_charge(&case_id, &value)?)
            }
            AuthorKind::WorkProduct => {
                let value: ProposedAdvocacyItem = serde_json::from_str(payload)?;
                json(&self.store.author_advocacy_item(&case_id, &value)?)
            }
            AuthorKind::Note => {
                let value: ProposedAnnotation = serde_json::from_str(payload)?;
                json(&self.store.annotate(&case_id, &value)?)
            }
            AuthorKind::Brief => {
                let value: ProposedBrief = serde_json::from_str(payload)?;
                json(&self.store.record_brief(&case_id, &value)?)
            }
            AuthorKind::ElementMapping => {
                let value: ProposedElementMapping = serde_json::from_str(payload)?;
                json(&self.store.map_element(&case_id, &value)?)
            }
            AuthorKind::Occurrence => {
                let value: ProposedOccurrence = serde_json::from_str(payload)?;
                json(&self.store.author_occurrence(&case_id, &value)?)
            }
        }
    }

    /// Produces a source-linked export as JSON.
    pub fn export(&self, audience: ExportAudience) -> GuiResult<String> {
        json(&self.store.export_case(self.active_case_id()?, audience)?)
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
        assert!(overview.contains("\"productions\": 1"));
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
}
