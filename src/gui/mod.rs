//! Platform-neutral application layer for native evidence workspaces.
//!
//! Frontends own widgets and event loops; this module owns case selection and
//! translates user operations into the kernel's typed API. A Windows frontend
//! is available behind `gui-winsafe`; a future Linux frontend can call the same
//! [`Workspace`] methods without depending on Win32.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{
    CaseId, DemoFixture, ExportAudience, NormalizedBatch, ProposedAdvocacyItem, ProposedAnnotation,
    ProposedBrief, ProposedCharge, ProposedElementMapping, ProposedEntity, ProposedLink,
    ProposedProposition, ReviewDecision, ReviewState, ReviewTarget, Store, SuggestionKind,
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
    /// Privileged issue workspaces.
    Issues,
    /// Charged and alternative offense comparison.
    Offenses,
    /// Records waiting for a named reviewer.
    ReviewQueue,
    /// Append-only review decisions.
    ReviewHistory,
}

impl WorkspaceView {
    /// Stable label used by native navigation controls.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Standing => "Case standing",
            Self::Discovery => "Discovery ledger",
            Self::Elements => "Element matrix",
            Self::Timeline => "Contested timeline",
            Self::Issues => "Issue workspaces",
            Self::Offenses => "Offense comparison",
            Self::ReviewQueue => "Review queue",
            Self::ReviewHistory => "Review history",
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
}

impl AuthorKind {
    /// All authoring kinds in the order shown by native frontends.
    pub const ALL: [Self; 8] = [
        Self::Proposition,
        Self::Link,
        Self::Entity,
        Self::Charge,
        Self::WorkProduct,
        Self::Note,
        Self::Brief,
        Self::ElementMapping,
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
        }
    }
}

/// Platform-neutral state and operations for one database-backed workspace.
pub struct Workspace {
    database: Option<PathBuf>,
    store: Store,
    cases: Vec<(CaseId, String)>,
    active_case: Option<usize>,
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
        let cases = store.cases()?;
        let active_case = (!cases.is_empty()).then_some(0);
        Ok(Self {
            database,
            store,
            cases,
            active_case,
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
        Ok(())
    }

    /// Seeds one curated demonstration case and selects it.
    pub fn seed(&mut self, fixture: DemoFixture) -> GuiResult<CaseId> {
        let id = fixture.seed(&mut self.store)?;
        self.refresh_cases()?;
        self.active_case = self
            .cases
            .iter()
            .position(|(candidate, _)| candidate == &id);
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

    /// Renders one case read model as presentation-ready JSON.
    pub fn render(&self, view: WorkspaceView) -> GuiResult<String> {
        let case_id = self.active_case_id()?;
        match view {
            WorkspaceView::Overview => json(&self.store.overview(case_id)?),
            WorkspaceView::Standing => json(&self.store.case_standing(case_id)?),
            WorkspaceView::Discovery => json(&self.store.discovery_ledger(case_id)?),
            WorkspaceView::Elements => json(&self.store.element_matrix(case_id)?),
            WorkspaceView::Timeline => json(&self.store.contested_timeline(case_id)?),
            WorkspaceView::Issues => json(&self.store.issue_workspaces(case_id)?),
            WorkspaceView::Offenses => json(&self.store.offense_comparison(case_id)?),
            WorkspaceView::ReviewQueue => json(&self.store.review_queue(case_id)?),
            WorkspaceView::ReviewHistory => json(&self.store.review_history(case_id, None)?),
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
        self.cases = self.store.cases()?;
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
                GuiError::new(
                    "No case is selected. Import a normalized batch or seed a demonstration case.",
                )
            })
    }
}

fn json(value: &impl Serialize) -> GuiResult<String> {
    Ok(serde_json::to_string_pretty(value)?)
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
    fn workspace_selects_seeded_case_and_renders_views() {
        let mut workspace = Workspace::in_memory().expect("workspace");
        assert!(workspace.render(WorkspaceView::Overview).is_err());

        let case_id = workspace.seed(DemoFixture::VehicleStop).expect("seed");
        assert_eq!(case_id.0, "case-vehicle-stop-001");
        let overview = workspace.render(WorkspaceView::Overview).expect("overview");
        assert!(overview.contains("case-vehicle-stop-001"));
        assert!(workspace.render(WorkspaceView::ReviewQueue).is_ok());
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
