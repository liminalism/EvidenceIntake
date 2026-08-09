#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ExtractionProvenance, NormalizedBatch, NormalizedContent,
    NormalizedSegment, NormalizedSource, ReviewState, SourceKind, Store, TemporalRelation,
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
