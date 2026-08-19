//! Keyframe retrieval is a finder index: it points, it does not assert.

#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ContentKind, EdgeKind, ExportAudience, ExtractionProvenance, IndexedKeyframe,
    KeyframeIndex, NodeKind, NormalizedBatch, NormalizedContent, NormalizedEdge, NormalizedSegment,
    NormalizedSource, ProposedCase, ReviewState, SourceKind, Store, TemporalRelation,
};

fn empty_store() -> Store {
    Store::in_memory().expect("in-memory store")
}

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
        extractor: "scene_cut".to_owned(),
        version: "1.0.0".to_owned(),
        machine_generated: true,
        confidence: None,
        review_state: ReviewState::Suggested,
    }
}

fn video(id: &str, production_id: &str, digit: char) -> NormalizedSource {
    NormalizedSource {
        id: id.to_owned(),
        production_id: production_id.to_owned(),
        logical_name: format!("{id}.mp4"),
        media_type: "video/mp4".to_owned(),
        source_kind: SourceKind::Video,
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: digit.to_string().repeat(64),
        byte_length: 12_000,
        segments: vec![NormalizedSegment {
            id: format!("{id}-scene-0001"),
            locator: "scene 1, 00:00:00.000–00:00:10.000".to_owned(),
            page: None,
            start_ms: Some(0),
            end_ms: Some(10_000),
            bounding_box: None,
            content: vec![NormalizedContent {
                id: format!("{id}-scenec-0001"),
                kind: ContentKind::Observation,
                text: "Scene 1 on the original timeline.".to_owned(),
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

fn still(
    video_id: &str,
    index: u32,
    production_id: &str,
    digit: char,
    start_ms: u64,
) -> NormalizedSource {
    let id = format!("{video_id}-still-{index:04}");
    let locator = format!(
        "scene {index}, 00:00:{:02}.000–00:00:{:02}.000",
        start_ms / 1000,
        start_ms / 1000
    );
    NormalizedSource {
        id: id.clone(),
        production_id: production_id.to_owned(),
        logical_name: format!("{video_id}.mp4 @ 00:00:{:02}.000", start_ms / 1000),
        media_type: "image/jpeg".to_owned(),
        source_kind: SourceKind::Other,
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: digit.to_string().repeat(64),
        byte_length: 2_048,
        segments: vec![NormalizedSegment {
            id: format!("{video_id}-stillseg-{index:04}"),
            locator,
            page: None,
            start_ms: Some(start_ms),
            end_ms: Some(start_ms),
            bounding_box: None,
            content: vec![NormalizedContent {
                id: format!("{video_id}-stillc-{index:04}"),
                kind: ContentKind::Observation,
                text: format!(
                    "Keyframe from `{video_id}.mp4`. Derived working copy — not the original."
                ),
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

fn derived_from(video_id: &str, index: u32) -> NormalizedEdge {
    let still_id = format!("{video_id}-still-{index:04}");
    NormalizedEdge {
        id: format!("{video_id}-stilledge-{index:04}"),
        from_kind: NodeKind::Source,
        from_id: still_id,
        relation: EdgeKind::DerivedFrom,
        to_kind: NodeKind::Source,
        to_id: video_id.to_owned(),
        rationale: "Keyframe extracted from the original; a derived working copy.".to_owned(),
        extraction: machine(),
    }
}

fn import_clip(store: &mut Store, case_id: &CaseId, production_id: &str, video_id: &str) {
    store
        .import_normalized(&NormalizedBatch {
            case_id: case_id.clone(),
            sources: vec![
                video(video_id, production_id, 'a'),
                still(video_id, 1, production_id, 'b', 0),
                still(video_id, 2, production_id, 'c', 10_000),
                still(video_id, 3, production_id, 'd', 20_000),
            ],
            edges: vec![
                derived_from(video_id, 1),
                derived_from(video_id, 2),
                derived_from(video_id, 3),
            ],
        })
        .expect("import clip");
}

fn embedding(source_id: &str, vector: Vec<f32>) -> IndexedKeyframe {
    IndexedKeyframe {
        source_id: source_id.to_owned(),
        model: "test-clip".to_owned(),
        extractor: "keyframe_embed".to_owned(),
        version: "from-json".to_owned(),
        vector,
    }
}

fn index_unit_vectors(store: &mut Store, case_id: &CaseId, video_id: &str) {
    store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![
                embedding(&format!("{video_id}-still-0001"), vec![0.6, 0.8]),
                embedding(&format!("{video_id}-still-0002"), vec![1.0, 0.0]),
                embedding(&format!("{video_id}-still-0003"), vec![0.0, 1.0]),
            ],
        })
        .expect("index");
}

fn content_and_edge_counts(store: &Store, case_id: &CaseId) -> (u32, usize) {
    let pending = store.overview(case_id).expect("overview").pending_review;
    let queue = store.review_queue(case_id).expect("queue").len();
    (pending, queue)
}

/// A hit names the original by hash and the time range on that original, plus
/// the derived still, so looking takes one action.
#[test]
fn a_matching_query_returns_the_original_hash_and_locator() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-hit");
    import_clip(&mut store, &case_id, &production_id, "cam");
    index_unit_vectors(&mut store, &case_id, "cam");

    let hits = store
        .search_keyframes(&case_id, "test-clip", &[1.0, 0.0], 25)
        .expect("search");
    assert_eq!(hits.len(), 2, "the orthogonal still stays below the cut");
    let first = hits.iter().find(|hit| hit.source_id == "cam-still-0002");
    let first = first.expect("the aligned still is a hit");
    assert_eq!(first.sha256, "a".repeat(64));
    assert_eq!(first.source, "cam.mp4");
    assert!(first.locator.contains("scene 2"));
    assert_eq!(first.still_sha256, "c".repeat(64));
    assert_eq!(first.review_state, "suggested");
    assert!(first.machine_generated);
}

/// Within the cut, identifier order is the order. A closer match with a later
/// id does not jump the queue.
#[test]
fn hits_are_ordered_by_identifier_not_similarity() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-order");
    import_clip(&mut store, &case_id, &production_id, "cam");
    index_unit_vectors(&mut store, &case_id, "cam");

    let hits = store
        .search_keyframes(&case_id, "test-clip", &[1.0, 0.0], 25)
        .expect("search");
    let ids: Vec<_> = hits.iter().map(|hit| hit.source_id.as_str()).collect();
    assert_eq!(ids, ["cam-still-0001", "cam-still-0002"]);
}

/// A neighbour below the cut is omitted rather than ranked last.
#[test]
fn below_the_cut_is_omitted() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-cut");
    import_clip(&mut store, &case_id, &production_id, "cam");
    index_unit_vectors(&mut store, &case_id, "cam");

    let hits = store
        .search_keyframes(&case_id, "test-clip", &[1.0, 0.0], 25)
        .expect("search");
    assert!(
        hits.iter().all(|hit| hit.source_id != "cam-still-0003"),
        "orthogonal still must not appear: {hits:?}"
    );
}

/// A number printed next to a frame is read as a measurement of the frame.
#[test]
fn nothing_in_the_hit_reports_a_similarity_number() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-score");
    import_clip(&mut store, &case_id, &production_id, "cam");
    index_unit_vectors(&mut store, &case_id, "cam");

    let hits = store
        .search_keyframes(&case_id, "test-clip", &[1.0, 0.0], 25)
        .expect("search");
    let rendered = serde_json::to_string(&hits).expect("serialize");
    for forbidden in [
        "score",
        "similarity",
        "cosine",
        "distance",
        "rank",
        "confidence",
        "strength",
    ] {
        assert!(
            !rendered.to_lowercase().contains(forbidden),
            "a keyframe hit must not speak in terms of `{forbidden}`: {rendered}"
        );
    }
}

/// Retrieval never writes an observation or an edge. Only what a reviewer
/// confirms is authored.
#[test]
fn indexing_writes_no_observation_and_no_edge() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-silent");
    import_clip(&mut store, &case_id, &production_id, "cam");
    let before = content_and_edge_counts(&store, &case_id);
    index_unit_vectors(&mut store, &case_id, "cam");
    let after = content_and_edge_counts(&store, &case_id);
    assert_eq!(before, after);
}

/// The finder index is not evidence and is not work product. No export
/// audience reads the table.
#[test]
fn export_never_carries_keyframe_vectors() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-export");
    import_clip(&mut store, &case_id, &production_id, "cam");
    store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![IndexedKeyframe {
                source_id: "cam-still-0001".to_owned(),
                model: "CANARY-EMBED-do-not-export".to_owned(),
                extractor: "keyframe_embed".to_owned(),
                version: "from-json".to_owned(),
                vector: vec![1.0, 0.0],
            }],
        })
        .expect("index");

    for audience in [ExportAudience::Disclosable, ExportAudience::WorkFile] {
        let export = store.export_case(&case_id, audience).expect("export");
        let rendered = serde_json::to_string(&export).expect("serialize");
        assert!(
            !rendered.contains("CANARY-EMBED-do-not-export"),
            "{audience:?} must not carry embedding metadata: {rendered}"
        );
        assert!(
            !rendered.contains("keyframe_embed"),
            "{audience:?} must not carry the embedding extractor: {rendered}"
        );
    }
}

/// One case's stills never surface in another's visual search.
#[test]
fn keyframe_search_is_scoped_to_its_case() {
    let mut store = empty_store();
    let (alpha, alpha_prod) = open_case(&mut store, "kf-alpha");
    let (beta, beta_prod) = open_case(&mut store, "kf-beta");
    import_clip(&mut store, &alpha, &alpha_prod, "cam-a");
    import_clip(&mut store, &beta, &beta_prod, "cam-b");
    index_unit_vectors(&mut store, &alpha, "cam-a");
    index_unit_vectors(&mut store, &beta, "cam-b");

    let hits = store
        .search_keyframes(&beta, "test-clip", &[1.0, 0.0], 25)
        .expect("search");
    assert!(
        hits.iter().all(|hit| hit.source_id.starts_with("cam-b-")),
        "alpha's stills must not appear: {hits:?}"
    );
}

/// A source that is not a derived still cannot be indexed. The original video
/// is not a working-copy jpeg.
#[test]
fn a_source_that_is_not_a_derived_still_is_refused() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-orig");
    import_clip(&mut store, &case_id, &production_id, "cam");
    let error = store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![embedding("cam", vec![1.0, 0.0])],
        })
        .expect_err("original is not a still");
    assert!(error.to_string().contains("not a derived still"), "{error}");
}

/// Mixing dimensions inside one model would make cosine meaningless.
#[test]
fn a_mismatched_model_dimension_is_refused() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-dim");
    import_clip(&mut store, &case_id, &production_id, "cam");
    store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![embedding("cam-still-0001", vec![1.0, 0.0])],
        })
        .expect("first index");
    let error = store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![embedding("cam-still-0002", vec![1.0, 0.0, 0.0])],
        })
        .expect_err("dimension change");
    assert!(error.to_string().contains("dimensional"), "{error}");
}

/// Overnight re-runs replace the vector rather than stacking a second index.
#[test]
fn reindexing_the_same_still_replaces_the_vector() {
    let mut store = empty_store();
    let (case_id, production_id) = open_case(&mut store, "kf-up");
    import_clip(&mut store, &case_id, &production_id, "cam");
    store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![embedding("cam-still-0001", vec![1.0, 0.0])],
        })
        .expect("first");
    store
        .index_keyframes(&KeyframeIndex {
            case_id: case_id.clone(),
            embeddings: vec![embedding("cam-still-0001", vec![0.0, 1.0])],
        })
        .expect("replace");

    let along_x = store
        .search_keyframes(&case_id, "test-clip", &[1.0, 0.0], 25)
        .expect("x");
    assert!(
        along_x.is_empty(),
        "the replaced vector is orthogonal to x: {along_x:?}"
    );
    let along_y = store
        .search_keyframes(&case_id, "test-clip", &[0.0, 1.0], 25)
        .expect("y");
    assert_eq!(along_y.len(), 1);
    assert_eq!(along_y[0].source_id, "cam-still-0001");
}
