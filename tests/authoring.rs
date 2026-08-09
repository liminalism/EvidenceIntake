#![allow(missing_docs)]

use evidence_intake::{
    CaseId, ChargePosture, DemoFixture, EdgeKind, ElementAssessment, NodeKind, NodeRef,
    ProposedCharge, ProposedElement, ProposedElementMapping, ProposedLink, ProposedProposition,
    Store,
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

fn charge(label: &str, elements: &[&str]) -> ProposedCharge {
    ProposedCharge {
        id: None,
        label: label.to_owned(),
        citation: Some("Example Code § 99-101".to_owned()),
        posture: ChargePosture::Charged,
        grade: Some("misdemeanor".to_owned()),
        elements: elements
            .iter()
            .map(|text| ProposedElement {
                id: None,
                text: (*text).to_owned(),
            })
            .collect(),
    }
}

fn mapping(
    element_id: &str,
    proposition_id: &str,
    assessment: ElementAssessment,
) -> ProposedElementMapping {
    ProposedElementMapping {
        id: None,
        element_id: element_id.to_owned(),
        proposition_id: proposition_id.to_owned(),
        assessment,
        notes: Some("The route and time remain unresolved.".to_owned()),
        author: "A. Reyes".to_owned(),
    }
}

/// Elements have a statutory order, and a gap in it would be a transcription
/// error, so ordinals come from the order given rather than from the caller.
#[test]
fn a_charge_numbers_its_elements_in_statutory_order() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .record_charge(
            &case_id,
            &charge(
                "Leaving the scene",
                &["The defendant drove a vehicle.", "The defendant left."],
            ),
        )
        .expect("record a charge");

    assert_eq!(authored.posture, "charged");
    let ordinals: Vec<u32> = authored.elements.iter().map(|el| el.ordinal).collect();
    assert_eq!(ordinals, vec![1, 2]);
    assert_eq!(authored.elements[0].text, "The defendant drove a vehicle.");
}

#[test]
fn a_charge_without_elements_is_refused() {
    let (mut store, case_id) = hit_and_run();
    let error = store
        .record_charge(&case_id, &charge("Leaving the scene", &[]))
        .expect_err("a charge with no elements must be refused");
    assert!(error.to_string().contains("at least one element"));
}

/// The charge and its elements are one act; a half-written charge would show up
/// in the element matrix as an offense nobody can reason about.
#[test]
fn a_charge_and_its_elements_are_written_together_or_not_at_all() {
    let (mut store, case_id) = hit_and_run();
    let mut proposal = charge("Leaving the scene", &["The defendant drove.", "   "]);
    proposal.id = Some("charge-partial".to_owned());

    let error = store
        .record_charge(&case_id, &proposal)
        .expect_err("an empty element must be refused");
    assert!(error.to_string().contains("must say something"));

    assert!(
        !store
            .element_matrix(&case_id)
            .expect("matrix")
            .iter()
            .any(|row| row.charge == "Leaving the scene"),
        "the refused charge must not have been written"
    );
}

/// An assessment is a direction, not a weight, and it names the person who made it.
#[test]
fn an_element_mapping_records_a_direction_and_its_author() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .record_charge(
            &case_id,
            &charge("Leaving the scene", &["The defendant drove."]),
        )
        .expect("record a charge");

    let mapped = store
        .map_element(
            &case_id,
            &mapping(
                &authored.elements[0].id,
                "hr-prop-client-driver",
                ElementAssessment::Uncertain,
            ),
        )
        .expect("map the element");

    assert_eq!(mapped.assessment, "uncertain");
    assert_eq!(mapped.created_by, "A. Reyes");

    let row = store
        .element_matrix(&case_id)
        .expect("matrix")
        .into_iter()
        .find(|row| row.charge == "Leaving the scene")
        .expect("the new charge appears in the matrix");
    assert_eq!(row.assessment.as_deref(), Some("uncertain"));
    assert_eq!(row.mapped_by.as_deref(), Some("A. Reyes"));
}

#[test]
fn an_element_mapping_requires_a_named_person() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .record_charge(
            &case_id,
            &charge("Leaving the scene", &["The defendant drove."]),
        )
        .expect("record a charge");

    let mut anonymous = mapping(
        &authored.elements[0].id,
        "hr-prop-client-driver",
        ElementAssessment::Supports,
    );
    anonymous.author = "  ".to_owned();
    let error = store
        .map_element(&case_id, &anonymous)
        .expect_err("an unattributed assessment must be refused");
    assert!(error.to_string().contains("must name the person"));
}

/// A proposition bears on an element in one direction. Filing it as both
/// supporting and opposing is not a richer reading but a contradictory one.
#[test]
fn a_proposition_is_mapped_to_an_element_in_one_direction_only() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .record_charge(
            &case_id,
            &charge("Leaving the scene", &["The defendant drove."]),
        )
        .expect("record a charge");
    let element_id = &authored.elements[0].id;

    store
        .map_element(
            &case_id,
            &mapping(
                element_id,
                "hr-prop-client-driver",
                ElementAssessment::Supports,
            ),
        )
        .expect("first assessment");

    let error = store
        .map_element(
            &case_id,
            &mapping(
                element_id,
                "hr-prop-client-driver",
                ElementAssessment::Opposes,
            ),
        )
        .expect_err("a contradictory second assessment must be refused");
    assert!(error.to_string().contains("already assessed `supports`"));
}

#[test]
fn mapping_an_unknown_element_or_proposition_is_refused() {
    let (mut store, case_id) = hit_and_run();
    let error = store
        .map_element(
            &case_id,
            &mapping(
                "no-such-element",
                "hr-prop-client-driver",
                ElementAssessment::Supports,
            ),
        )
        .expect_err("an unknown element must be refused");
    assert!(error.to_string().contains("element `no-such-element`"));

    let authored = store
        .record_charge(
            &case_id,
            &charge("Leaving the scene", &["The defendant drove."]),
        )
        .expect("record a charge");
    let error = store
        .map_element(
            &case_id,
            &mapping(
                &authored.elements[0].id,
                "no-such-proposition",
                ElementAssessment::Supports,
            ),
        )
        .expect_err("an unknown proposition must be refused");
    assert!(error.to_string().contains("no-such-proposition"));
}

/// `element_links` carries no case column of its own, so nothing but an explicit
/// check keeps one case's element matrix from surfacing another case's text.
#[test]
fn an_element_mapping_cannot_reach_a_proposition_in_another_case() {
    let mut store = Store::in_memory().expect("store");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");
    DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle stop");

    let authored = store
        .record_charge(
            &hit_run,
            &charge("Leaving the scene", &["The defendant drove."]),
        )
        .expect("record a charge");

    let error = store
        .map_element(
            &hit_run,
            &mapping(
                &authored.elements[0].id,
                "prop-consent",
                ElementAssessment::Supports,
            ),
        )
        .expect_err("a cross-case mapping must be refused");
    assert!(error.to_string().contains("prop-consent"));

    let row = store
        .element_matrix(&hit_run)
        .expect("matrix")
        .into_iter()
        .find(|row| row.charge == "Leaving the scene")
        .expect("the new charge appears in the matrix");
    assert_eq!(
        row.proposition, None,
        "the refused mapping must not have been written, and the element must \
         show no proposition rather than another case's"
    );
    assert_eq!(row.assessment, None);
}

/// A verified excerpt can be tied to a proposition by a relationship nobody has
/// looked at. The reader has to be able to see that the connection is unchecked.
#[test]
fn proposition_evidence_distinguishes_the_relationship_from_the_content() {
    let (mut store, case_id) = hit_and_run();
    let authored = store
        .author_proposition(
            &case_id,
            &proposition("Morgan did not perceive the impact."),
        )
        .expect("author a proposition");
    store
        .link_evidence(
            &case_id,
            &link(
                content("hr-content-client-driving"),
                EdgeKind::Supports,
                NodeRef::new(NodeKind::Proposition, &authored.id),
            ),
        )
        .expect("link");

    let evidence = store
        .proposition_evidence(&case_id, &authored.id)
        .expect("evidence");
    assert_eq!(evidence[0].relation_review_state, "unreviewed");
    assert_eq!(evidence[0].review_state, "suggested");
}
