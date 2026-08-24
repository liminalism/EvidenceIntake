#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ExtractionProvenance, NormalizedBatch, NormalizedContent,
    NormalizedSegment, NormalizedSource, ReviewDecision, ReviewState, ReviewTarget, SourceKind,
    Store, TemporalRelation,
};

fn fixture() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("seed fixture");
    (store, case_id)
}

#[test]
fn fixture_exposes_discovery_completeness_problem() {
    let (store, case_id) = fixture();
    let overview = store.overview(&case_id).expect("overview");
    assert_eq!(overview.missing_references, 1);

    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    assert!(ledger.iter().any(|item| {
        item.source == "Officer Chen backup BWC" && item.integrity_status == "missing"
    }));
    assert!(ledger.iter().any(|item| item.supersedes.is_some()));
}

#[test]
fn element_matrix_preserves_uncertainty() {
    let (store, case_id) = fixture();
    let matrix = store.element_matrix(&case_id).expect("matrix");
    assert!(matrix.iter().any(|row| {
        row.element.contains("possessed") && row.assessment.as_deref() == Some("uncertain")
    }));
    assert!(!matrix.iter().any(|row| {
        row.assessment
            .as_deref()
            .is_some_and(|assessment| assessment == "true" || assessment == "false")
    }));
}

#[test]
fn witness_dossier_orders_distinct_attributed_accounts() {
    let (store, case_id) = fixture();
    let dossier = store
        .witness_dossier(&case_id, "person-patel")
        .expect("witness dossier");
    assert_eq!(dossier.len(), 2);
    assert!(dossier[0].text.contains("belonged"));
    assert!(dossier[1].text.contains("did not see"));
    assert!(
        dossier[1]
            .credibility_links
            .iter()
            .any(|link| link.starts_with("impeaches:"))
    );
}

#[test]
fn timeline_keeps_competing_lanes_and_raw_time() {
    let (store, case_id) = fixture();
    let timeline = store.contested_timeline(&case_id).expect("timeline");
    assert!(timeline.iter().any(|event| event.lane == "recorded"));
    assert!(
        timeline
            .iter()
            .any(|event| event.lane == "police_narrative")
    );
    assert!(
        timeline
            .iter()
            .any(|event| event.lane == "attorney_hypothesis")
    );
    assert!(timeline.iter().all(|event| event.raw_time.is_some()));
}

#[test]
fn collation_preserves_time_location_provenance_without_fuzzy_place_matching() {
    let (store, case_id) = fixture();
    let index = store.collation_index(&case_id).expect("collation index");

    let day = index
        .by_date
        .iter()
        .find(|group| group.normalized_date.as_deref() == Some("2026-01-08"))
        .expect("normalized date group");
    let bodycam = day
        .entries
        .iter()
        .find(|entry| entry.content_id == "content-bodycam-question")
        .expect("body-camera passage");
    assert_eq!(bodycam.source_id, "src-bodycam");
    assert_eq!(bodycam.source, "Chen BWC 0042.mp4");
    assert_eq!(bodycam.source_kind, "video");
    assert_eq!(bodycam.locator, "00:04:10–00:04:32");
    assert_eq!(bodycam.raw_time.as_deref(), Some("BWC 22:18:10"));
    assert!(bodycam.content_created_at.is_none());
    assert_eq!(
        bodycam.asserted_time.as_deref(),
        Some("2026-01-08T22:18:10")
    );
    assert_eq!(
        bodycam.normalized_start.as_deref(),
        Some("2026-01-08T22:14:08Z")
    );
    assert!(
        bodycam
            .time_basis
            .as_deref()
            .is_some_and(|basis| basis.contains("clock correction"))
    );
    assert_eq!(bodycam.location.as_deref(), Some("400 block of Oak Street"));
    assert!(!bodycam.machine_generated);
    assert_eq!(bodycam.extractor.as_deref(), Some("human_fixture"));
    assert_eq!(bodycam.review_state, "verified");
    assert!(
        day.entries.windows(2).all(|pair| {
            pair[0].normalized_start.as_deref().unwrap_or("")
                <= pair[1].normalized_start.as_deref().unwrap_or("")
        }),
        "date buckets must remain chronological"
    );

    let exact_place = index
        .by_location
        .iter()
        .find(|group| group.location.as_deref() == Some("400 block of Oak Street"))
        .expect("exact location group");
    assert!(
        !exact_place
            .entries
            .iter()
            .any(|entry| entry.content_id == "content-dispatch"),
        "`Oak St` is not silently expanded into `400 block of Oak Street`"
    );

    let bodycam_coverage = index
        .source_coverage
        .iter()
        .find(|source| source.source_id == "src-bodycam")
        .expect("body-camera coverage");
    assert_eq!(bodycam_coverage.source_kind, "video");
    assert_eq!(bodycam_coverage.passages, 3);
    assert_eq!(bodycam_coverage.with_raw_time, 3);
    assert_eq!(bodycam_coverage.with_content_created_at, 0);
    assert_eq!(bodycam_coverage.with_asserted_time, 2);
    assert_eq!(bodycam_coverage.with_normalized_date, 3);
    assert_eq!(bodycam_coverage.with_location, 3);

    assert_eq!(index.without_normalized_date, 4);
    assert_eq!(index.without_location, 1);
    assert_eq!(index.needs_placement.len(), 4);
    let missing_reference = index
        .needs_placement
        .iter()
        .find(|gap| gap.entry.content_id == "content-backup-ref")
        .expect("unplaced evidence reference");
    assert_eq!(
        missing_reference.missing_anchors,
        ["normalized_date", "location"]
    );
    assert_eq!(missing_reference.entry.locator, "page 4, paragraph 2");

    let rendered = serde_json::to_string(&index).expect("serialize collation index");
    for forbidden in ["score", "rank", "likelihood", "probability", "confidence"] {
        assert!(
            !rendered.to_lowercase().contains(forbidden),
            "collation reported an evaluative field: {forbidden}"
        );
    }
}

#[test]
fn placement_gaps_exclude_content_a_reviewer_rejected() {
    let (mut store, case_id) = fixture();
    let batch = sample_machine_batch(case_id.clone(), ReviewState::Suggested);
    store
        .import_normalized(&batch)
        .expect("import unplaced content");
    assert!(
        store
            .collation_index(&case_id)
            .expect("before rejection")
            .needs_placement
            .iter()
            .any(|gap| gap.entry.content_id == "adapter-content")
    );

    store
        .apply_review(
            &case_id,
            &ReviewDecision {
                target: ReviewTarget::Content,
                target_id: "adapter-content".to_owned(),
                to_state: ReviewState::Rejected,
                actor: "A. Reviewer".to_owned(),
                basis: Some("Not useful to this review.".to_owned()),
                verified_against_locator: None,
            },
        )
        .expect("reject content");

    let index = store.collation_index(&case_id).expect("after rejection");
    assert!(
        !index
            .needs_placement
            .iter()
            .any(|gap| gap.entry.content_id == "adapter-content"),
        "rejected material must not remain in placement work"
    );
    assert!(
        index
            .source_coverage
            .iter()
            .all(|source| source.source_id != "adapter-source"),
        "coverage describes active content, not rejected content"
    );
}

#[test]
fn shared_anchor_groups_need_shared_date_location_and_distinct_originals() {
    let (mut store, case_id) = fixture();
    let batch = NormalizedBatch {
        case_id: case_id.clone(),
        edges: Vec::new(),
        sources: vec![
            collation_source(
                "collation-report",
                '6',
                "application/pdf",
                SourceKind::Document,
                "2026-01-08T22:14:20Z",
                "400 block of Oak Street",
            ),
            collation_source(
                "collation-audio",
                '7',
                "audio/wav",
                SourceKind::Audio,
                "2026-01-08T22:14:25Z",
                "  400 BLOCK of Oak Street  ",
            ),
        ],
    };
    store
        .import_normalized(&batch)
        .expect("import collation sample");
    let queue_before = store.review_queue(&case_id).expect("queue before");

    let index = store.collation_index(&case_id).expect("collation index");
    let group = index
        .shared_anchor_unconfirmed
        .iter()
        .find(|group| {
            let ids: Vec<_> = group
                .entries
                .iter()
                .map(|entry| entry.source_id.as_str())
                .collect();
            ids.contains(&"collation-report") && ids.contains(&"collation-audio")
        })
        .expect("multi-source date and location group");
    assert_eq!(group.normalized_date.as_deref(), Some("2026-01-08"));
    assert_eq!(group.location.as_deref(), Some("400 block of Oak Street"));
    assert!(group.distinct_originals >= 2);
    assert!(
        group
            .rationale
            .contains("Shared reviewed date/location anchor only")
    );
    assert!(group.rationale.contains("Relationship not established"));
    for forbidden in ["score", "confidence", "likely", "probably", "same event"] {
        assert!(
            !group.rationale.to_lowercase().contains(forbidden),
            "rationale crossed the structural boundary: {}",
            group.rationale
        );
    }

    assert_eq!(
        store.review_queue(&case_id).expect("queue after"),
        queue_before,
        "opening a read model must write no relationship or review item"
    );
    let other = CaseId("not-this-case".to_owned());
    let error = store
        .collation_index(&other)
        .expect_err("unknown case must be refused");
    assert!(error.to_string().contains("not-this-case"));
}

#[test]
fn issue_workspace_does_not_encode_legal_outcome() {
    let (store, case_id) = fixture();
    let issues = store.issue_workspaces(&case_id).expect("issues");
    assert_eq!(issues.len(), 1);
    assert!(issues[0].body.contains("No legal conclusion"));
    assert_eq!(issues[0].follow_up.len(), 2);
}

#[test]
fn seeding_is_idempotent() {
    let (mut store, case_id) = fixture();
    let second = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("seed twice");
    assert_eq!(case_id, second);
    assert_eq!(store.cases().expect("cases").len(), 1);
}

#[test]
fn normalized_machine_content_enters_as_suggestion() {
    let (mut store, case_id) = fixture();
    let batch = sample_machine_batch(case_id.clone(), ReviewState::Suggested);
    store
        .import_normalized(&batch)
        .expect("import normalized machine output");

    let overview = store.overview(&case_id).expect("overview");
    assert_eq!(overview.sources, 6);
    let evidence = store.discovery_ledger(&case_id).expect("discovery ledger");
    assert!(
        evidence
            .iter()
            .any(|item| item.source == "adapter sample.txt")
    );
}

#[test]
fn normalized_machine_content_cannot_self_verify() {
    let (mut store, case_id) = fixture();
    let batch = sample_machine_batch(case_id, ReviewState::Verified);
    let error = store
        .import_normalized(&batch)
        .expect_err("machine verification must be rejected");
    assert!(error.to_string().contains("must enter as suggested"));
}

#[test]
fn hit_and_run_exposes_lesser_offense_without_collapsing_uncertainty() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit-and-run fixture");
    let offenses = store
        .offense_comparison(&case_id)
        .expect("offense comparison");

    assert_eq!(offenses.len(), 3);
    let lesser = offenses
        .iter()
        .find(|offense| offense.posture == "lesser_candidate")
        .expect("lesser candidate");
    assert_eq!(lesser.grade.as_deref(), Some("misdemeanor"));
    assert!(lesser.elements.iter().any(
        |element| element.element.contains("property damage") && !element.supporting.is_empty()
    ));

    let dui = offenses
        .iter()
        .find(|offense| offense.id == "hr-charge-dui")
        .expect("DUI charge");
    assert!(
        dui.elements
            .iter()
            .any(|element| element.element.contains("impaired")
                && !element.uncertain.is_empty()
                && !element.opposing.is_empty())
    );
}

#[test]
fn hit_and_run_reconstruction_keeps_later_impairment_at_later_time() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit-and-run fixture");
    let evidence = store
        .proposition_evidence(&case_id, "hr-prop-impaired-driving")
        .expect("impairment reconstruction");

    assert_eq!(evidence.len(), 4);
    assert!(evidence.iter().any(|item| {
        item.text.contains("0.060")
            && item.normalized_start.as_deref() == Some("2026-02-27T22:42:00Z")
            && item
                .rationale
                .as_deref()
                .is_some_and(|text| text.contains("no admitted extrapolation"))
    }));
    assert!(evidence.iter().any(|item| {
        item.text.contains("two drinks after")
            && item.relation == "explains"
            && item.machine_generated
    }));
}

#[test]
fn hit_and_run_keeps_report_creation_distinct_from_alleged_event_time() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit-and-run fixture");
    let evidence = store
        .proposition_evidence(&case_id, "hr-prop-collision")
        .expect("collision reconstruction");
    let report = evidence
        .iter()
        .find(|item| item.text.starts_with("Lee's stopped vehicle"))
        .expect("after-event report assertion");

    assert_eq!(report.source_time.as_deref(), Some("2026-02-27T23:30:00Z"));
    assert_eq!(report.asserted_time.as_deref(), Some("2026-02-27T21:07:00"));
}

fn sample_machine_batch(case_id: CaseId, review_state: ReviewState) -> NormalizedBatch {
    NormalizedBatch {
        case_id,
        edges: Vec::new(),
        sources: vec![NormalizedSource {
            id: "adapter-source".to_owned(),
            production_id: "prod-01".to_owned(),
            logical_name: "adapter sample.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            source_kind: SourceKind::Document,
            temporal_relation: TemporalRelation::AfterEvent,
            sha256: "abababababababababababababababababababababababababababababababab".to_owned(),
            byte_length: 42,
            segments: vec![NormalizedSegment {
                id: "adapter-segment".to_owned(),
                locator: "line 1".to_owned(),
                page: None,
                start_ms: None,
                end_ms: None,
                bounding_box: None,
                content: vec![NormalizedContent {
                    id: "adapter-content".to_owned(),
                    kind: ContentKind::Observation,
                    text: "A bounded machine observation.".to_owned(),
                    speaker_entity_id: None,
                    attributed_to_entity_id: None,
                    parent_content_id: None,
                    raw_time: None,
                    content_created_at: Some("2026-01-09T12:00:00Z".to_owned()),
                    asserted_time: None,
                    normalized_start: None,
                    normalized_end: None,
                    time_basis: None,
                    location_text: None,
                    extraction: ExtractionProvenance {
                        extractor: "test_adapter".to_owned(),
                        version: "1.0.0".to_owned(),
                        machine_generated: true,
                        confidence: Some(0.75),
                        review_state,
                    },
                }],
            }],
        }],
    }
}

fn collation_source(
    id: &str,
    hash_digit: char,
    media_type: &str,
    source_kind: SourceKind,
    normalized_start: &str,
    location: &str,
) -> NormalizedSource {
    NormalizedSource {
        id: id.to_owned(),
        production_id: "prod-01".to_owned(),
        logical_name: format!("{id}.{media_type}"),
        media_type: media_type.to_owned(),
        source_kind,
        temporal_relation: TemporalRelation::Contemporaneous,
        sha256: hash_digit.to_string().repeat(64),
        byte_length: 128,
        segments: vec![NormalizedSegment {
            id: format!("{id}-segment"),
            locator: "source locator".to_owned(),
            page: None,
            start_ms: None,
            end_ms: None,
            bounding_box: None,
            content: vec![NormalizedContent {
                id: format!("{id}-content"),
                kind: ContentKind::Observation,
                text: format!("Bounded observation from {id}."),
                speaker_entity_id: None,
                attributed_to_entity_id: None,
                parent_content_id: None,
                raw_time: Some("device 22:14".to_owned()),
                content_created_at: None,
                asserted_time: Some("2026-01-08T22:14:00".to_owned()),
                normalized_start: Some(normalized_start.to_owned()),
                normalized_end: None,
                time_basis: Some("reviewer-entered synchronization".to_owned()),
                location_text: Some(location.to_owned()),
                extraction: ExtractionProvenance {
                    extractor: "collation_fixture".to_owned(),
                    version: "1".to_owned(),
                    machine_generated: true,
                    confidence: None,
                    review_state: ReviewState::Suggested,
                },
            }],
        }],
    }
}

/// Tasks belong to the issue that raised them. Listing every open task in the
/// case under every issue told a defender reading one workspace to chase work
/// that belongs to an unrelated question.
#[test]
fn each_issue_workspace_carries_only_its_own_follow_up() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit and run");
    let issues = store.issue_workspaces(&case_id).expect("issues");
    assert_eq!(issues.len(), 2);

    let identity = issues
        .iter()
        .find(|issue| issue.id == "hr-issue-identity")
        .expect("identity issue");
    let timing = issues
        .iter()
        .find(|issue| issue.id == "hr-issue-dui-time")
        .expect("timing issue");

    assert_ne!(
        identity.follow_up, timing.follow_up,
        "two issues must not carry an identical case-wide task list"
    );
    assert!(
        identity
            .follow_up
            .iter()
            .any(|task| task.starts_with("Independent vehicle comparison")),
        "vehicle comparison is an identity task"
    );
    assert!(
        !timing
            .follow_up
            .iter()
            .any(|task| task.starts_with("Independent vehicle comparison")),
        "vehicle comparison is not an impairment-timing task"
    );

    // A task serving two issues appears under both.
    let shared = "Test route and arrival-time account";
    assert!(identity.follow_up.iter().any(|t| t.starts_with(shared)));
    assert!(timing.follow_up.iter().any(|t| t.starts_with(shared)));

    // The follow-up edges are not also listed as factual material.
    assert!(
        identity
            .linked_material
            .iter()
            .all(|link| !link.starts_with("requires_follow_up")),
        "an issue's tasks must not appear twice under two headings"
    );
}
