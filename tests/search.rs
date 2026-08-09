//! What full-text search over the case may and may not reach.

use evidence_intake::{
    AdvocacyKind, CaseId, ContentKind, DemoFixture, Error, ExtractionProvenance, NormalizedBatch,
    NormalizedContent, NormalizedSegment, NormalizedSource, ProposedAdvocacyItem, ProposedBrief,
    ReviewState, SourceKind, Store, TemporalRelation,
};

/// A phrase seeded into privileged tables and expected nowhere in a search.
const CANARY: &str = "zzzprivilegedcanary";

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("in-memory store");
    let case = DemoFixture::HitAndRun.seed(&mut store).expect("seed");
    (store, case)
}

/// A search hit is a place to look, so it carries the exact original locator
/// rather than only the words that matched.
#[test]
fn a_hit_names_the_original_it_can_be_opened_in() {
    let (store, case) = hit_and_run();
    let hits = store.search(&case, "hatchback", 25).expect("search");

    assert!(!hits.is_empty(), "the fixture describes a hatchback");
    for hit in &hits {
        assert!(!hit.locator.trim().is_empty(), "every hit needs a locator");
        assert!(!hit.source.trim().is_empty(), "every hit names its source");
        assert!(
            hit.excerpt.contains('[') && hit.excerpt.contains(']'),
            "the matched words are marked in the excerpt: {}",
            hit.excerpt
        );
    }
}

/// Search is over the words in originals. Whether the case has done anything
/// with a passage is part of the answer: a hit nobody has connected to a
/// proposition is work waiting rather than work done.
#[test]
fn a_hit_reports_what_the_case_has_already_tied_it_to() {
    let (store, case) = hit_and_run();
    let hits = store.search(&case, "hatchback", 25).expect("search");

    assert!(
        hits.iter().any(|hit| !hit.bears_on.is_empty()),
        "the fixture ties some excerpts to propositions"
    );
    assert!(
        hits.iter()
            .flat_map(|hit| &hit.bears_on)
            .any(|link| link.contains(':')),
        "each link names its relationship"
    );
}

/// Privileged work product is not part of the evidentiary record, and a search
/// path that reached it would surface attorney analysis somewhere that does not
/// know it is privileged.
#[test]
fn search_never_reaches_privileged_work_product() {
    let (mut store, case) = hit_and_run();
    store
        .author_advocacy_item(
            &case,
            &ProposedAdvocacyItem {
                id: None,
                kind: AdvocacyKind::LegalIssue,
                title: format!("Issue {CANARY}"),
                body: format!("Analysis mentioning {CANARY}."),
                status: None,
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("advocacy item");
    store
        .record_brief(
            &case,
            &ProposedBrief {
                id: None,
                posture: "trial".to_owned(),
                summary: format!("Brief mentioning {CANARY}."),
                strengths: String::new(),
                risks: String::new(),
                unresolved_questions: String::new(),
                client_topics: String::new(),
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("brief");

    let hits = store.search(&case, CANARY, 25).expect("search");
    assert!(
        hits.is_empty(),
        "privileged text must not be reachable by search: {hits:?}"
    );
}

/// One case's words never surface in another's search.
#[test]
fn search_is_scoped_to_its_case() {
    let (mut store, hit_run) = hit_and_run();
    let vehicle_stop = DemoFixture::VehicleStop.seed(&mut store).expect("seed");

    let elsewhere = store
        .search(&vehicle_stop, "hatchback", 25)
        .expect("search");
    assert!(
        elsewhere.is_empty(),
        "the hatchback belongs to the other case: {elsewhere:?}"
    );
    assert!(
        !store
            .search(&hit_run, "hatchback", 25)
            .expect("search")
            .is_empty(),
        "and it is findable in its own"
    );
}

/// Content arriving from an adapter is findable as soon as it commits: the
/// index is maintained inside the same transaction as the write, not rebuilt
/// on a schedule that could leave a committed excerpt unfindable.
#[test]
fn imported_content_is_findable_immediately() {
    let (mut store, case) = hit_and_run();
    store
        .import_normalized(&NormalizedBatch {
            case_id: case.clone(),
            sources: vec![NormalizedSource {
                id: "search-source".to_owned(),
                production_id: "hr-prod-initial".to_owned(),
                logical_name: "late transcript.txt".to_owned(),
                media_type: "text/plain".to_owned(),
                source_kind: SourceKind::Audio,
                temporal_relation: TemporalRelation::AfterEvent,
                sha256: "cd".repeat(32),
                byte_length: 64,
                segments: vec![NormalizedSegment {
                    id: "search-segment".to_owned(),
                    locator: "00:04:12".to_owned(),
                    page: None,
                    start_ms: Some(252_000),
                    end_ms: Some(258_000),
                    bounding_box: None,
                    content: vec![NormalizedContent {
                        id: "search-content".to_owned(),
                        kind: ContentKind::Statement,
                        text: "The streetlight above the crossing was unlit that night.".to_owned(),
                        speaker_entity_id: None,
                        attributed_to_entity_id: None,
                        parent_content_id: None,
                        raw_time: None,
                        content_created_at: Some("2026-03-01T09:00:00Z".to_owned()),
                        asserted_time: None,
                        normalized_start: None,
                        normalized_end: None,
                        time_basis: None,
                        location_text: None,
                        extraction: ExtractionProvenance {
                            extractor: "asr".to_owned(),
                            version: "1.0.0".to_owned(),
                            machine_generated: true,
                            confidence: Some(0.8),
                            review_state: ReviewState::Suggested,
                        },
                    }],
                }],
            }],
        })
        .expect("import");

    let hits = store.search(&case, "streetlight", 25).expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "search-content");
    assert_eq!(hits[0].locator, "00:04:12");
    assert!(
        hits[0].bears_on.is_empty(),
        "nothing has been done with it yet, and that shows"
    );
}

/// A stray quote is a typo, not a broken database, and the message says which
/// query could not be read.
#[test]
fn an_unreadable_query_is_refused_with_what_was_typed() {
    let (store, case) = hit_and_run();
    let error = store
        .search(&case, "AND AND\"", 25)
        .expect_err("unreadable full-text syntax");

    assert!(matches!(error, Error::InvalidSearch(_)), "{error}");
    assert!(error.to_string().contains("AND AND"), "{error}");
}

#[test]
fn an_empty_query_is_refused_rather_than_matching_everything() {
    let (store, case) = hit_and_run();
    for query in ["", "   "] {
        let error = store
            .search(&case, query, 25)
            .expect_err("an empty search must be refused");
        assert!(matches!(error, Error::InvalidSearch(_)), "{error}");
    }
}

#[test]
fn a_limit_bounds_what_comes_back() {
    let (store, case) = hit_and_run();
    let hits = store
        .search(&case, "sedan OR hatchback", 2)
        .expect("search");
    assert!(hits.len() <= 2, "{} hits returned", hits.len());
}

/// Results are ordered by how well a passage matches the words asked for. That
/// ordering is never reported as a number, because a number attached to an
/// excerpt reads as a measurement of the evidence rather than of the match.
#[test]
fn a_hit_carries_no_score() {
    let (store, case) = hit_and_run();
    let hits = store.search(&case, "sedan", 25).expect("search");
    let rendered = serde_json::to_string(&hits).expect("serialize");

    for forbidden in ["score", "rank", "relevance", "confidence\":"] {
        assert!(
            !rendered.to_lowercase().contains(forbidden),
            "a search hit must not report `{forbidden}`"
        );
    }
}
