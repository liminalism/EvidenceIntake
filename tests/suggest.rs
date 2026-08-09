#![allow(missing_docs)]

use evidence_intake::{
    CaseId, DemoFixture, ReviewDecision, ReviewState, ReviewTarget, Store, SuggestionKind,
    SuggestionRun,
};

fn vehicle_stop() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("seed vehicle-stop fixture");
    (store, case_id)
}

fn suggest_all(store: &mut Store, case_id: &CaseId) -> SuggestionRun {
    store
        .suggest(case_id, &SuggestionKind::ALL)
        .expect("run analyzers")
}

/// Every review state in the case, so a test can assert an analyzer changed
/// nothing except by adding suggestions of its own.
fn review_states(store: &Store, case_id: &CaseId) -> Vec<(String, String, String)> {
    store
        .review_queue(case_id)
        .expect("queue")
        .into_iter()
        .map(|item| (item.target_kind, item.target_id, item.review_state))
        .collect()
}

/// A machine may propose. It may not conclude.
#[test]
fn a_suggestion_enters_as_a_machine_suggestion_awaiting_a_person() {
    let (mut store, case_id) = vehicle_stop();
    let run = suggest_all(&mut store, &case_id);
    assert!(run.proposed > 0, "the fixture holds tensions to find");

    let proposed: Vec<_> = run
        .analyzers
        .iter()
        .flat_map(|report| &report.proposed)
        .collect();
    for link in &proposed {
        assert_eq!(
            link.review_state, "suggested",
            "an analyzer may not produce any other state"
        );
        assert!(link.created_by.starts_with("suggest:"));
    }

    let queue = store.review_queue(&case_id).expect("queue");
    for link in &proposed {
        let waiting = queue
            .iter()
            .find(|item| item.target_id == link.id)
            .expect("every suggestion waits in the review queue");
        assert!(
            waiting.machine_generated,
            "a proposed relationship must be visibly machine-generated"
        );
        assert_eq!(
            waiting.extractor.as_deref(),
            Some(link.created_by.as_str()),
            "the queue must name which analyzer proposed it"
        );
    }

    // Machine suggestions sort ahead of hand-entered work.
    let first_human = queue
        .iter()
        .position(|item| !item.machine_generated)
        .expect("hand-entered material");
    assert!(
        queue[..first_human]
            .iter()
            .all(|item| item.machine_generated)
    );
}

#[test]
fn running_the_analyzers_twice_proposes_nothing_new() {
    let (mut store, case_id) = vehicle_stop();
    let first = suggest_all(&mut store, &case_id);
    let second = suggest_all(&mut store, &case_id);

    assert!(first.proposed > 0);
    assert_eq!(second.proposed, 0, "a rerun must not repeat itself");
    assert_eq!(
        second.already_recorded,
        first.proposed + first.already_recorded,
        "everything found the second time was already held"
    );
}

/// A reviewer who has said no does not get asked again.
#[test]
fn a_rejected_suggestion_is_never_proposed_again() {
    let (mut store, case_id) = vehicle_stop();
    let first = suggest_all(&mut store, &case_id);
    let rejected = first
        .analyzers
        .iter()
        .flat_map(|report| &report.proposed)
        .next()
        .expect("at least one suggestion")
        .clone();

    store
        .apply_review(
            &case_id,
            &ReviewDecision {
                target: ReviewTarget::Edge,
                target_id: rejected.id.clone(),
                to_state: ReviewState::Rejected,
                actor: "A. Reyes".to_owned(),
                basis: Some("These describe different moments.".to_owned()),
                verified_against_locator: None,
            },
        )
        .expect("reject the suggestion");

    let again = suggest_all(&mut store, &case_id);
    assert_eq!(
        again.proposed, 0,
        "a rejected proposal must not come back on the next run"
    );
    assert!(
        !store
            .review_queue(&case_id)
            .expect("queue")
            .iter()
            .any(|item| item.target_id == rejected.id),
        "the rejected suggestion has left the queue and stays gone"
    );
}

/// The fixture already records that Patel's two accounts pull apart, in the
/// opposite direction to the one the analyzer would write. That is the same
/// question, already answered.
#[test]
fn an_analyzer_does_not_re_propose_what_a_person_already_recorded() {
    let (mut store, case_id) = vehicle_stop();
    let run = store
        .suggest(&case_id, &[SuggestionKind::ConflictingAttribution])
        .expect("run");

    assert_eq!(run.proposed, 0);
    assert_eq!(
        run.already_recorded, 1,
        "the pair was found and deliberately not written"
    );
}

/// Lanes never collapse. An overlap is a statement about time, not about which
/// account is right.
#[test]
fn overlapping_events_are_proposed_without_collapsing_lanes() {
    let (mut store, case_id) = vehicle_stop();
    let lanes_before = store
        .contested_timeline(&case_id)
        .expect("timeline")
        .into_iter()
        .map(|entry| (entry.id, entry.lane))
        .collect::<Vec<_>>();

    let run = store
        .suggest(&case_id, &[SuggestionKind::TemporalOverlap])
        .expect("run");
    assert!(run.proposed > 0);

    for link in run.analyzers.iter().flat_map(|report| &report.proposed) {
        assert_eq!(link.relation, "temporally_overlaps");
        assert_ne!(link.from_id, link.to_id, "an event does not overlap itself");
    }

    let lanes_after = store
        .contested_timeline(&case_id)
        .expect("timeline")
        .into_iter()
        .map(|entry| (entry.id, entry.lane))
        .collect::<Vec<_>>();
    assert_eq!(lanes_before, lanes_after, "no event changed lane");
}

/// The whole authority of an analyzer is to point at a pair and say why.
#[test]
fn an_analyzer_changes_no_record_a_person_owns() {
    let (mut store, case_id) = vehicle_stop();
    let before = review_states(&store, &case_id);
    let entities_before = store
        .witness_dossier(&case_id, "person-patel")
        .expect("dossier");

    let run = suggest_all(&mut store, &case_id);
    let proposed: Vec<String> = run
        .analyzers
        .iter()
        .flat_map(|report| &report.proposed)
        .map(|link| link.id.clone())
        .collect();

    let after = review_states(&store, &case_id);
    let untouched: Vec<_> = after
        .iter()
        .filter(|(_, id, _)| !proposed.contains(id))
        .cloned()
        .collect();
    assert_eq!(
        before, untouched,
        "an analyzer must add suggestions and alter nothing else"
    );

    // Nothing was merged: the two Patel accounts remain two accounts.
    assert_eq!(
        entities_before.len(),
        store
            .witness_dossier(&case_id, "person-patel")
            .expect("dossier")
            .len()
    );

    // And the trail records no decision, because no decision was made.
    assert!(
        store
            .review_history(&case_id, None)
            .expect("history")
            .is_empty(),
        "an analyzer writes nothing to the review trail"
    );
}

/// A suggestion a defender cannot reconstruct is one they cannot argue with.
#[test]
fn every_suggestion_states_the_reason_it_was_proposed() {
    let (mut store, case_id) = vehicle_stop();
    let run = suggest_all(&mut store, &case_id);

    for link in run.analyzers.iter().flat_map(|report| &report.proposed) {
        assert!(
            link.rationale.len() > 20,
            "`{}` was proposed without a usable reason",
            link.id
        );
        // No score, no confidence, no ranking anywhere in what a machine says.
        let lowered = link.rationale.to_lowercase();
        for forbidden in ["confidence", "score", "likely", "probably", "% "] {
            assert!(
                !lowered.contains(forbidden),
                "`{}` reads as an assessment rather than an observation: {}",
                link.id,
                link.rationale
            );
        }
    }
}

/// One witness changing their account is a question about the witness; two
/// different witnesses disagreeing is a question about the facts. The analyzers
/// do not both claim the same pair.
#[test]
fn the_two_content_analyzers_do_not_claim_the_same_pair() {
    let mut store = Store::in_memory().expect("store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("hit and run");

    let contradictions = store
        .suggest(&case_id, &[SuggestionKind::ContradictionCandidate])
        .expect("run");
    let attributions = store
        .suggest(&case_id, &[SuggestionKind::ConflictingAttribution])
        .expect("run");

    let pair = |run: &SuggestionRun| -> Vec<(String, String)> {
        run.analyzers
            .iter()
            .flat_map(|report| &report.proposed)
            .map(|link| (link.from_id.clone(), link.to_id.clone()))
            .collect()
    };
    for shared in pair(&contradictions) {
        assert!(
            !pair(&attributions).contains(&shared),
            "{shared:?} was proposed by both analyzers"
        );
    }
}

#[test]
fn suggestions_are_scoped_to_their_case() {
    let mut store = Store::in_memory().expect("store");
    let vehicle = DemoFixture::VehicleStop.seed(&mut store).expect("vehicle");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");

    let hit_run_queue_before = store.review_queue(&hit_run).expect("queue").len();
    let run = suggest_all(&mut store, &vehicle);
    assert!(run.proposed > 0);
    assert_eq!(run.case_id, vehicle.0);

    assert_eq!(
        store.review_queue(&hit_run).expect("queue").len(),
        hit_run_queue_before,
        "analyzing one case must not touch another"
    );
}

#[test]
fn analyzing_an_unknown_case_is_refused() {
    let (mut store, _) = vehicle_stop();
    let error = store
        .suggest(&CaseId("no-such-case".to_owned()), &SuggestionKind::ALL)
        .expect_err("an unknown case must be refused");
    assert!(error.to_string().contains("no-such-case"));
}
