//! Append-only human interpretation inputs and effective read models.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{CaseId, ReviewState};

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        impl $name {
            /// Every variant, in declaration order, for frontends that offer
            /// the whole controlled vocabulary in one control.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// Returns the stable database and wire representation.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }

            /// Parses the stable representation a control round-trips.
            pub fn from_value(value: &str) -> Option<Self> {
                Self::from_db(value)
            }

            pub(crate) fn from_db(value: &str) -> Option<Self> {
                match value { $($value => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

/// The semantic form of one passage, independent of its source modality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentForm {
    /// Spoken words preserved by a recording.
    RecordedUtterance,
    /// A document author's own assertion.
    AuthoredAssertion,
    /// Words presented as a direct quotation.
    QuotedStatement,
    /// One person's account of another person's words.
    ReportedStatement,
    /// A bounded visual observation.
    VisualObservation,
    /// A result produced by measurement.
    MeasuredResult,
    /// A reference to another item of evidence.
    EvidenceReference,
    /// An official or legal characterization rather than a direct observation.
    OfficialCharacterization,
    /// A machine-generated proposal awaiting a person.
    MachineSuggestion,
    /// An interval in which ASR aligned no speech.
    NoSpeechAligned,
    /// Demonstrated loss or absence in a recording.
    RecordingLoss,
    /// Repeated or non-material form language.
    Boilerplate,
}

string_enum!(ContentForm {
    RecordedUtterance => "recorded_utterance",
    AuthoredAssertion => "authored_assertion",
    QuotedStatement => "quoted_statement",
    ReportedStatement => "reported_statement",
    VisualObservation => "visual_observation",
    MeasuredResult => "measured_result",
    EvidenceReference => "evidence_reference",
    OfficialCharacterization => "official_characterization",
    MachineSuggestion => "machine_suggestion",
    NoSpeechAligned => "no_speech_aligned",
    RecordingLoss => "recording_loss",
    Boilerplate => "boilerplate",
});

/// How the speaker or author says they know a passage's contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerceptionBasis {
    /// Personally saw it.
    Saw,
    /// Personally heard it.
    Heard,
    /// Produced it by measurement.
    Measured,
    /// Preserved it by recording.
    Recorded,
    /// Read it in another source.
    ReadInSource,
    /// Was told it by another person.
    ToldByPerson,
    /// Drew or repeated an inference or characterization.
    InferredOrCharacterized,
    /// Basis is not yet known.
    Unknown,
}

string_enum!(PerceptionBasis {
    Saw => "saw",
    Heard => "heard",
    Measured => "measured",
    Recorded => "recorded",
    ReadInSource => "read_in_source",
    ToldByPerson => "told_by_person",
    InferredOrCharacterized => "inferred_or_characterized",
    Unknown => "unknown",
});

/// Relationship between a passage and the occurrence time it discusses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemporalStance {
    /// Direct capture made as the occurrence unfolded.
    ContemporaneousCapture,
    /// A contemporaneous person's account.
    ContemporaneousAccount,
    /// A later recollection.
    RetrospectiveRecollection,
    /// A later report of an earlier statement.
    ReportOfPriorStatement,
    /// A measurement made after the occurrence.
    LaterMeasurement,
    /// Analysis made after the occurrence.
    LaterAnalysis,
    /// Stance is not yet known.
    Unknown,
}

string_enum!(TemporalStance {
    ContemporaneousCapture => "contemporaneous_capture",
    ContemporaneousAccount => "contemporaneous_account",
    RetrospectiveRecollection => "retrospective_recollection",
    ReportOfPriorStatement => "report_of_prior_statement",
    LaterMeasurement => "later_measurement",
    LaterAnalysis => "later_analysis",
    Unknown => "unknown",
});

/// Evidentiary role of an original, separate from document/audio/video modality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    /// Primary police report.
    PoliceReport,
    /// Supplemental police report.
    SupplementalReport,
    /// Witness statement form.
    WitnessStatementForm,
    /// Computer-aided dispatch export.
    CadLog,
    /// Dispatch-channel audio.
    DispatchAudio,
    /// Emergency call recording.
    NineOneOneCall,
    /// Body-worn camera recording.
    BodyCamera,
    /// Surveillance recording.
    SurveillanceVideo,
    /// Recorded interview.
    RecordedInterview,
    /// Jail call.
    JailCall,
    /// Laboratory report.
    LabReport,
    /// Medical record.
    MedicalRecord,
    /// Receipt or transaction record.
    Receipt,
    /// Evidence inventory.
    EvidenceInventory,
    /// Set of photographs.
    PhotographSet,
    /// Transcript derived from another original.
    DerivedTranscript,
    /// A role outside the controlled list.
    Other,
}

string_enum!(SourceRole {
    PoliceReport => "police_report",
    SupplementalReport => "supplemental_report",
    WitnessStatementForm => "witness_statement_form",
    CadLog => "cad_log",
    DispatchAudio => "dispatch_audio",
    NineOneOneCall => "nine_one_one_call",
    BodyCamera => "body_camera",
    SurveillanceVideo => "surveillance_video",
    RecordedInterview => "recorded_interview",
    JailCall => "jail_call",
    LabReport => "lab_report",
    MedicalRecord => "medical_record",
    Receipt => "receipt",
    EvidenceInventory => "evidence_inventory",
    PhotographSet => "photograph_set",
    DerivedTranscript => "derived_transcript",
    Other => "other",
});

/// Whether a passage should participate in later substantive review sweeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Materiality {
    /// Substantive case material.
    Material,
    /// Repeated form or template language.
    Boilerplate,
    /// Administrative material.
    Administrative,
    /// Not yet classified.
    #[default]
    Unknown,
}

string_enum!(Materiality {
    Material => "material",
    Boilerplate => "boilerplate",
    Administrative => "administrative",
    Unknown => "unknown",
});

/// A semantic unit to which an interpretation can attach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InterpretationTarget {
    /// One immutable adapter-produced content row.
    Content {
        /// Immutable content identifier.
        id: String,
    },
    /// A reviewer-defined ordered group of content rows.
    ContentGroup {
        /// Reviewer-defined group identifier.
        id: String,
    },
}

impl InterpretationTarget {
    /// Returns the stable identifier of the target.
    pub fn id(&self) -> &str {
        match self {
            Self::Content { id } | Self::ContentGroup { id } => id,
        }
    }
}

/// One append-only reading of a source as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedSourceProfile {
    /// Stable identifier, generated when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Original source being classified.
    pub source_id: String,
    /// Source's evidentiary role.
    pub source_role: SourceRole,
    /// Default author/speaker entity.
    #[serde(default)]
    pub author_entity_id: Option<String>,
    /// Date or time the source claims it was created.
    #[serde(default)]
    pub created_at_claim: Option<String>,
    /// Inherited passage form.
    #[serde(default)]
    pub default_content_form: Option<ContentForm>,
    /// Inherited temporal stance.
    #[serde(default)]
    pub default_temporal_stance: Option<TemporalStance>,
    /// Inherited perception basis.
    #[serde(default)]
    pub default_perception_basis: Option<PerceptionBasis>,
    /// Device-clock offset from the case clock.
    #[serde(default)]
    pub clock_offset_ms: Option<i64>,
    /// Written basis for a clock offset.
    #[serde(default)]
    pub clock_offset_basis: Option<String>,
    /// Human or candidate review state.
    pub review_state: ReviewState,
    /// Named person or versioned `suggest:` rule.
    pub created_by: String,
    /// Profile version being replaced.
    #[serde(default)]
    pub supersedes_profile_id: Option<String>,
}

/// One append-only semantic interpretation snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedInterpretation {
    /// Stable identifier, generated when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Content or content group being read.
    pub target: InterpretationTarget,
    /// Inclusive UTF-8 byte offset for a sub-span.
    #[serde(default)]
    pub char_start: Option<u32>,
    /// Exclusive UTF-8 byte offset for a sub-span.
    #[serde(default)]
    pub char_end: Option<u32>,
    /// Semantic form.
    #[serde(default)]
    pub content_form: Option<ContentForm>,
    /// Perception basis.
    #[serde(default)]
    pub perception_basis: Option<PerceptionBasis>,
    /// Temporal stance.
    #[serde(default)]
    pub temporal_stance: Option<TemporalStance>,
    /// Speaker or author.
    #[serde(default)]
    pub speaker_entity_id: Option<String>,
    /// Person whose words this passage reports.
    #[serde(default)]
    pub attributed_entity_id: Option<String>,
    /// Interpretation that structurally reports this one.
    #[serde(default)]
    pub reporting_parent_interpretation_id: Option<String>,
    /// When this content was created.
    #[serde(default)]
    pub content_created_at: Option<String>,
    /// Beginning of the occurrence interval asserted by the passage.
    #[serde(default)]
    pub asserted_start: Option<String>,
    /// End of the asserted interval.
    #[serde(default)]
    pub asserted_end: Option<String>,
    /// Beginning of the reviewer-normalized case interval.
    #[serde(default)]
    pub normalized_start: Option<String>,
    /// End of the normalized interval.
    #[serde(default)]
    pub normalized_end: Option<String>,
    /// Written basis for normalization.
    #[serde(default)]
    pub time_alignment_basis: Option<String>,
    /// Location language as entered.
    #[serde(default)]
    pub location_text: Option<String>,
    /// Resolved location entity.
    #[serde(default)]
    pub location_entity_id: Option<String>,
    /// Materiality for sweep navigation.
    #[serde(default)]
    pub materiality: Materiality,
    /// Per-field origin such as `entered` or `accepted:rule@version`.
    #[serde(default)]
    pub field_provenance: BTreeMap<String, String>,
    /// Optional written basis.
    #[serde(default)]
    pub basis: Option<String>,
    /// Current state of this reading.
    pub review_state: ReviewState,
    /// Named person or versioned `suggest:` rule.
    pub created_by: String,
    /// Interpretation version being replaced.
    #[serde(default)]
    pub supersedes_interpretation_id: Option<String>,
}

/// Ordered reviewer-defined semantic unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedContentGroup {
    /// Stable identifier, generated when absent.
    #[serde(default)]
    pub id: Option<String>,
    /// Optional reviewer-facing label.
    #[serde(default)]
    pub label: Option<String>,
    /// Ordered immutable content identifiers.
    pub content_ids: Vec<String>,
    /// State of the group boundary decision.
    pub review_state: ReviewState,
    /// Named person making the grouping decision.
    pub created_by: String,
    /// Group version being replaced.
    #[serde(default)]
    pub supersedes_group_id: Option<String>,
}

/// Atomic human-authored source profiles, groups, and interpretations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterpretationBatch {
    /// Case that owns every referenced record.
    pub case_id: CaseId,
    /// Source-wide defaults.
    #[serde(default)]
    pub source_profiles: Vec<ProposedSourceProfile>,
    /// Reviewer-defined units.
    #[serde(default)]
    pub content_groups: Vec<ProposedContentGroup>,
    /// Passage/group readings.
    #[serde(default)]
    pub interpretations: Vec<ProposedInterpretation>,
}

/// Stored source-profile head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceProfile {
    /// Stable profile-version identifier.
    pub id: String,
    /// Original source.
    pub source_id: String,
    /// Role of the source.
    pub source_role: SourceRole,
    /// Default author/speaker entity.
    pub author_entity_id: Option<String>,
    /// Claimed creation date.
    pub created_at_claim: Option<String>,
    /// Default form.
    pub default_content_form: Option<ContentForm>,
    /// Default stance.
    pub default_temporal_stance: Option<TemporalStance>,
    /// Default basis.
    pub default_perception_basis: Option<PerceptionBasis>,
    /// Clock offset.
    pub clock_offset_ms: Option<i64>,
    /// Basis for the clock offset.
    pub clock_offset_basis: Option<String>,
    /// Review state.
    pub review_state: ReviewState,
    /// Author or rule.
    pub created_by: String,
    /// Replaced version.
    pub supersedes_profile_id: Option<String>,
    /// Storage timestamp.
    pub created_at: String,
}

/// Stored interpretation head or history row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentInterpretation {
    /// Stable interpretation-version identifier.
    pub id: String,
    /// Target semantic unit.
    pub target: InterpretationTarget,
    /// Inclusive sub-span byte offset.
    pub char_start: Option<u32>,
    /// Exclusive sub-span byte offset.
    pub char_end: Option<u32>,
    /// Semantic form.
    pub content_form: Option<ContentForm>,
    /// Perception basis.
    pub perception_basis: Option<PerceptionBasis>,
    /// Temporal stance.
    pub temporal_stance: Option<TemporalStance>,
    /// Speaker/author entity.
    pub speaker_entity_id: Option<String>,
    /// Attributed person entity.
    pub attributed_entity_id: Option<String>,
    /// Reporting-parent interpretation.
    pub reporting_parent_interpretation_id: Option<String>,
    /// Passage creation time.
    pub content_created_at: Option<String>,
    /// Asserted interval start.
    pub asserted_start: Option<String>,
    /// Asserted interval end.
    pub asserted_end: Option<String>,
    /// Normalized interval start.
    pub normalized_start: Option<String>,
    /// Normalized interval end.
    pub normalized_end: Option<String>,
    /// Alignment basis.
    pub time_alignment_basis: Option<String>,
    /// Location text.
    pub location_text: Option<String>,
    /// Location entity.
    pub location_entity_id: Option<String>,
    /// Materiality.
    pub materiality: Materiality,
    /// Per-field provenance.
    pub field_provenance: BTreeMap<String, String>,
    /// Optional basis.
    pub basis: Option<String>,
    /// Review state.
    pub review_state: ReviewState,
    /// Author or rule.
    pub created_by: String,
    /// Replaced version.
    pub supersedes_interpretation_id: Option<String>,
    /// Storage timestamp.
    pub created_at: String,
}

/// Effective reading after source-profile inheritance is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveInterpretation {
    /// Explicit current interpretation, if one exists and is not rejected.
    pub explicit: Option<ContentInterpretation>,
    /// Effective semantic form.
    pub content_form: Option<ContentForm>,
    /// Effective perception basis.
    pub perception_basis: Option<PerceptionBasis>,
    /// Effective temporal stance.
    pub temporal_stance: Option<TemporalStance>,
    /// Effective speaker/author.
    pub speaker_entity_id: Option<String>,
    /// Effective creation time.
    pub content_created_at: Option<String>,
    /// Provenance for each populated effective field.
    pub field_provenance: BTreeMap<String, String>,
}
