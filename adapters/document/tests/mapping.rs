#![allow(missing_docs)]

use std::fs;

use evidence_document::{DocumentRequest, Error, from_docir};
#[cfg(unix)]
use evidence_document::{LegeOcrCliBackend, process};
use evidence_intake::{
    CaseId, ContentKind, EdgeKind, ExportAudience, NodeKind, NodeRef, ProposedCase, ProposedLink,
    ProposedProposition, ReviewDecision, ReviewState, ReviewTarget, SourceKind, Store,
    TemporalRelation,
};
use serde_json::json;
use tempfile::TempDir;

fn fixture() -> (TempDir, DocumentRequest, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("report.pdf");
    fs::write(&original, b"untouched-pdf").unwrap();
    let hash = format!("blake3:{}", blake3::hash(b"untouched-pdf").to_hex());
    let docir = directory.path().join("report.lege.json");
    fs::write(
        &docir,
        serde_json::to_vec_pretty(&json!({
            "schema": {"name": "lege.document", "version": 1},
            "id": "doc-1",
            "source": {
                "path": original.to_string_lossy(),
                "content_hash": hash,
                "byte_len": 13,
                "mime_type": "application/pdf"
            },
            "processing": {
                "pipeline_version": "0.1.0",
                "profile": "search",
                "quality": "thorough",
                "configuration_hash": "blake3:config",
                "models": [],
                "warnings": []
            },
            "pages": [{
                "index": 0,
                "source_size": {"width": 100, "height": 200},
                "page_size_points": {"width": 50.0, "height": 100.0},
                "source_to_page": {"matrix": [1.0,0.0,0.0,1.0,0.0,0.0]},
                "source_kind": "scanned-image",
                "regions": [{
                    "id": "p000001-r0001",
                    "kind": "paragraph",
                    "polygon": [{"x": 10.0,"y": 20.0},{"x": 90.0,"y": 20.0},{"x": 90.0,"y": 60.0},{"x": 10.0,"y": 60.0}],
                    "confidence": {"recognition": 0.875},
                    "content": {"type": "text", "value": {"lines": [
                        {"text": {"raw": "Raw OCR", "normalized": "Normalized", "corrected": "Corrected", "alternatives": [], "corrections": []}, "polygon": [], "confidence": {"mean_token": 0.8}, "words": [], "provenance": {"provider":"paddle","model":null,"preprocessing":null,"language":"eng"}},
                        {"text": {"raw": "second line", "normalized": null, "corrected": null, "alternatives": [], "corrections": []}, "polygon": [], "confidence": {"mean_token": 0.9}, "words": [], "provenance": {"provider":"paddle","model":null,"preprocessing":null,"language":"eng"}}
                    ]}},
                    "provenance": {"provider":"paddle","model":null,"preprocessing":null,"language":"eng"}
                },{
                    "id": "ignored-table",
                    "kind": "table",
                    "polygon": [],
                    "confidence": {},
                    "content": {"type": "table", "value": {"rows": 0, "columns": 0, "cells": []}},
                    "provenance": {"provider":"layout","model":null,"preprocessing":null,"language":null}
                }],
                "reading_order": ["p000001-r0001"],
                "warnings": []
            }],
            "outline": []
        }))
        .unwrap(),
    )
    .unwrap();
    let request = DocumentRequest {
        case_id: CaseId("case-1".to_owned()),
        production_id: "production-1".to_owned(),
        source_id: "source-report".to_owned(),
        path: original,
        logical_name: None,
        temporal_relation: TemporalRelation::AfterEvent,
        artifacts_dir: directory.path().join("artifacts"),
    };
    (directory, request, docir)
}

fn expand_to_ten_pages(docir: &std::path::Path) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(docir).unwrap()).unwrap();
    let template = value["pages"][0].clone();
    let mut pages = Vec::new();
    for index in 0..10_u32 {
        let mut page = template.clone();
        page["index"] = json!(index);
        if index == 0 {
            pages.push(page);
            continue;
        }
        if matches!(index, 1 | 9) {
            let mut region = template["regions"][0].clone();
            let page_number = index + 1;
            let region_id = format!("p{page_number:06}-r0001");
            let page_word = if page_number == 2 { "two" } else { "ten" };
            region["id"] = json!(region_id);
            region["content"]["value"]["lines"][0]["text"]["raw"] =
                json!(format!("Synthetic page {page_word} observation"));
            region["content"]["value"]["lines"][1]["text"]["raw"] = json!("");
            page["regions"] = json!([region]);
            page["reading_order"] = json!([region_id]);
        } else {
            page["regions"] = json!([]);
            page["reading_order"] = json!([]);
        }
        pages.push(page);
    }
    value["pages"] = json!(pages);
    fs::write(docir, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn maps_raw_regions_with_page_geometry_and_suggested_provenance() {
    let (_directory, request, docir) = fixture();
    let batch = from_docir(&request, &docir).unwrap();
    assert_eq!(batch.case_id, CaseId("case-1".to_owned()));
    assert!(batch.edges.is_empty());
    let source = &batch.sources[0];
    assert_eq!(source.source_kind, SourceKind::Document);
    assert_eq!(source.logical_name, "report.pdf");
    assert_eq!(source.byte_length, 13);
    assert_eq!(source.sha256.len(), 64);
    assert_eq!(source.segments.len(), 1);
    let segment = &source.segments[0];
    assert_eq!(segment.page, Some(1));
    assert_eq!(segment.locator, "page 1, region p000001-r0001");
    assert_eq!(segment.bounding_box, Some([10.0, 20.0, 80.0, 40.0]));
    let content = &segment.content[0];
    assert_eq!(content.kind, ContentKind::Statement);
    assert_eq!(content.text, "Raw OCR\nsecond line");
    assert_eq!(content.extraction.review_state, ReviewState::Suggested);
    assert!(content.extraction.machine_generated);
    assert_eq!(content.extraction.confidence, Some(0.875));
    assert_eq!(content.extraction.version, "0.1.0+blake3:config");
}

/// A deliberately fictional document exercises the complete adapter-to-core
/// seam without introducing privileged case material into the test suite.
#[test]
fn synthetic_document_collates_reviews_and_exports_in_page_order() {
    let (_directory, mut request, docir) = fixture();
    expand_to_ten_pages(&docir);

    let mut store = Store::in_memory().unwrap();
    let opened = store
        .open_case(&ProposedCase {
            id: Some(request.case_id.0.clone()),
            name: "Synthetic document integration matter".to_owned(),
            reference: Some("SYNTHETIC-ONLY".to_owned()),
            jurisdiction: None,
            production: Some("Synthetic production".to_owned()),
        })
        .unwrap();
    request.production_id = opened.production.id;

    let batch = from_docir(&request, &docir).unwrap();
    store.import_normalized(&batch).unwrap();

    let overview = store.overview(&request.case_id).unwrap();
    assert_eq!(overview.sources, 1);
    assert_eq!(
        overview.pending_review, 4,
        "one source and three OCR passages"
    );

    let ledger = store.discovery_ledger(&request.case_id).unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].source_kind, "document");
    assert_eq!(ledger[0].media_type, "application/pdf");

    let hits = store
        .search(&request.case_id, "synthetic AND page AND two", 10)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].locator, "page 2, region p000002-r0001");
    assert_eq!(hits[0].review_state, "suggested");
    assert!(hits[0].machine_generated);

    let index = store.collation_index(&request.case_id).unwrap();
    assert!(index.by_date.is_empty());
    assert!(index.by_location.is_empty());
    assert!(index.possibly_related.is_empty());
    assert_eq!(index.without_normalized_date, 3);
    assert_eq!(index.without_location, 3);
    assert_eq!(
        index
            .needs_placement
            .iter()
            .map(|gap| gap.entry.locator.as_str())
            .collect::<Vec<_>>(),
        [
            "page 1, region p000001-r0001",
            "page 2, region p000002-r0001",
            "page 10, region p000010-r0001",
        ],
        "structured page numbers, not locator strings, determine document order"
    );
    let coverage = &index.source_coverage[0];
    assert_eq!(coverage.source_kind, "document");
    assert_eq!(coverage.passages, 3);
    assert_eq!(coverage.with_raw_time, 0);
    assert_eq!(coverage.with_normalized_date, 0);
    assert_eq!(coverage.with_location, 0);

    let page_two_id = "source-report-page-0002-region-0000-stmt";
    let page_two_queue_item = store
        .review_queue(&request.case_id)
        .unwrap()
        .into_iter()
        .find(|item| item.target_id == page_two_id)
        .unwrap();
    let page_two_locator = page_two_queue_item.locator.unwrap();
    assert_eq!(
        page_two_locator,
        "report.pdf @ page 2, region p000002-r0001"
    );
    assert_eq!(
        page_two_queue_item.extractor.as_deref(),
        Some("lege_document_ocr")
    );
    store
        .apply_review(
            &request.case_id,
            &ReviewDecision {
                target: ReviewTarget::Content,
                target_id: page_two_id.to_owned(),
                to_state: ReviewState::Verified,
                actor: "Synthetic Reviewer".to_owned(),
                basis: None,
                verified_against_locator: Some(page_two_locator),
            },
        )
        .unwrap();

    let proposition = store
        .author_proposition(
            &request.case_id,
            &ProposedProposition {
                id: None,
                text: "The fictional report contains two selected observations.".to_owned(),
                author: "Synthetic Reviewer".to_owned(),
            },
        )
        .unwrap();
    for content_id in ["source-report-page-0010-region-0000-stmt", page_two_id] {
        store
            .link_evidence(
                &request.case_id,
                &ProposedLink {
                    id: None,
                    from: NodeRef::new(NodeKind::Content, content_id),
                    relation: EdgeKind::Supports,
                    to: NodeRef::new(NodeKind::Proposition, &proposition.id),
                    rationale: "Selected only for the synthetic integration test.".to_owned(),
                    author: "Synthetic Reviewer".to_owned(),
                },
            )
            .unwrap();
    }

    let evidence = store
        .proposition_evidence(&request.case_id, &proposition.id)
        .unwrap();
    assert_eq!(
        evidence
            .iter()
            .map(|item| item.locator.as_str())
            .collect::<Vec<_>>(),
        [
            "page 2, region p000002-r0001",
            "page 10, region p000010-r0001",
        ]
    );
    assert_eq!(evidence[0].review_state, "verified");
    assert_eq!(evidence[1].review_state, "suggested");

    let export = store
        .export_case(&request.case_id, ExportAudience::Disclosable)
        .unwrap();
    assert_eq!(export.propositions.len(), 1);
    assert_eq!(export.propositions[0].evidence, evidence);
    assert_eq!(export.unreviewed_evidence_included, 2);
}

#[test]
fn rejects_docir_for_different_original_bytes() {
    let (_directory, request, docir) = fixture();
    fs::write(&request.path, b"different").unwrap();
    assert!(matches!(
        from_docir(&request, &docir),
        Err(Error::SourceMismatch { .. })
    ));
}

#[test]
fn rejects_unknown_schema_versions() {
    let (_directory, request, docir) = fixture();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&docir).unwrap()).unwrap();
    value["schema"]["version"] = json!(2);
    fs::write(&docir, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        from_docir(&request, &docir),
        Err(Error::InvalidDocIr(_))
    ));
}

#[cfg(unix)]
#[test]
fn packaged_cli_artifact_is_retained_and_mapped() {
    use std::os::unix::fs::PermissionsExt;

    let (_directory, request, docir) = fixture();
    let script = request.path.parent().unwrap().join("mock-lege-ocr.sh");
    let body = format!(
        "#!/bin/sh\nset -eu\n[ \"$1\" = batch ]\n[ \"$3\" = --output ]\nmkdir -p \"$4/run\"\ncp \"{}\" \"$4/run/report.lege.json\"\n",
        docir.display()
    );
    fs::write(&script, body).unwrap();
    let mut permissions = fs::metadata(&script).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).unwrap();

    let batch = process(
        &request,
        &LegeOcrCliBackend {
            bin: script,
            tensorrt_root: None,
        },
    )
    .unwrap();
    assert_eq!(batch.sources[0].segments.len(), 1);
    assert!(request.artifacts_dir.join("run/report.lege.json").is_file());
}
