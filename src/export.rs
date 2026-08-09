//! Source-linked case exports.
//!
//! An export is the point at which the collation leaves the tool: a chronology
//! for a client meeting, an attachment to a discovery letter, the factual
//! predicate of a motion. Two rules govern it, and they are the reason this is a
//! separate audience-aware read model rather than another view.
//!
//! Every factual line resolves to an exact original locator. A sentence that
//! cannot be traced back to something a reader could open is not exported as a
//! bare assertion; the proposition is reported as unsupported instead.
//!
//! Privileged work product never leaves in a disclosable export. That is
//! enforced structurally — a disclosable export does not read the advocacy,
//! annotation, or brief tables at all — rather than by filtering on a flag that
//! some future writer could set wrongly.

use serde::{Deserialize, Serialize};

use crate::{DiscoveryItem, PropositionEvidence};

/// Who an export is being produced for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportAudience {
    /// Anything that may leave the defense team: an exhibit list, a chronology
    /// attached to a filing, a letter. The privileged tables are never read.
    Disclosable,
    /// The defense team's own complete file, privileged analysis included.
    /// Never produce one of these in response to a discovery obligation.
    WorkFile,
}

impl ExportAudience {
    /// Returns the stable representation used in the export header.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disclosable => "disclosable",
            Self::WorkFile => "work_file",
        }
    }

    /// Returns whether privileged material may be read for this audience.
    pub const fn includes_privileged(self) -> bool {
        matches!(self, Self::WorkFile)
    }
}

/// A case as it leaves the tool, with every factual line traceable.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CaseExport {
    /// Case identifier.
    pub case_id: String,
    /// Human-readable case name.
    pub case_name: String,
    /// Who this export was produced for.
    pub audience: String,
    /// Whether privileged work product is present. False for a disclosable
    /// export, always, and stated so a reader never has to infer it.
    pub includes_privileged: bool,
    /// The production and completeness ledger.
    pub productions: Vec<DiscoveryItem>,
    /// Contested propositions and the exact material bearing on each.
    pub propositions: Vec<ExportedProposition>,
    /// Propositions with no source-grounded evidence, named rather than
    /// exported as assertions nobody can check.
    pub unsupported: Vec<UnsupportedProposition>,
    /// Privileged analysis. Always empty for a disclosable export.
    pub privileged: Vec<ExportedWorkProduct>,
    /// Evidence omitted because a reviewer rejected it, counted so that nothing
    /// leaves the tool silently reduced.
    pub rejected_evidence_omitted: u32,
    /// Exported evidence that no person has reviewed — either the extraction or
    /// the relationship is still in an intake state.
    ///
    /// Every line already carries its own state, but a defender deciding whether
    /// to attach this to a filing should not have to count them. Nothing leaves
    /// silently reduced, and nothing leaves silently unchecked either.
    pub unreviewed_evidence_included: u32,
}

/// One contested proposition and the material bearing on it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExportedProposition {
    /// Stable proposition identifier.
    pub id: String,
    /// The proposition as a person stated it.
    pub text: String,
    /// `contested`, `undisputed`, or `withdrawn`.
    pub status: String,
    /// Human review state of the proposition itself.
    pub review_state: String,
    /// Every item bearing on it, each carrying its own exact locator.
    pub evidence: Vec<PropositionEvidence>,
}

/// A proposition that resolves to no source-grounded evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnsupportedProposition {
    /// Stable proposition identifier.
    pub id: String,
    /// The proposition as a person stated it.
    pub text: String,
    /// Why it is listed here rather than in the body of the export.
    pub reason: String,
}

/// One privileged work-product item, present only in a work-file export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportedWorkProduct {
    /// Stable identifier of the current version.
    pub id: String,
    /// Which kind of work product this is.
    pub kind: String,
    /// Title, or the posture for a brief.
    pub title: String,
    /// The analysis itself.
    pub body: String,
    /// Which version this is.
    pub version: u32,
    /// The named person who wrote this version.
    pub author: String,
}
