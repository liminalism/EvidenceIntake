#![allow(missing_docs)]

use evidence_intake::{
    CaseId, DemoFixture, EdgeKind, NodeKind, NodeRef, ProposedLink, ProposedProposition, Store,
};

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("seed hit-and-run fixture");
    (store, case_id)
}

fn proposition(text: &str) -> ProposedProposition {
    ProposedProposition {
        id: None,
        text: text.to_owned(),
        author: "A. Reyes".to_owned(),
    }
}

fn link(from: NodeRef, relation: EdgeKind, to: NodeRef) -> ProposedLink {
    ProposedLink {
        id: None,
        from,
        relation,
        to,
        rationale: "The caller describes the driver swerving before impact.".to_owned(),
        author: "A. Reyes".to_owned(),
    }
}

fn content(id: &str) -> NodeRef {
    NodeRef::new(NodeKind::Content, id)
}

/// Writing a proposition down is not evidence that anyone checked it, and it is
/// not a finding that the evidence has settled.
#[test]
fn an_authored_proposition_enters_unreviewed_and_contested() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .author_proposition(
            &case_id,
            &proposition("Morgan did not perceive the impact."),
        )
        .expect("author a proposition");

    assert_eq!(authored.review_state, "unreviewed");
    assert_eq!(authored.status, "contested");
    assert_eq!(authored.created_by, "A. Reyes");
    assert!(!authored.id.is_empty());
}

#[test]
fn authoring_requires_a_named_person() {
    let (mut store, case_id) = hit_and_run();

    let mut anonymous = proposition("Morgan did not perceive the impact.");
    anonymous.author = "   ".to_owned();
    let error = store
        .author_proposition(&case_id, &anonymous)
        .expect_err("anonymous authoring must be refused");
    assert!(error.to_string().contains("must name the person"));

    let mut unsigned = link(
        content("hr-content-911-injury"),
        EdgeKind::Supports,
        content("hr-content-victim-impact"),
    );
    unsigned.author = String::new();
    let error = store
        .link_evidence(&case_id, &unsigned)
        .expect_err("an unsigned relationship must be refused");
    assert!(error.to_string().contains("must name the person"));
}

#[test]
fn a_proposition_must_say_something() {
    let (mut store, case_id) = hit_and_run();
    let error = store
        .author_proposition(&case_id, &proposition("  \n "))
        .expect_err("an empty proposition must be refused");
    assert!(error.to_string().contains("must say something"));
}

/// A relationship spans sources and has no original of its own, so the written
/// reason is the only thing a later reader can weigh.
#[test]
fn a_link_requires_a_written_rationale() {
    let (mut store, case_id) = hit_and_run();
    let mut bare = link(
        content("hr-content-911-injury"),
        EdgeKind::Contradicts,
        content("hr-content-victim-injury"),
    );
    bare.rationale = "   ".to_owned();

    let error = store
        .link_evidence(&case_id, &bare)
        .expect_err("a silent relationship must be refused");
    assert!(error.to_string().contains("requires a written rationale"));
}

#[test]
fn linking_an_unknown_node_is_refused_rather_than_created() {
    let (mut store, case_id) = hit_and_run();
    let proposal = link(
        content("no-such-content"),
        EdgeKind::Supports,
        NodeRef::new(NodeKind::Proposition, "hr-prop-collision"),
    );

    let error = store
        .link_evidence(&case_id, &proposal)
        .expect_err("an unknown endpoint must be refused");
    assert!(error.to_string().contains("no-such-content"));
    assert!(error.to_string().contains("not found"));
}

/// A record belongs to one case; a relationship must not reach across.
#[test]
fn a_link_cannot_reach_a_node_in_another_case() {
    let mut store = Store::in_memory().expect("store");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");
    let vehicle_stop = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle stop");

    let proposal = link(
        content("hr-content-911-injury"),
        EdgeKind::Supports,
        NodeRef::new(NodeKind::Proposition, "prop-consent"),
    );

    let error = store
        .link_evidence(&hit_run, &proposal)
        .expect_err("a cross-case relationship must be refused");
    assert!(error.to_string().contains("prop-consent"));

    let error = store
        .link_evidence(&vehicle_stop, &proposal)
        .expect_err("the other direction is equally refused");
    assert!(error.to_string().contains("hr-content-911-injury"));
}

/// The same claim written twice would stand in front of a reviewer as two
/// separate assertions and be counted twice in every view.
#[test]
fn repeating_a_link_is_refused_rather_than_duplicating_it() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .author_proposition(
            &case_id,
            &proposition("Morgan did not perceive the impact."),
        )
        .expect("author a proposition");
    let target = NodeRef::new(NodeKind::Proposition, &authored.id);

    let proposal = link(
        content("hr-content-client-driving"),
        EdgeKind::Supports,
        target.clone(),
    );
    store
        .link_evidence(&case_id, &proposal)
        .expect("first link");

    let mut again = proposal.clone();
    again.rationale = "A differently worded reason for the same claim.".to_owned();
    let error = store
        .link_evidence(&case_id, &again)
        .expect_err("the same claim twice must be refused");
    assert!(error.to_string().contains("already exists"));

    assert_eq!(
        store
            .proposition_evidence(&case_id, &authored.id)
            .expect("evidence")
            .len(),
        1,
        "the refused assertion must not have been written"
    );
}

#[test]
fn a_supplied_identifier_is_never_silently_reused() {
    let (mut store, case_id) = hit_and_run();
    let mut taken = proposition("Morgan did not perceive the impact.");
    taken.id = Some("hr-prop-collision".to_owned());

    let error = store
        .author_proposition(&case_id, &taken)
        .expect_err("an identifier already in use must be refused");
    assert!(error.to_string().contains("already exists"));
}

#[test]
fn nothing_may_stand_in_a_relationship_to_itself() {
    let (mut store, case_id) = hit_and_run();
    let proposal = link(
        content("hr-content-911-injury"),
        EdgeKind::Corroborates,
        content("hr-content-911-injury"),
    );

    let error = store
        .link_evidence(&case_id, &proposal)
        .expect_err("a self-relationship must be refused");
    assert!(error.to_string().contains("relationship to itself"));
}

/// Authored records join the same queue as everything else, because authoring
/// is not review.
#[test]
fn authored_records_wait_in_the_review_queue() {
    let (mut store, case_id) = hit_and_run();
    let before = store.overview(&case_id).expect("overview").pending_review;

    let authored = store
        .author_proposition(
            &case_id,
            &proposition("Morgan did not perceive the impact."),
        )
        .expect("author a proposition");
    let linked = store
        .link_evidence(
            &case_id,
            &link(
                content("hr-content-client-driving"),
                EdgeKind::Supports,
                NodeRef::new(NodeKind::Proposition, &authored.id),
            ),
        )
        .expect("link evidence");

    let queue = store.review_queue(&case_id).expect("queue");
    let waiting = |id: &str| {
        queue
            .iter()
            .find(|item| item.target_id == id)
            .unwrap_or_else(|| panic!("`{id}` must be waiting on a person"))
    };
    assert_eq!(waiting(&authored.id).review_state, "unreviewed");
    assert!(!waiting(&authored.id).machine_generated);
    assert_eq!(waiting(&linked.id).review_state, "unreviewed");

    assert_eq!(
        store.overview(&case_id).expect("overview").pending_review,
        before + 2
    );
}

/// The round trip that makes authoring worth having: a proposition a person
/// wrote resolves back to an exact locator in an original.
#[test]
fn authored_evidence_appears_under_the_proposition_it_supports() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .author_proposition(
            &case_id,
            &proposition("Morgan did not perceive the impact."),
        )
        .expect("author a proposition");

    let mut proposal = link(
        content("hr-content-client-driving"),
        EdgeKind::Supports,
        NodeRef::new(NodeKind::Proposition, &authored.id),
    );
    proposal.rationale = "Morgan expressly disputes awareness of any impact.".to_owned();
    store.link_evidence(&case_id, &proposal).expect("link");

    let evidence = store
        .proposition_evidence(&case_id, &authored.id)
        .expect("proposition evidence");
    assert_eq!(evidence.len(), 1);
    let item = &evidence[0];
    assert_eq!(item.relation, "supports");
    // The state shown is the underlying content's, not the relationship's; the
    // link a person just asserted is still `unreviewed` in the queue.
    assert_eq!(item.review_state, "suggested");
    assert_eq!(
        item.rationale.as_deref(),
        Some("Morgan expressly disputes awareness of any impact.")
    );
    assert!(
        !item.locator.trim().is_empty(),
        "authored evidence must still resolve to an exact original"
    );
}
