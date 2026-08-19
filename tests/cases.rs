//! Case lifecycle and the rule that cases do not share records.

#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ContentKind, DemoFixture, ExtractionProvenance, NormalizedBatch, NormalizedContent,
    NormalizedSegment, NormalizedSource, ProposedCase, ProposedEntity, ProposedProduction,
    ReviewState, SourceKind, Store, TemporalRelation,
};

fn empty_store() -> Store {
    Store::in_memory().expect("in-memory store")
}

fn both_fixtures() -> (Store, CaseId, CaseId) {
    let mut store = empty_store();
    let hit_run = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit-and-run");
    let vehicle = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle-stop");
    (store, hit_run, vehicle)
}

fn observation_batch(
    case_id: CaseId,
    production_id: &str,
    source_id: &str,
    sha256: &str,
) -> NormalizedBatch {
    NormalizedBatch {
        case_id,
        edges: Vec::new(),
        sources: vec![NormalizedSource {
            id: source_id.to_owned(),
            production_id: production_id.to_owned(),
            logical_name: "intake.wav".to_owned(),
            media_type: "audio/wav".to_owned(),
            source_kind: SourceKind::Audio,
            temporal_relation: TemporalRelation::Contemporaneous,
            sha256: sha256.to_owned(),
            byte_length: 64,
            segments: vec![NormalizedSegment {
                id: format!("{source_id}-seg"),
                locator: "00:00:00–00:00:01".to_owned(),
                page: None,
                start_ms: Some(0),
                end_ms: Some(1_000),
                bounding_box: None,
                content: vec![NormalizedContent {
                    id: format!("{source_id}-content"),
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
                    extraction: ExtractionProvenance {
                        extractor: "test_adapter".to_owned(),
                        version: "1.0.0".to_owned(),
                        machine_generated: true,
                        confidence: Some(0.5),
                        review_state: ReviewState::Suggested,
                    },
                }],
            }],
        }],
    }
}

/// A named case can be opened without a fixture and receives a first production
/// so intake has a ledger to attach to.
#[test]
fn a_case_can_be_opened_without_a_fixture() {
    let mut store = empty_store();
    let opened = store
        .open_case(&ProposedCase {
            id: Some("case-hall-001".to_owned()),
            name: "State v. Hall".to_owned(),
            reference: Some("PD-2026-0900".to_owned()),
            jurisdiction: Some("Example".to_owned()),
            production: Some("Brady disk 1".to_owned()),
        })
        .expect("open case");

    assert_eq!(opened.id, "case-hall-001");
    assert_eq!(opened.name, "State v. Hall");
    assert_eq!(opened.reference.as_deref(), Some("PD-2026-0900"));
    assert_eq!(opened.production.label, "Brady disk 1");

    let case_id = CaseId(opened.id.clone());
    let overview = store.overview(&case_id).expect("overview");
    assert_eq!(overview.case_name, "State v. Hall");
    assert_eq!(overview.productions, 1);
    assert_eq!(overview.sources, 0);

    store
        .import_normalized(&observation_batch(
            case_id.clone(),
            &opened.production.id,
            "src-911",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ))
        .expect("import into the new case");
    assert_eq!(store.overview(&case_id).expect("overview").sources, 1);
}

#[test]
fn opening_a_case_requires_a_name() {
    let mut store = empty_store();
    let error = store
        .open_case(&ProposedCase {
            id: None,
            name: "   ".to_owned(),
            reference: None,
            jurisdiction: None,
            production: None,
        })
        .expect_err("an unnamed case must be refused");
    assert!(error.to_string().contains("must have a name"));
}

#[test]
fn a_case_identifier_cannot_be_reused() {
    let mut store = empty_store();
    let proposal = ProposedCase {
        id: Some("case-reuse".to_owned()),
        name: "First".to_owned(),
        reference: None,
        jurisdiction: None,
        production: None,
    };
    store.open_case(&proposal).expect("first");
    let error = store
        .open_case(&proposal)
        .expect_err("the same case id must be refused");
    assert!(error.to_string().contains("already exists"));
}

#[test]
fn a_second_production_opens_on_the_same_case() {
    let mut store = empty_store();
    let opened = store
        .open_case(&ProposedCase {
            id: None,
            name: "State v. Hall".to_owned(),
            reference: None,
            jurisdiction: None,
            production: None,
        })
        .expect("case");
    let case_id = CaseId(opened.id.clone());
    let second = store
        .open_production(
            &case_id,
            &ProposedProduction {
                id: None,
                label: "Supplemental".to_owned(),
                received_at: Some("2026-04-01T12:00:00Z".to_owned()),
                producing_party: Some("Prosecution".to_owned()),
                notes: None,
            },
        )
        .expect("production");
    assert_eq!(second.label, "Supplemental");
    assert_eq!(store.productions(&case_id).expect("list").len(), 2);
}

/// Import refuses a speaker, attributed person, or parent content that belongs
/// to another case.
#[test]
fn ingest_refuses_records_from_another_case() {
    let (mut store, hit_run, _) = both_fixtures();
    let opened = store
        .open_case(&ProposedCase {
            id: Some("case-fresh".to_owned()),
            name: "State v. Fresh".to_owned(),
            reference: None,
            jurisdiction: None,
            production: None,
        })
        .expect("fresh case");
    let fresh = CaseId(opened.id.clone());

    let mut speaker = observation_batch(
        fresh.clone(),
        &opened.production.id,
        "src-speaker",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    );
    speaker.sources[0].segments[0].content[0].speaker_entity_id = Some("person-chen".to_owned());
    let error = store
        .import_normalized(&speaker)
        .expect_err("a speaker from another case must be refused");
    assert!(error.to_string().contains("another case"), "{error}");

    let mut parent = observation_batch(
        fresh,
        &opened.production.id,
        "src-parent",
        "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
    );
    parent.sources[0].segments[0].content[0].parent_content_id =
        Some("hr-content-911-injury".to_owned());
    let error = store
        .import_normalized(&parent)
        .expect_err("parent content from another case must be refused");
    assert!(error.to_string().contains("another case"), "{error}");

    // The other case is untouched.
    assert!(store.overview(&hit_run).expect("hit-run").sources > 0);
}

/// The case list is a docket row (name, reference, counts), not a bare pair of
/// identifiers.
#[test]
fn the_docket_lists_each_case_without_the_others_counts() {
    let (mut store, hit_run, vehicle) = both_fixtures();
    store
        .record_entity(
            &hit_run,
            &ProposedEntity {
                id: None,
                kind: evidence_intake::EntityKind::Person,
                display_name: "Unused".to_owned(),
                is_client: false,
                notes: None,
            },
        )
        .expect("entity does not change source counts");

    let docket = store.cases().expect("docket");
    assert_eq!(docket.len(), 2);

    let hit = docket
        .iter()
        .find(|row| row.id == hit_run.0)
        .expect("hit-and-run");
    let stop = docket
        .iter()
        .find(|row| row.id == vehicle.0)
        .expect("vehicle-stop");

    assert_eq!(hit.reference.as_deref(), Some("PD-2026-0118"));
    assert_eq!(stop.reference.as_deref(), Some("PD-2026-0042"));
    assert_ne!(hit.sources, stop.sources);
    assert_eq!(
        hit.sources,
        store.overview(&hit_run).expect("overview").sources
    );
    assert_eq!(
        stop.sources,
        store.overview(&vehicle).expect("overview").sources
    );
    assert!(hit.pending_review > 0);
    assert!(stop.pending_review > 0);
}

#[test]
fn two_cases_may_reuse_a_production_label_but_not_an_identifier() {
    let mut store = empty_store();
    let first = store
        .open_case(&ProposedCase {
            id: Some("case-a".to_owned()),
            name: "A".to_owned(),
            reference: None,
            jurisdiction: None,
            production: Some("Initial production".to_owned()),
        })
        .expect("first");
    let second = store
        .open_case(&ProposedCase {
            id: Some("case-b".to_owned()),
            name: "B".to_owned(),
            reference: None,
            jurisdiction: None,
            production: Some("Initial production".to_owned()),
        })
        .expect("second");
    assert_eq!(first.production.label, second.production.label);
    assert_ne!(first.production.id, second.production.id);

    let error = store
        .open_production(
            &CaseId(second.id),
            &ProposedProduction {
                id: Some(first.production.id.clone()),
                label: "Other disk".to_owned(),
                received_at: None,
                producing_party: None,
                notes: None,
            },
        )
        .expect_err("production ids are globally unique");
    assert!(error.to_string().contains("already exists"));
}
