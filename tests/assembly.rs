#![allow(missing_docs)]

use evidence_intake::{
    CaseId, DemoFixture, EdgeKind, NodeKind, NodeRef, ProposedLink, ProposedOccurrence,
    ReviewDecision, ReviewState, ReviewTarget, Store, TimelineLane,
};

fn fixture() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::VehicleStop.seed(&mut store).expect("fixture");
    (store, case_id)
}

fn occurrence() -> ProposedOccurrence {
    ProposedOccurrence {
        id: None,
        label: "Consent exchange at the driver's window".to_owned(),
        lane: TimelineLane::Recorded,
        raw_time: Some("22:18".to_owned()),
        normalized_start: None,
        normalized_end: None,
        time_basis: None,
        location_text: Some("Roadside, eastbound shoulder".to_owned()),
        proposition_id: None,
        passage_ids: vec![
            "content-bodycam-question".to_owned(),
            "content-report-consent".to_owned(),
        ],
        rationale: "Both passages describe the same exchange at the window, seconds apart."
            .to_owned(),
        author: "P. Paralegal".to_owned(),
    }
}

#[test]
fn assembling_accounts_writes_one_event_and_unreviewed_account_links() {
    let (mut store, case_id) = fixture();
    let authored = store
        .author_occurrence(&case_id, &occurrence())
        .expect("occurrence");

    assert_eq!(authored.account_links.len(), 2);
    assert!(
        authored
            .account_links
            .iter()
            .all(|link| link.review_state == "unreviewed"
                && link.relation == EdgeKind::AccountOf.as_str()
                && link.to_id == authored.event_id
                && link.created_by == "P. Paralegal"),
        "authoring an occurrence is not review: {:?}",
        authored.account_links
    );

    let timeline = store.contested_timeline(&case_id).expect("timeline");
    assert!(
        timeline
            .iter()
            .any(|entry| entry.label == "Consent exchange at the driver's window"),
        "the assembled occurrence should appear in the timeline"
    );
}

#[test]
fn an_occurrence_needs_at_least_two_accounts_and_a_written_reason() {
    let (mut store, case_id) = fixture();

    let mut single = occurrence();
    single.passage_ids = vec!["content-bodycam-question".to_owned()];
    assert!(store.author_occurrence(&case_id, &single).is_err());

    let mut silent = occurrence();
    silent.rationale = "   ".to_owned();
    assert!(store.author_occurrence(&case_id, &silent).is_err());

    let mut anonymous = occurrence();
    anonymous.author = String::new();
    assert!(store.author_occurrence(&case_id, &anonymous).is_err());
}

#[test]
fn a_normalized_occurrence_time_requires_its_alignment_basis() {
    let (mut store, case_id) = fixture();
    let mut proposal = occurrence();
    proposal.normalized_start = Some("2026-01-08T22:18:10".to_owned());

    assert!(store.author_occurrence(&case_id, &proposal).is_err());

    proposal.time_basis = Some("Body-camera clock compared to dispatch.".to_owned());
    assert!(store.author_occurrence(&case_id, &proposal).is_ok());
}

#[test]
fn a_rejected_same_occurrence_candidate_blocks_the_grouping() {
    let (mut store, case_id) = fixture();
    let link = store
        .link_evidence(
            &case_id,
            &ProposedLink {
                id: Some("edge-same-occurrence".to_owned()),
                from: NodeRef {
                    kind: NodeKind::Content,
                    id: "content-bodycam-question".to_owned(),
                },
                relation: EdgeKind::CandidateSameOccurrence,
                to: NodeRef {
                    kind: NodeKind::Content,
                    id: "content-report-consent".to_owned(),
                },
                rationale: "Same reviewed date and roadside location.".to_owned(),
                author: "P. Paralegal".to_owned(),
            },
        )
        .expect("candidate link");
    store
        .apply_review(
            &case_id,
            &ReviewDecision {
                target: ReviewTarget::Edge,
                target_id: link.id,
                to_state: ReviewState::Rejected,
                actor: "D. Counsel".to_owned(),
                basis: Some(
                    "The report describes a later conversation inside the cruiser.".to_owned(),
                ),
                verified_against_locator: None,
            },
        )
        .expect("rejection");

    let error = store
        .author_occurrence(&case_id, &occurrence())
        .expect_err("a rejected candidate must block the grouping");
    assert!(
        error.to_string().contains("rejected"),
        "unexpected error: {error}"
    );
}
