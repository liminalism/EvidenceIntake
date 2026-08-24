#![allow(missing_docs)]

use std::collections::BTreeMap;

use evidence_intake::{
    CaseId, ContentForm, ContentKind, DemoFixture, ExtractionProvenance, InterpretationBatch,
    InterpretationTarget, Materiality, NormalizedBatch, NormalizedContent, NormalizedSegment,
    NormalizedSource, PerceptionBasis, ProposedContentGroup, ProposedInterpretation,
    ProposedSourceProfile, ReviewState, SourceKind, SourceRole, Store, TemporalRelation,
    TemporalStance,
};

fn fixture() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("fixture");
    (store, case_id)
}

fn profile() -> ProposedSourceProfile {
    ProposedSourceProfile {
        id: Some("profile-bodycam-v1".to_owned()),
        source_id: "src-bodycam".to_owned(),
        source_role: SourceRole::BodyCamera,
        author_entity_id: Some("person-chen".to_owned()),
        created_at_claim: Some("2026-01-08T22:18:10".to_owned()),
        default_content_form: Some(ContentForm::RecordedUtterance),
        default_temporal_stance: Some(TemporalStance::ContemporaneousCapture),
        default_perception_basis: Some(PerceptionBasis::Recorded),
        clock_offset_ms: Some(-242_000),
        clock_offset_basis: Some("Compared displayed device time to dispatch clock.".to_owned()),
        review_state: ReviewState::Reviewed,
        created_by: "P. Paralegal".to_owned(),
        supersedes_profile_id: None,
    }
}

fn interpretation(id: &str) -> ProposedInterpretation {
    ProposedInterpretation {
        id: Some(id.to_owned()),
        target: InterpretationTarget::Content {
            id: "content-bodycam-question".to_owned(),
        },
        char_start: None,
        char_end: None,
        content_form: Some(ContentForm::RecordedUtterance),
        perception_basis: Some(PerceptionBasis::Recorded),
        temporal_stance: Some(TemporalStance::ContemporaneousCapture),
        speaker_entity_id: Some("person-chen".to_owned()),
        attributed_entity_id: None,
        reporting_parent_interpretation_id: None,
        content_created_at: None,
        asserted_start: None,
        asserted_end: None,
        normalized_start: None,
        normalized_end: None,
        time_alignment_basis: None,
        location_text: None,
        location_entity_id: None,
        materiality: Materiality::Material,
        field_provenance: BTreeMap::from([
            ("content_form".to_owned(), "entered".to_owned()),
            ("speaker_entity_id".to_owned(), "entered".to_owned()),
        ]),
        basis: None,
        review_state: ReviewState::Reviewed,
        created_by: "P. Paralegal".to_owned(),
        supersedes_interpretation_id: None,
    }
}

#[test]
fn reviewed_source_defaults_are_inherited_without_passage_rows() {
    let (mut store, case_id) = fixture();
    store
        .append_source_profile(&case_id, &profile())
        .expect("profile");

    let effective = store
        .effective_interpretation(
            &case_id,
            &InterpretationTarget::Content {
                id: "content-bodycam-question".to_owned(),
            },
        )
        .expect("effective interpretation");
    assert!(effective.explicit.is_none());
    assert_eq!(effective.content_form, Some(ContentForm::RecordedUtterance));
    assert_eq!(effective.speaker_entity_id.as_deref(), Some("person-chen"));
    assert_eq!(
        effective
            .field_provenance
            .get("content_form")
            .map(String::as_str),
        Some("inherited:profile-bodycam-v1")
    );
}

#[test]
fn interpretations_are_revised_by_superseding_and_keep_history() {
    let (mut store, case_id) = fixture();
    let first = store
        .append_interpretation(&case_id, &interpretation("reading-v1"))
        .expect("first reading");

    let mut second = interpretation("reading-v2");
    second.content_form = Some(ContentForm::QuotedStatement);
    second.supersedes_interpretation_id = Some(first.id.clone());
    let second = store
        .append_interpretation(&case_id, &second)
        .expect("revision");

    let current = store
        .current_interpretation(&case_id, &second.target, None, None)
        .expect("current")
        .expect("current row");
    assert_eq!(current.id, "reading-v2");
    assert_eq!(current.content_form, Some(ContentForm::QuotedStatement));
    let history = store
        .interpretation_history(&case_id, &second.target, None, None)
        .expect("history");
    assert_eq!(history.len(), 2);

    let mut fork = interpretation("reading-fork");
    fork.supersedes_interpretation_id = Some(first.id);
    assert!(
        store
            .append_interpretation(&case_id, &fork)
            .expect_err("superseded head")
            .to_string()
            .contains("already been superseded")
    );
}

#[test]
fn rejected_explicit_reading_falls_back_to_the_reviewed_profile() {
    let (mut store, case_id) = fixture();
    store
        .append_source_profile(&case_id, &profile())
        .expect("profile");
    let mut rejected = interpretation("rejected-reading");
    rejected.content_form = Some(ContentForm::OfficialCharacterization);
    rejected.review_state = ReviewState::Rejected;
    rejected.basis = Some("Cue described a quote in another document.".to_owned());
    store
        .append_interpretation(&case_id, &rejected)
        .expect("rejection");

    let effective = store
        .effective_interpretation(&case_id, &rejected.target)
        .expect("effective");
    assert!(effective.explicit.is_none());
    assert_eq!(effective.content_form, Some(ContentForm::RecordedUtterance));
}

#[test]
fn content_groups_are_ordered_case_local_units_and_spans_stay_on_content() {
    let (mut store, case_id) = fixture();
    let group_id = store
        .append_content_group(
            &case_id,
            &ProposedContentGroup {
                id: Some("group-consent".to_owned()),
                label: Some("Consent exchange".to_owned()),
                content_ids: vec![
                    "content-report-consent".to_owned(),
                    "content-bodycam-question".to_owned(),
                ],
                review_state: ReviewState::Reviewed,
                created_by: "P. Paralegal".to_owned(),
                supersedes_group_id: None,
            },
        )
        .expect("group");
    let mut grouped = interpretation("group-reading");
    grouped.target = InterpretationTarget::ContentGroup { id: group_id };
    grouped.char_start = Some(0);
    grouped.char_end = Some(3);
    assert!(
        store
            .append_interpretation(&case_id, &grouped)
            .expect_err("group span")
            .to_string()
            .contains("cannot carry a character span")
    );
}

#[test]
fn interpretation_batches_are_atomic_and_cannot_self_verify_or_impersonate_rules() {
    let (mut store, case_id) = fixture();
    let mut bad = interpretation("batch-reading");
    bad.review_state = ReviewState::Suggested;
    bad.created_by = "suggest:reported-statement@1".to_owned();
    let batch = InterpretationBatch {
        case_id: case_id.clone(),
        source_profiles: vec![profile()],
        content_groups: Vec::new(),
        interpretations: vec![bad],
    };
    assert!(store.import_interpretations(&batch).is_err());
    assert!(
        store
            .current_source_profile(&case_id, "src-bodycam")
            .expect("query")
            .is_none(),
        "the valid profile must roll back with the invalid interpretation"
    );

    let mut verified = interpretation("verified-batch-reading");
    verified.review_state = ReviewState::Verified;
    let error = store
        .import_interpretations(&InterpretationBatch {
            case_id,
            source_profiles: Vec::new(),
            content_groups: Vec::new(),
            interpretations: vec![verified],
        })
        .expect_err("batch verification");
    assert!(error.to_string().contains("original-locator review path"));
}

#[test]
fn normalized_time_and_clock_offsets_require_written_bases() {
    let (mut store, case_id) = fixture();
    let mut no_clock_basis = profile();
    no_clock_basis.clock_offset_basis = None;
    assert!(
        store
            .append_source_profile(&case_id, &no_clock_basis)
            .expect_err("clock basis")
            .to_string()
            .contains("clock offset")
    );

    let mut no_alignment_basis = interpretation("unaligned-reading");
    no_alignment_basis.normalized_start = Some("2026-01-08T22:14:08Z".to_owned());
    assert!(
        store
            .append_interpretation(&case_id, &no_alignment_basis)
            .expect_err("alignment basis")
            .to_string()
            .contains("alignment basis")
    );
}

#[test]
fn a_later_pass_appends_to_an_identical_source_without_rewriting_it() {
    let (mut store, case_id) = fixture();
    let first = pass(
        &case_id,
        "later-source",
        '9',
        "segment-a",
        "content-a",
        "line 1",
    );
    store.import_normalized(&first).expect("first pass");

    let second = pass(
        &case_id,
        "later-source",
        '9',
        "segment-b",
        "content-b",
        "line 2",
    );
    store.import_normalized(&second).expect("later pass");
    assert_eq!(
        store
            .search(&case_id, "second pass", 10)
            .expect("search")
            .len(),
        1
    );

    let mut mismatch = pass(
        &case_id,
        "later-source",
        '8',
        "segment-c",
        "content-c",
        "line 3",
    );
    mismatch.sources[0].byte_length += 1;
    assert!(
        store
            .import_normalized(&mismatch)
            .expect_err("identity mismatch")
            .to_string()
            .contains("does not exactly match")
    );

    let duplicate = pass(
        &case_id,
        "later-source",
        '9',
        "segment-b",
        "content-d",
        "line 4",
    );
    assert!(
        store
            .import_normalized(&duplicate)
            .expect_err("duplicate segment")
            .to_string()
            .contains("source segment")
    );
}

fn pass(
    case_id: &CaseId,
    source_id: &str,
    hash: char,
    segment_id: &str,
    content_id: &str,
    locator: &str,
) -> NormalizedBatch {
    NormalizedBatch {
        case_id: case_id.clone(),
        edges: Vec::new(),
        sources: vec![NormalizedSource {
            id: source_id.to_owned(),
            production_id: "prod-01".to_owned(),
            logical_name: "later pass.txt".to_owned(),
            media_type: "text/plain".to_owned(),
            source_kind: SourceKind::Document,
            temporal_relation: TemporalRelation::AfterEvent,
            sha256: hash.to_string().repeat(64),
            byte_length: 64,
            segments: vec![NormalizedSegment {
                id: segment_id.to_owned(),
                locator: locator.to_owned(),
                page: None,
                start_ms: None,
                end_ms: None,
                bounding_box: None,
                content: vec![NormalizedContent {
                    id: content_id.to_owned(),
                    kind: ContentKind::Statement,
                    text: if content_id == "content-b" {
                        "Second pass text.".to_owned()
                    } else {
                        "First pass text.".to_owned()
                    },
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
                        extractor: "test".to_owned(),
                        version: "1".to_owned(),
                        machine_generated: true,
                        confidence: None,
                        review_state: ReviewState::Suggested,
                    },
                }],
            }],
        }],
    }
}
