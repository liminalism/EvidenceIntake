#![allow(missing_docs)]

use evidence_intake::{CaseId, DemoFixture, ReviewDecision, ReviewState, ReviewTarget, Store};

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("seed hit-and-run fixture");
    (store, case_id)
}

fn decision(target: ReviewTarget, id: &str, to_state: ReviewState) -> ReviewDecision {
    ReviewDecision {
        target,
        target_id: id.to_owned(),
        to_state,
        actor: "A. Reyes".to_owned(),
        basis: None,
        verified_against_locator: None,
    }
}

/// The queue is the point of the intake states: it must surface machine
/// suggestions ahead of hand-entered material and hand over a locator to open.
#[test]
fn queue_puts_machine_suggestions_first_with_an_openable_locator() {
    let (store, case_id) = hit_and_run();
    let queue = store.review_queue(&case_id).expect("review queue");

    let first_human = queue
        .iter()
        .position(|item| !item.machine_generated)
        .expect("hand-entered material awaiting review");
    assert!(
        queue[..first_human]
            .iter()
            .all(|item| item.machine_generated),
        "machine suggestions must sort ahead of hand-entered records"
    );

    let suggestion = queue
        .iter()
        .find(|item| item.machine_generated)
        .expect("a machine suggestion");
    assert_eq!(suggestion.review_state, "suggested");
    assert_eq!(suggestion.target_kind, "content");
    assert!(suggestion.locator.is_some());
    assert!(suggestion.extractor.is_some());
}

#[test]
fn overview_counts_what_is_still_waiting_on_a_person() {
    let (mut store, case_id) = hit_and_run();
    let before = store.overview(&case_id).expect("overview");
    let queued = store.review_queue(&case_id).expect("queue").len();
    assert_eq!(
        usize::try_from(before.pending_review).expect("fits"),
        queued
    );
    assert!(before.pending_review > 0);

    let item = store.review_queue(&case_id).expect("queue")[0].clone();
    let mut call = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Reviewed,
    );
    call.basis = Some("Listened to the cited segment.".to_owned());
    store.apply_review(&case_id, &call).expect("review");

    let after = store.overview(&case_id).expect("overview");
    assert_eq!(after.pending_review, before.pending_review - 1);
}

/// Verification is a claim about an original, so it must cite that original.
#[test]
fn verifying_content_requires_the_records_own_locator() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();
    let locator = item.locator.clone().expect("content locator");

    let bare = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Verified,
    );
    let error = store
        .apply_review(&case_id, &bare)
        .expect_err("verification without a locator must be refused");
    assert!(error.to_string().contains("requires citing the original"));

    let mut wrong = bare.clone();
    wrong.verified_against_locator = Some("another exhibit @ page 4".to_owned());
    let error = store
        .apply_review(&case_id, &wrong)
        .expect_err("verification against the wrong original must be refused");
    assert!(error.to_string().contains("but its original locator is"));

    let mut correct = bare;
    correct.verified_against_locator = Some(locator.clone());
    let event = store.apply_review(&case_id, &correct).expect("verify");
    assert_eq!(event.to_state, "verified");
    assert_eq!(event.from_state, item.review_state);
    assert_eq!(event.verified_against_locator, Some(locator));
}

/// A relationship is an attorney judgment spanning sources; there is no single
/// original to open, so the reviewer must say what they compared instead.
#[test]
fn verifying_a_relationship_requires_a_written_basis_instead() {
    let (mut store, case_id) = hit_and_run();
    let edge = store
        .review_queue(&case_id)
        .expect("queue")
        .into_iter()
        .find(|item| item.target_kind == "edge")
        .expect("an edge awaiting review");
    assert!(edge.locator.is_none());

    let bare = decision(ReviewTarget::Edge, &edge.target_id, ReviewState::Verified);
    let error = store
        .apply_review(&case_id, &bare)
        .expect_err("bare verification of a relationship must be refused");
    assert!(error.to_string().contains("requires a written basis"));

    let mut reasoned = bare;
    reasoned.basis = Some("Compared both cited excerpts.".to_owned());
    let event = store.apply_review(&case_id, &reasoned).expect("verify");
    assert_eq!(event.to_state, "verified");
}

/// The trail must not record a locator nobody could have opened. A relationship
/// has no original, so citing one is a claim about an artifact that is not there.
#[test]
fn verifying_a_relationship_refuses_a_cited_locator() {
    let (mut store, case_id) = hit_and_run();
    let edge = store
        .review_queue(&case_id)
        .expect("queue")
        .into_iter()
        .find(|item| item.target_kind == "edge")
        .expect("an edge awaiting review");

    let mut call = decision(ReviewTarget::Edge, &edge.target_id, ReviewState::Verified);
    call.basis = Some("Compared both cited excerpts.".to_owned());
    call.verified_against_locator = Some("Officer Chen report.pdf @ page 3".to_owned());

    let error = store
        .apply_review(&case_id, &call)
        .expect_err("a relationship has no original to cite");
    assert!(error.to_string().contains("has no original to cite"));
    assert!(
        store
            .review_history(&case_id, Some(&edge.target_id))
            .expect("history")
            .is_empty(),
        "a refused decision must leave no trace in the trail"
    );
}

#[test]
fn rejecting_evidence_requires_a_written_reason() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();

    let bare = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Rejected,
    );
    let error = store
        .apply_review(&case_id, &bare)
        .expect_err("silent rejection must be refused");
    assert!(error.to_string().contains("requires a written reason"));
}

#[test]
fn a_reviewer_cannot_return_a_record_to_an_intake_state() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();

    for state in [ReviewState::Unreviewed, ReviewState::Suggested] {
        let call = decision(ReviewTarget::Content, &item.target_id, state);
        let error = store
            .apply_review(&case_id, &call)
            .expect_err("intake states are not reviewer decisions");
        assert!(error.to_string().contains("produced by import"));
    }
}

#[test]
fn a_decision_must_name_the_person_making_it() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();

    let mut call = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Reviewed,
    );
    call.actor = "   ".to_owned();
    let error = store
        .apply_review(&case_id, &call)
        .expect_err("anonymous review must be refused");
    assert!(error.to_string().contains("must name the person"));
}

/// Later material can undo an earlier reading, but the earlier reading stays.
#[test]
fn history_is_append_only_and_records_a_withdrawn_verification() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();
    let locator = item.locator.clone().expect("content locator");

    let mut verify = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Verified,
    );
    verify.verified_against_locator = Some(locator);
    store.apply_review(&case_id, &verify).expect("verify");

    let mut withdraw = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Rejected,
    );
    withdraw.basis = Some("Supplemental production shows a mislabel.".to_owned());
    store.apply_review(&case_id, &withdraw).expect("withdraw");

    let history = store
        .review_history(&case_id, Some(&item.target_id))
        .expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].to_state, "verified");
    assert_eq!(history[1].from_state, "verified");
    assert_eq!(history[1].to_state, "rejected");
    assert!(history.iter().all(|event| event.actor == "A. Reyes"));
}

#[test]
fn repeating_a_decision_is_refused_rather_than_duplicating_the_trail() {
    let (mut store, case_id) = hit_and_run();
    let item = store.review_queue(&case_id).expect("queue")[0].clone();

    let mut call = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Reviewed,
    );
    call.basis = Some("Read the cited paragraph.".to_owned());
    store.apply_review(&case_id, &call).expect("first review");
    let error = store
        .apply_review(&case_id, &call)
        .expect_err("a no-op decision must be refused");
    assert!(error.to_string().contains("already holds that state"));

    let history = store
        .review_history(&case_id, Some(&item.target_id))
        .expect("history");
    assert_eq!(history.len(), 1);
}

#[test]
fn an_unknown_record_is_not_silently_created() {
    let (mut store, case_id) = hit_and_run();
    let call = decision(
        ReviewTarget::Content,
        "no-such-content",
        ReviewState::Reviewed,
    );
    let error = store
        .apply_review(&case_id, &call)
        .expect_err("unknown review target");
    assert!(error.to_string().contains("review target"));
    assert!(store.review_history(&case_id, None).expect("h").is_empty());
}

/// A record belongs to one case; review must not reach across the boundary.
#[test]
fn review_is_scoped_to_its_case() {
    let mut store = Store::in_memory().expect("store");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");
    let vehicle_stop = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle stop");
    let item = store.review_queue(&hit_run).expect("queue")[0].clone();

    let mut call = decision(
        ReviewTarget::Content,
        &item.target_id,
        ReviewState::Reviewed,
    );
    call.basis = Some("Read the cited paragraph.".to_owned());
    let error = store
        .apply_review(&vehicle_stop, &call)
        .expect_err("cross-case review must be refused");
    assert!(error.to_string().contains("review target"));
}
