//! Minimal typed reader for the canonical `lege.document` schema.

use std::path::Path;

use serde::Deserialize;

use crate::{Error, Result};

/// Canonical DocIR schema name accepted by this adapter.
pub const SCHEMA_NAME: &str = "lege.document";
/// Canonical DocIR schema version accepted by this adapter.
pub const SCHEMA_VERSION: u32 = 1;

/// The subset of a canonical Lege document needed for evidence mapping.
#[derive(Debug, Clone, Deserialize)]
pub struct Document {
    /// Schema identity.
    pub schema: SchemaIdentity,
    /// Original source identity recorded by Lege.
    pub source: SourceIdentity,
    /// Ordered zero-indexed pages.
    #[serde(default)]
    pub pages: Vec<Page>,
    /// Processing provenance.
    pub processing: ProcessingManifest,
}

impl Document {
    /// Read and validate one canonical DocIR JSON file.
    pub fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let document: Self = serde_json::from_slice(&bytes)?;
        document.validate()?;
        Ok(document)
    }

    /// Validate the schema and contiguous page numbering used by mapping.
    pub fn validate(&self) -> Result<()> {
        if self.schema.name != SCHEMA_NAME || self.schema.version != SCHEMA_VERSION {
            return Err(Error::InvalidDocIr(format!(
                "expected {SCHEMA_NAME} version {SCHEMA_VERSION}, got {} version {}",
                self.schema.name, self.schema.version
            )));
        }
        for (expected, page) in self.pages.iter().enumerate() {
            if page.index as usize != expected {
                return Err(Error::InvalidDocIr(format!(
                    "expected page index {expected}, got {}",
                    page.index
                )));
            }
        }
        if self.processing.pipeline_version.trim().is_empty()
            || self.processing.configuration_hash.trim().is_empty()
        {
            return Err(Error::InvalidDocIr(
                "processing version and configuration hash are required".to_owned(),
            ));
        }
        Ok(())
    }
}

/// DocIR schema discriminator.
#[derive(Debug, Clone, Deserialize)]
pub struct SchemaIdentity {
    /// Schema name.
    pub name: String,
    /// Schema version.
    pub version: u32,
}

/// Original source identity recorded by the OCR product.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceIdentity {
    /// Source path used during processing.
    pub path: String,
    /// BLAKE3 source fingerprint.
    pub content_hash: String,
    /// Untouched source length.
    pub byte_len: u64,
    /// Source media type.
    #[serde(default)]
    pub mime_type: String,
}

/// Version and configuration of the OCR run.
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessingManifest {
    /// Lege pipeline version.
    pub pipeline_version: String,
    /// Hash of the complete processing configuration.
    pub configuration_hash: String,
}

/// One zero-indexed page.
#[derive(Debug, Clone, Deserialize)]
pub struct Page {
    /// Zero-indexed page number.
    pub index: u32,
    /// Ordered layout/text regions.
    #[serde(default)]
    pub regions: Vec<Region>,
}

/// One region on a page.
#[derive(Debug, Clone, Deserialize)]
pub struct Region {
    /// Stable DocIR region identifier.
    pub id: String,
    /// Region polygon in source page space.
    #[serde(default)]
    pub polygon: Vec<Point>,
    /// Aggregate region confidences.
    #[serde(default)]
    pub confidence: RegionConfidence,
    /// Region payload.
    pub content: RegionContent,
}

/// A page-space point.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

/// Aggregate region confidence.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RegionConfidence {
    /// OCR recognition confidence, when supplied.
    pub recognition: Option<f32>,
}

/// Region content; only raw text blocks become first-slice Evidence records.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum RegionContent {
    /// A block of native or OCR text.
    Text(TextBlock),
    /// Structured table retained only in DocIR for now.
    Table(serde_json::Value),
    /// Formula retained only in DocIR for now.
    Formula(serde_json::Value),
    /// Figure retained only in DocIR for now.
    Figure(serde_json::Value),
    /// Non-text separator.
    Separator,
    /// Unclassified non-text region.
    Unknown,
}

/// Ordered text lines in one region.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TextBlock {
    /// Raw lines.
    #[serde(default)]
    pub lines: Vec<TextLine>,
}

/// One line and its OCR confidence details.
#[derive(Debug, Clone, Deserialize)]
pub struct TextLine {
    /// Raw and derivative text views.
    pub text: TextEvidence,
    /// Token-level confidence summary.
    #[serde(default)]
    pub confidence: RecognitionConfidence,
}

/// Raw text plus derivative alternatives. Mapping intentionally reads only `raw`.
#[derive(Debug, Clone, Deserialize)]
pub struct TextEvidence {
    /// Text as extracted without correction.
    pub raw: String,
}

/// Confidence information retained on a DocIR line.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RecognitionConfidence {
    /// Mean token probability.
    pub mean_token: Option<f32>,
}
