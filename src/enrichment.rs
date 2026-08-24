//! Platform-neutral keyboard enrichment state and preview contracts.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    ContentForm, ContentInterpretation, EffectiveInterpretation, Materiality, PerceptionBasis,
    TemporalStance,
};

/// Semantic column currently being swept down a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentField {
    /// Passage form.
    ContentForm,
    /// Speaker or author.
    Speaker,
    /// Person whose words are being reported.
    AttributedPerson,
    /// Temporal stance.
    TemporalStance,
    /// Asserted or normalized time.
    Time,
    /// Location text/entity.
    Location,
    /// Perception basis.
    PerceptionBasis,
}

/// Value entered by one sweep command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum EnrichmentValue {
    /// Passage form.
    ContentForm(ContentForm),
    /// Speaker entity identifier.
    Speaker(String),
    /// Attributed-person entity identifier.
    AttributedPerson(String),
    /// Temporal stance.
    TemporalStance(TemporalStance),
    /// Perception basis.
    PerceptionBasis(PerceptionBasis),
    /// Location text entered exactly as stated.
    Location(String),
    /// Resolved location entity identifier.
    LocationEntity(String),
    /// Materiality classification.
    Materiality(Materiality),
}

/// Extra input expected after a command such as `@`, `t`, or `s`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingInput {
    /// Entity autocomplete/creation.
    Entity,
    /// Time expression and sticky basis.
    Time,
    /// Character-span boundary.
    Span,
    /// Search text.
    Search,
}

/// One source passage shown by the enrichment grid.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnrichmentPassage {
    /// Immutable content identifier.
    pub content_id: String,
    /// Immutable source identifier.
    pub source_id: String,
    /// Original source display name.
    pub source_name: String,
    /// Broad source modality.
    pub source_kind: String,
    /// Exact original locator.
    pub locator: String,
    /// Extracted text.
    pub text: String,
    /// Document page, when applicable.
    pub page: Option<u32>,
    /// Original-timeline start.
    pub start_ms: Option<u64>,
    /// Original-timeline end.
    pub end_ms: Option<u64>,
    /// Original-page region.
    pub bounding_box: Option<[f64; 4]>,
    /// Whether extraction was machine-generated.
    pub machine_generated: bool,
    /// Extractor name.
    pub extractor: Option<String>,
    /// Intake review state.
    pub content_review_state: String,
    /// Current explicit head, including rejected candidates.
    pub current: Option<ContentInterpretation>,
    /// Current effective reading after source-profile inheritance.
    pub effective: EffectiveInterpretation,
}

impl EnrichmentPassage {
    /// Whether the selected sweep field still needs a human decision.
    pub fn needs(&self, field: EnrichmentField) -> bool {
        match field {
            EnrichmentField::ContentForm => self.effective.content_form.is_none(),
            EnrichmentField::Speaker => self.effective.speaker_entity_id.is_none(),
            EnrichmentField::AttributedPerson => self
                .current
                .as_ref()
                .and_then(|item| item.attributed_entity_id.as_ref())
                .is_none(),
            EnrichmentField::TemporalStance => self.effective.temporal_stance.is_none(),
            EnrichmentField::Time => self.current.as_ref().is_none_or(|item| {
                item.asserted_start.is_none() && item.normalized_start.is_none()
            }),
            EnrichmentField::Location => self.current.as_ref().is_none_or(|item| {
                item.location_text.is_none() && item.location_entity_id.is_none()
            }),
            EnrichmentField::PerceptionBasis => self.effective.perception_basis.is_none(),
        }
    }

    /// Whether a versioned deterministic candidate is the current head.
    pub fn has_candidate(&self) -> bool {
        self.current.as_ref().is_some_and(|item| {
            item.review_state == crate::ReviewState::Suggested
                && item.created_by.starts_with("suggest:")
        })
    }
}

/// One case source as the enrichment picker lists it.
///
/// The picker exists because a sweep is chosen per source: a twenty-page
/// report and a two-hour recording are different pieces of work, and the
/// numbers here are what a person uses to decide which one to open next.
/// Nothing on it ranks a source; the counts say how much of it a person has
/// already read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnrichmentSource {
    /// Immutable source identifier.
    pub source_id: String,
    /// Original file's display name.
    pub name: String,
    /// Broad source modality.
    pub kind: String,
    /// Extracted passages that have not been rejected.
    pub passages: u32,
    /// Passages carrying a reading a person entered.
    pub decided: u32,
    /// Passages whose current reading is a machine candidate awaiting a person.
    pub candidates: u32,
    /// Source-profile role, when a profile has been written.
    pub profile_role: Option<String>,
    /// Profile author's display name, when the profile names one.
    pub profile_author: Option<String>,
    /// Review state of the current profile.
    pub profile_state: Option<String>,
}

impl EnrichmentSource {
    /// Passages with no reading a person has entered.
    pub const fn outstanding(&self) -> u32 {
        self.passages.saturating_sub(self.decided)
    }

    /// Whether a source profile exists to inherit from.
    pub const fn has_profile(&self) -> bool {
        self.profile_role.is_some()
    }
}

/// One entity offered by `@` autocomplete.
///
/// Rule 7 holds here: an offered entity is a name the case already knows, not
/// a claim that this passage is about that person. Selecting one records the
/// reviewer's identification, and `possibly_same_person` remains the way to
/// say two records may be one without merging them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntityCandidate {
    /// Stable entity identifier.
    pub id: String,
    /// Person, organization, object, or location.
    pub kind: String,
    /// Name as the case shows it.
    pub display_name: String,
    /// Anything recorded about the identification itself.
    pub notes: Option<String>,
}

/// Locator-synchronized context displayed beside the grid.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreviewDescriptor {
    /// Rendered document page with the selected region highlighted.
    Document {
        /// Untouched source path, when registered.
        original_path: Option<PathBuf>,
        /// One-indexed page.
        page: u32,
        /// Region in original page coordinates.
        bounding_box: Option<[f64; 4]>,
        /// Retained page-preview image, when available.
        page_image: Option<PathBuf>,
        /// Whether the retained path/hash was available for verification.
        verified_context: bool,
    },
    /// Audio waveform and playable interval.
    Audio {
        /// Untouched source path, when registered.
        original_path: Option<PathBuf>,
        /// Selected original-timeline start.
        start_ms: u64,
        /// Selected original-timeline end.
        end_ms: u64,
        /// Retained waveform peak data, when available.
        waveform: Option<PathBuf>,
        /// Whether the retained path/hash was available for verification.
        verified_context: bool,
    },
    /// Video frame strip and playable interval.
    Video {
        /// Untouched source path, when registered.
        original_path: Option<PathBuf>,
        /// Selected original-timeline start.
        start_ms: u64,
        /// Selected original-timeline end.
        end_ms: u64,
        /// Retained stills around the interval.
        frames: Vec<PathBuf>,
        /// Whether the retained path/hash was available for verification.
        verified_context: bool,
    },
    /// Source context cannot currently be rendered.
    Unavailable {
        /// Exact original locator still shown to the reviewer.
        locator: String,
        /// Why integrated verification is unavailable.
        reason: String,
    },
}

impl PreviewDescriptor {
    /// Whether this preview can support a `verified` decision.
    pub const fn verified_context(&self) -> bool {
        match self {
            Self::Document {
                verified_context, ..
            }
            | Self::Audio {
                verified_context, ..
            }
            | Self::Video {
                verified_context, ..
            } => *verified_context,
            Self::Unavailable { .. } => false,
        }
    }
}

/// Current keyboard session over one source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnrichmentSession {
    /// Source being swept.
    pub source_id: String,
    /// Named person recorded on mutations.
    pub actor: String,
    /// Current sweep column.
    pub field: EnrichmentField,
    /// Passage rows in original order.
    pub rows: Vec<EnrichmentPassage>,
    /// Selected row index.
    pub cursor: usize,
    /// Start of a visual range.
    pub visual_mark: Option<usize>,
    /// Last entered value for repeat.
    pub last_value: Option<EnrichmentValue>,
    /// Alignment basis reused during a time sweep.
    pub sticky_time_basis: Option<String>,
    /// Extra input expected from the frontend.
    pub pending_input: Option<PendingInput>,
    /// Whether single-letter commands address the command grammar instead of a field.
    pub command_mode: bool,
    /// Physical key/confirmation count recorded by the harness.
    pub keystrokes: u32,
}

impl EnrichmentSession {
    /// Selected passage, when the source is non-empty.
    pub fn selected(&self) -> Option<&EnrichmentPassage> {
        self.rows.get(self.cursor)
    }

    /// Moves by one row without wrapping.
    pub fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        self.cursor = self
            .cursor
            .saturating_add_signed(delta)
            .min(self.rows.len() - 1);
    }

    /// Moves to the next/previous candidate or missing value.
    pub fn move_to_work(&mut self, forward: bool) {
        let field = self.field;
        let range: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new((self.cursor + 1)..self.rows.len())
        } else {
            Box::new((0..self.cursor).rev())
        };
        if let Some(index) = range.into_iter().find(|index| {
            let row = &self.rows[*index];
            row.has_candidate() || row.needs(field)
        }) {
            self.cursor = index;
        }
    }

    /// Advances after a semantic decision.
    pub fn advance(&mut self) {
        self.move_by(1);
    }
}

/// Time value submitted from the keyboard time editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeEntry {
    /// Raw accepted expression (`22:42`, full timestamp, or relative form).
    pub value: String,
    /// Whether it is an asserted rather than normalized time.
    #[serde(default)]
    pub asserted: bool,
    /// Written alignment basis; when absent, the session's sticky basis is used.
    #[serde(default)]
    pub basis: Option<String>,
    /// Whether the expression is explicitly approximate.
    #[serde(default)]
    pub approximate: bool,
}
