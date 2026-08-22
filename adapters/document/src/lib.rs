//! PDF document intake through canonical Lege DocIR.
//!
//! The adapter keeps OCR and PDF rendering outside the collation kernel. It
//! invokes the separately packaged Lege product or reads an existing canonical
//! artifact, then maps raw page regions into a [`NormalizedBatch`].

mod backend;
pub mod docir;
mod error;
mod map;

pub use backend::{BrokeredLegeOcrCliBackend, DocumentBackend, LegeOcrCliBackend};
pub use error::{Error, Result};
pub use map::{DocumentIdentity, EXTRACTOR_DOCUMENT, document_to_batch};

use std::path::{Path, PathBuf};

use evidence_intake::{CaseId, NormalizedBatch, TemporalRelation};

/// Inputs for one PDF to one normalized batch.
#[derive(Debug, Clone)]
pub struct DocumentRequest {
    /// Existing case.
    pub case_id: CaseId,
    /// Existing production.
    pub production_id: String,
    /// Adapter-assigned source identifier.
    pub source_id: String,
    /// Untouched PDF.
    pub path: PathBuf,
    /// Display name; file name when omitted.
    pub logical_name: Option<String>,
    /// Whether the document was created during or after the event.
    pub temporal_relation: TemporalRelation,
    /// Durable directory for DocIR and QA artifacts.
    pub artifacts_dir: PathBuf,
}

/// Run a document backend, retain its artifacts, and map raw regions.
pub fn process(
    request: &DocumentRequest,
    backend: &dyn DocumentBackend,
) -> Result<NormalizedBatch> {
    let (_, document) = backend.process(&request.path, &request.artifacts_dir)?;
    map_document(request, &document)
}

/// Map an existing canonical DocIR artifact without running OCR.
pub fn from_docir(request: &DocumentRequest, docir_path: &Path) -> Result<NormalizedBatch> {
    let document = docir::Document::read(docir_path)?;
    map_document(request, &document)
}

fn map_document(request: &DocumentRequest, document: &docir::Document) -> Result<NormalizedBatch> {
    let logical_name = request.logical_name.clone().unwrap_or_else(|| {
        request
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("document.pdf")
            .to_owned()
    });
    document_to_batch(
        &DocumentIdentity {
            case_id: request.case_id.clone(),
            production_id: request.production_id.clone(),
            source_id: request.source_id.clone(),
            logical_name,
            temporal_relation: request.temporal_relation,
        },
        &request.path,
        document,
    )
}
