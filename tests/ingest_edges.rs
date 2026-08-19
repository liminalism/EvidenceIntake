//! An adapter may propose how records sit relative to each other, never what
//! they establish.

#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ContentKind, EdgeKind, ExtractionProvenance, NodeKind, NormalizedBatch,
    NormalizedContent, NormalizedEdge, NormalizedSegment, NormalizedSource, ProposedCase,
    ReviewQueueItem, ReviewState, SourceKind, Store, TemporalRelation,
};

fn empty_store() -> Store {
    Store::in_memory().expect("in-memory store")
}

/// Opens a bare case with one production, so intake has a ledger to attach to
/// and the source counts stay predictable.
fn open_case(store: &mut Store, id: &str) -> (CaseId, String) {
    let opened = store
        .open_case(&ProposedCase {
            id: Some(id.to_owned()),
            name: format!("State v. {id}"),
            reference: None,
            jurisdiction: None,
            production: Some("Initial production".to_owned()),
        })
        .expect("open case");
    (CaseId(opened.id.clone()), opened.production.id)
}

fn machine() -> ExtractionProvenance {
    ExtractionProvenance {
        extractor: "camera_sync".to_owned(),
        version: "2.1.0".to_owned(),
        machine_generated: true,
        confidence: Some(0.9),
        review_state: ReviewState::Suggested,
    }
}

/// One source carrying one segment and one bounded observation, so a test has
/// both a source and a content endpoint to relate.
fn source(id: &str, production_id: &str, digit: char, kind: SourceKind) -> NormalizedSource {
    NormalizedSource {
        id: id.to_owned(),
        production_id: production_id.to_owned(),
        logical_name: format!("{id}.mp4"),
        media_type: "video/mp4".to_owned(),
        source_kind: kind,
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: digit.to_string().repeat(64),
        byte_length: 4_096,
        segments: vec![NormalizedSegment {
            id: format!("{id}-seg"),
            locator: "00:00:00-00:00:10".to_owned(),
            page: None,
            start_ms: Some(0),
            end_ms: Some(10_000),
            bounding_box: None,
            content: vec![NormalizedContent {
                id: format!("{id}-obs"),
                kind: ContentKind::Observation,
                text: "A bounded machine observation.".to_owned(),
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
                extraction: machine(),
            }],
        }],
    }
}

fn edge(
    id: &str,
    from: (NodeKind, &str),
    relation: EdgeKind,
    to: (NodeKind, &str),
) -> NormalizedEdge {
    NormalizedEdge {
        id: id.to_owned(),
        from_kind: from.0,
        from_id: from.1.to_owned(),
        relation,
        to_kind: to.0,
        to_id: to.1.to_owned(),
        rationale: "Audio cross-correlation puts camera B 1.30 s ahead of camera A.".to_owned(),
        extraction: machine(),
    }
}

fn batch(
    case_id: &CaseId,
    sources: Vec<NormalizedSource>,
    edges: Vec<NormalizedEdge>,
) -> NormalizedBatch {
    NormalizedBatch {
        case_id: case_id.clone(),
        sources,
        edges,
    }
}

fn queued(store: &Store, case_id: &CaseId, target_id: &str) -> ReviewQueueItem {
    store
        .review_queue(case_id)
        .expect("review queue")
        .into_iter()
        .find(|item| item.target_id == target_id)
        .expect("the record waits in the review queue")
}

fn sources_held(store: &Store, case_id: &CaseId) -> u32 {
    store.overview(case_id).expect("overview").sources
}

/// The point of the contract: a second pass can relate two originals that were
/// imported on different days, without re-delivering either of them.
#[test]
fn an_adapter_may_relate_two_sources_it_did_not_bring() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-relate");

    for (id, digit) in [("src-cam-a", '1'), ("src-cam-b", '2')] {
        store
            .import_normalized(&batch(
                &case_id,
                vec![source(id, &production, digit, SourceKind::Video)],
                Vec::new(),
            ))
            .expect("import one original");
    }

    store
        .import_normalized(&batch(
            &case_id,
            Vec::new(),
            vec![edge(
                "edge-cam-overlap",
                (NodeKind::Source, "src-cam-a"),
                EdgeKind::TemporallyOverlaps,
                (NodeKind::Source, "src-cam-b"),
            )],
        ))
        .expect("a batch of relationships alone must import");

    let item = queued(&store, &case_id, "edge-cam-overlap");
    assert_eq!(item.target_kind, "edge");
    assert_eq!(item.review_state, "suggested");
    assert!(
        item.machine_generated,
        "an adapter's proposal must be visibly machine-generated"
    );
    let extractor = item
        .extractor
        .clone()
        .expect("the queue names what proposed it");
    assert!(
        extractor.starts_with("suggest:"),
        "an adapter proposal is attributed like an analyzer's: {extractor}"
    );
    assert_eq!(extractor, "suggest:camera_sync@2.1.0");
    assert!(item.summary.contains("temporally_overlaps"), "{item:?}");
}

/// Endpoints are resolved after the batch's own rows are in, so a delivery can
/// describe its own internal structure.
#[test]
fn a_batch_may_relate_the_stills_it_brings() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-stills");

    store
        .import_normalized(&batch(
            &case_id,
            vec![
                source("src-body-cam", &production, '3', SourceKind::Video),
                source("src-still-14", &production, '4', SourceKind::ImageSet),
            ],
            vec![edge(
                "edge-still-from-video",
                (NodeKind::Source, "src-still-14"),
                EdgeKind::DerivedFrom,
                (NodeKind::Source, "src-body-cam"),
            )],
        ))
        .expect("a batch may relate what it brings");

    let item = queued(&store, &case_id, "edge-still-from-video");
    assert_eq!(item.review_state, "suggested");
    assert!(item.summary.contains("derived_from"), "{item:?}");
}

/// An adapter may report structure. Whether one record supports or contradicts
/// another is a person's judgment, and import will not take it.
#[test]
fn an_adapter_edge_cannot_be_evaluative() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-evaluative");
    store
        .import_normalized(&batch(
            &case_id,
            vec![
                source("src-e-a", &production, '5', SourceKind::Video),
                source("src-e-b", &production, '6', SourceKind::Video),
            ],
            Vec::new(),
        ))
        .expect("originals");

    for relation in [EdgeKind::Supports, EdgeKind::Contradicts] {
        let error = store
            .import_normalized(&batch(
                &case_id,
                Vec::new(),
                vec![edge(
                    "edge-evaluative",
                    (NodeKind::Source, "src-e-a"),
                    relation,
                    (NodeKind::Source, "src-e-b"),
                )],
            ))
            .expect_err("an evaluative relation must be refused");
        assert!(error.to_string().contains("structural"), "{error}");
        assert!(error.to_string().contains(relation.as_str()), "{error}");
    }
}

/// Machines cannot confer verification, and a batch that tries takes nothing
/// with it.
#[test]
fn an_adapter_edge_cannot_arrive_verified() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-verified");
    store
        .import_normalized(&batch(
            &case_id,
            vec![
                source("src-v-a", &production, '7', SourceKind::Video),
                source("src-v-b", &production, '8', SourceKind::Video),
            ],
            Vec::new(),
        ))
        .expect("originals");
    let before = sources_held(&store, &case_id);

    let mut verified = edge(
        "edge-verified",
        (NodeKind::Source, "src-v-a"),
        EdgeKind::TemporallyOverlaps,
        (NodeKind::Source, "src-v-b"),
    );
    verified.extraction.review_state = ReviewState::Verified;

    let mut authored = edge(
        "edge-authored",
        (NodeKind::Source, "src-v-a"),
        EdgeKind::RefersTo,
        (NodeKind::Source, "src-v-b"),
    );
    authored.extraction.machine_generated = false;

    for (proposed, expected) in [
        (verified, "must enter as suggested"),
        (authored, "attributed to a person"),
    ] {
        let carried = source("src-carried", &production, '9', SourceKind::Video);
        let error = store
            .import_normalized(&batch(&case_id, vec![carried], vec![proposed]))
            .expect_err("an adapter may not confer review");
        assert!(error.to_string().contains(expected), "{error}");
        assert_eq!(
            sources_held(&store, &case_id),
            before,
            "the whole batch must roll back, including the source it carried"
        );
    }
}

/// Cases do not share records, and an identifier the case does not hold is
/// refused rather than quietly creating the node it names.
#[test]
fn an_adapter_edge_needs_both_endpoints_in_the_case() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-endpoints");
    let (other_id, other_production) = open_case(&mut store, "case-neighbour");
    store
        .import_normalized(&batch(
            &case_id,
            vec![source("src-here", &production, 'a', SourceKind::Video)],
            Vec::new(),
        ))
        .expect("this case's original");
    store
        .import_normalized(&batch(
            &other_id,
            vec![source(
                "src-elsewhere",
                &other_production,
                'b',
                SourceKind::Video,
            )],
            Vec::new(),
        ))
        .expect("the other case's original");
    let before = sources_held(&store, &case_id);

    let missing = store
        .import_normalized(&batch(
            &case_id,
            vec![source("src-tagalong", &production, 'c', SourceKind::Video)],
            vec![edge(
                "edge-missing",
                (NodeKind::Source, "src-here"),
                EdgeKind::TemporallyOverlaps,
                (NodeKind::Source, "src-never-delivered"),
            )],
        ))
        .expect_err("an endpoint the case does not hold must be refused");
    assert!(missing.to_string().contains("was not found"), "{missing}");
    assert_eq!(
        sources_held(&store, &case_id),
        before,
        "the endpoint check runs inside the transaction, so the batch rolls back"
    );

    let borrowed = store
        .import_normalized(&batch(
            &case_id,
            Vec::new(),
            vec![edge(
                "edge-borrowed",
                (NodeKind::Source, "src-here"),
                EdgeKind::TemporallyOverlaps,
                (NodeKind::Source, "src-elsewhere"),
            )],
        ))
        .expect_err("another case's record must be refused");
    assert!(borrowed.to_string().contains("another case"), "{borrowed}");
    assert_eq!(
        sources_held(&store, &other_id),
        1,
        "the other case is untouched"
    );
}

/// An adapter points at a pair, not at an orientation, so the mirror image of a
/// relationship the case already holds is not a second thing to review.
#[test]
fn an_adapter_edge_is_not_held_twice_in_either_direction() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-pair");
    store
        .import_normalized(&batch(
            &case_id,
            vec![
                source("src-p-a", &production, 'd', SourceKind::Video),
                source("src-p-b", &production, 'e', SourceKind::Video),
            ],
            vec![edge(
                "edge-first",
                (NodeKind::Source, "src-p-a"),
                EdgeKind::TemporallyOverlaps,
                (NodeKind::Source, "src-p-b"),
            )],
        ))
        .expect("first proposal");

    let error = store
        .import_normalized(&batch(
            &case_id,
            Vec::new(),
            vec![edge(
                "edge-mirror",
                (NodeKind::Source, "src-p-b"),
                EdgeKind::TemporallyOverlaps,
                (NodeKind::Source, "src-p-a"),
            )],
        ))
        .expect_err("the mirror image is the same claim");
    assert!(error.to_string().contains("already exists"), "{error}");

    // A different relation between the same pair says something else, and is
    // still a new thing for a reviewer to look at.
    store
        .import_normalized(&batch(
            &case_id,
            Vec::new(),
            vec![edge(
                "edge-other-relation",
                (NodeKind::Source, "src-p-b"),
                EdgeKind::RefersTo,
                (NodeKind::Source, "src-p-a"),
            )],
        ))
        .expect("a different relation is a different claim");
    assert_eq!(
        queued(&store, &case_id, "edge-other-relation").review_state,
        "suggested"
    );
}

/// A relationship spans sources and has no original of its own, so the written
/// basis is the only thing a reviewer can weigh.
#[test]
fn an_adapter_edge_needs_a_written_rationale() {
    let mut store = empty_store();
    let (case_id, production) = open_case(&mut store, "case-rationale");
    store
        .import_normalized(&batch(
            &case_id,
            vec![
                source("src-r-a", &production, 'f', SourceKind::Video),
                source("src-r-b", &production, '0', SourceKind::Video),
            ],
            Vec::new(),
        ))
        .expect("originals");

    for blank in ["", "   \t "] {
        let mut bare = edge(
            "edge-bare",
            (NodeKind::Source, "src-r-a"),
            EdgeKind::TemporallyOverlaps,
            (NodeKind::Source, "src-r-b"),
        );
        bare.rationale = blank.to_owned();
        let error = store
            .import_normalized(&batch(&case_id, Vec::new(), vec![bare]))
            .expect_err("an unexplained proposal must be refused");
        assert!(error.to_string().contains("written rationale"), "{error}");
    }
}

/// `edges` is new to the contract, so a batch document written before it still
/// parses and simply proposes nothing.
#[test]
fn an_older_batch_document_without_edges_still_parses() {
    let document = r#"{
      "case_id": "case-legacy",
      "sources": [
        {
          "id": "src-legacy",
          "production_id": "prod-legacy",
          "logical_name": "report.pdf",
          "media_type": "application/pdf",
          "source_kind": "document",
          "temporal_relation": "after_event",
          "sha256": "1111111111111111111111111111111111111111111111111111111111111111",
          "byte_length": 12,
          "segments": []
        }
      ]
    }"#;
    let parsed: NormalizedBatch = serde_json::from_str(document).expect("an older batch parses");
    assert_eq!(parsed.case_id.0, "case-legacy");
    assert!(parsed.edges.is_empty());
}
