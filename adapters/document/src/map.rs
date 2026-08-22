//! Pure DocIR-to-kernel mapping.

use std::path::Path;

use evidence_intake::{
    CaseId, ContentKind, ExtractionProvenance, NormalizedBatch, NormalizedContent,
    NormalizedSegment, NormalizedSource, ReviewState, SourceKind, TemporalRelation,
};
use sha2::{Digest, Sha256};

use crate::docir::{Document, Point, Region, RegionContent, TextBlock};
use crate::{Error, Result};

/// Extractor name stamped on document text.
pub const EXTRACTOR_DOCUMENT: &str = "lege_document_ocr";

/// Caller-controlled identity for one untouched PDF.
#[derive(Debug, Clone)]
pub struct DocumentIdentity {
    /// Existing case.
    pub case_id: CaseId,
    /// Existing production.
    pub production_id: String,
    /// Adapter-assigned source identifier.
    pub source_id: String,
    /// Display name.
    pub logical_name: String,
    /// Whether the document was created during or after the event.
    pub temporal_relation: TemporalRelation,
}

/// Validate `document` against `original` and create a normalized batch.
pub fn document_to_batch(
    identity: &DocumentIdentity,
    original: &Path,
    document: &Document,
) -> Result<NormalizedBatch> {
    document.validate()?;
    let bytes = std::fs::read(original)?;
    validate_source(original, &bytes, document)?;
    let sha256 = hex::encode(Sha256::digest(&bytes));
    let version = format!(
        "{}+{}",
        document.processing.pipeline_version, document.processing.configuration_hash
    );
    let mut segments = Vec::new();
    for page in &document.pages {
        let page_number = page.index.checked_add(1).ok_or_else(|| {
            Error::InvalidDocIr("page index cannot be represented as one-indexed".to_owned())
        })?;
        for (ordinal, region) in page.regions.iter().enumerate() {
            if let Some(segment) =
                map_region(&identity.source_id, page_number, ordinal, region, &version)
            {
                segments.push(segment);
            }
        }
    }
    if segments.is_empty() {
        return Err(Error::NoText);
    }
    let media_type = if document.source.mime_type.trim().is_empty() {
        "application/pdf".to_owned()
    } else {
        document.source.mime_type.clone()
    };
    Ok(NormalizedBatch {
        case_id: identity.case_id.clone(),
        edges: Vec::new(),
        sources: vec![NormalizedSource {
            id: identity.source_id.clone(),
            production_id: identity.production_id.clone(),
            logical_name: identity.logical_name.clone(),
            media_type,
            source_kind: SourceKind::Document,
            temporal_relation: identity.temporal_relation,
            sha256,
            byte_length: bytes.len() as u64,
            segments,
        }],
    })
}

fn validate_source(original: &Path, bytes: &[u8], document: &Document) -> Result<()> {
    if document.source.byte_len != bytes.len() as u64 {
        return Err(Error::SourceMismatch {
            path: original.to_path_buf(),
            reason: format!(
                "DocIR length {} differs from original length {}",
                document.source.byte_len,
                bytes.len()
            ),
        });
    }
    let actual = format!("blake3:{}", blake3::hash(bytes).to_hex());
    if !document.source.content_hash.eq_ignore_ascii_case(&actual) {
        return Err(Error::SourceMismatch {
            path: original.to_path_buf(),
            reason: format!(
                "DocIR hash {} differs from original hash {actual}",
                document.source.content_hash
            ),
        });
    }
    Ok(())
}

fn map_region(
    source_id: &str,
    page_number: u32,
    ordinal: usize,
    region: &Region,
    version: &str,
) -> Option<NormalizedSegment> {
    let RegionContent::Text(block) = &region.content else {
        return None;
    };
    let text = raw_text(block);
    if text.is_empty() {
        return None;
    }
    let region_id = format!("{source_id}-page-{page_number:04}-region-{ordinal:04}");
    let confidence = region
        .confidence
        .recognition
        .or_else(|| line_confidence(block))
        .map(|value| f64::from(value.clamp(0.0, 1.0)));
    Some(NormalizedSegment {
        id: region_id.clone(),
        locator: format!("page {page_number}, region {}", region.id),
        page: Some(page_number),
        start_ms: None,
        end_ms: None,
        bounding_box: bounding_box(&region.polygon),
        content: vec![NormalizedContent {
            id: format!("{region_id}-stmt"),
            kind: ContentKind::Statement,
            text,
            speaker_entity_id: None,
            attributed_to_entity_id: None,
            parent_content_id: None,
            raw_time: None,
            content_created_at: None,
            asserted_time: None,
            normalized_start: None,
            normalized_end: None,
            time_basis: None,
            location_text: None,
            extraction: ExtractionProvenance {
                extractor: EXTRACTOR_DOCUMENT.to_owned(),
                version: version.to_owned(),
                machine_generated: true,
                confidence,
                review_state: ReviewState::Suggested,
            },
        }],
    })
}

fn raw_text(block: &TextBlock) -> String {
    block
        .lines
        .iter()
        .map(|line| line.text.raw.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn line_confidence(block: &TextBlock) -> Option<f32> {
    let values = block
        .lines
        .iter()
        .filter_map(|line| line.confidence.mean_token)
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f32>() / values.len() as f32)
    }
}

fn bounding_box(points: &[Point]) -> Option<[f64; 4]> {
    let finite = points
        .iter()
        .filter(|point| point.x.is_finite() && point.y.is_finite())
        .collect::<Vec<_>>();
    let first = finite.first()?;
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first.x, first.x, first.y, first.y);
    for point in finite.iter().skip(1) {
        min_x = min_x.min(point.x);
        max_x = max_x.max(point.x);
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    if max_x <= min_x || max_y <= min_y {
        return None;
    }
    Some([
        f64::from(min_x),
        f64::from(min_y),
        f64::from(max_x - min_x),
        f64::from(max_y - min_y),
    ])
}
