//! A client is tied to an evidence entity only by a person deciding it is so.

use office_core::{
    ContactKind, IdentityLinkState, OfficeFixture, OfficeStore, ProposedClient,
    ProposedClientContact, ProposedIdentityLinkDecision,
};

const ANCHOR: &str = "2026-08-31";

fn seeded() -> OfficeStore {
    let mut store = OfficeStore::in_memory().expect("in-memory store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut store, ANCHOR)
        .expect("seed the misdemeanor docket");
    store
}

fn user(store: &mut OfficeStore, name: &str) -> String {
    store.user_named(name, "attorney").expect("user")
}

/// The whole of the conservative-identity rule. The office notices a
/// resemblance and says so; it does not act on one.
#[test]
fn a_similar_name_is_offered_as_a_candidate_and_never_merged() {
    let store = seeded();

    let candidates = store
        .possible_client_duplicates(
            "Alex Rivera",
            &["555-481-2290".to_owned()],
            Some("client-rivera-possible"),
        )
        .expect("look for people the office may already know");

    assert!(
        candidates
            .iter()
            .any(|person| person.client_id == "client-rivera"),
        "the existing person is offered: {candidates:#?}"
    );
    let existing = candidates
        .iter()
        .find(|person| person.client_id == "client-rivera")
        .expect("the candidate");
    assert!(
        existing
            .matched_on
            .iter()
            .any(|why| why.starts_with("name:")),
        "and says the name matched: {:?}",
        existing.matched_on
    );
    assert!(
        existing
            .matched_on
            .iter()
            .any(|why| why.starts_with("contact:")),
        "and that the number did too, however it was punctuated: {:?}",
        existing.matched_on
    );
    assert_eq!(existing.matters, 3, "with enough context to judge it");

    // Nothing has been merged, written, or decided. Both records stand.
    assert_eq!(
        store
            .client_profile("client-rivera")
            .expect("first")
            .display_name,
        "Alex Rivera"
    );
    assert_eq!(
        store
            .client_profile("client-rivera-possible")
            .expect("second")
            .display_name,
        "Alex Rivera"
    );
    assert_eq!(
        store
            .client_profile("client-rivera-possible")
            .expect("second")
            .matters
            .len(),
        0,
        "the newer record kept its own emptiness rather than inheriting matters"
    );
}

/// A resemblance found at intake is a question. Writing the client anyway is
/// the correct behaviour, because refusing would merge two people by name.
#[test]
fn a_person_who_resembles_an_existing_client_is_still_written() {
    let mut store = seeded();
    let author = user(&mut store, "R. Ocampo");

    let written = store
        .create_client(&ProposedClient {
            id: Some("client-third-rivera".to_owned()),
            display_name: "Alex Rivera".to_owned(),
            date_of_birth: Some("1971-07-04".to_owned()),
            sex: None,
            preferred_language: None,
            notes: None,
            aliases: Vec::new(),
            contacts: vec![ProposedClientContact {
                kind: ContactKind::Phone,
                value: "(555) 481-2290".to_owned(),
                label: None,
                is_primary: true,
            }],
            author_user_id: author,
        })
        .expect("a third person with the same name is still a person");

    assert_eq!(written.id, "client-third-rivera");
    let candidates = store
        .possible_client_duplicates(
            "Alex Rivera",
            &["(555) 481-2290".to_owned()],
            Some("client-third-rivera"),
        )
        .expect("candidates");
    assert!(
        candidates.len() >= 2,
        "and every earlier one is offered for a person to judge: {candidates:#?}"
    );
}

/// Declining changes nothing about either record. It only stops the asking.
#[test]
fn declining_a_prompt_leaves_both_records_untouched() {
    let store = seeded();
    let before = store.client_profile("client-rivera").expect("before");

    store
        .decide_identity_link(&ProposedIdentityLinkDecision {
            client_id: "client-rivera".to_owned(),
            evidence_case_id: "case-hit-run-001".to_owned(),
            evidence_entity_id: "hr-entity-witness".to_owned(),
            state: IdentityLinkState::Dismissed,
            matched_on: Some("name: Alex Rivera / A. Rivera".to_owned()),
            author_user_id: store.users().expect("users")[0].0.clone(),
        })
        .expect("record the decision");

    let after = store.client_profile("client-rivera").expect("after");
    assert_eq!(before, after, "declining altered nothing about the client");
    assert!(
        store
            .linked_entities("client-rivera")
            .expect("links")
            .is_empty(),
        "and tied it to nothing"
    );
}

/// A declined prompt is not offered again, which is the only reason the
/// dismissal is stored at all.
#[test]
fn a_dismissed_identity_prompt_is_not_offered_again() {
    let store = seeded();
    let author = store.users().expect("users")[0].0.clone();

    assert_eq!(
        store
            .identity_link_state("client-rivera", "case-hit-run-001", "hr-entity-driver")
            .expect("state"),
        None,
        "nobody has been asked yet, so the prompt is live"
    );

    store
        .decide_identity_link(&ProposedIdentityLinkDecision {
            client_id: "client-rivera".to_owned(),
            evidence_case_id: "case-hit-run-001".to_owned(),
            evidence_entity_id: "hr-entity-driver".to_owned(),
            state: IdentityLinkState::Dismissed,
            matched_on: None,
            author_user_id: author.clone(),
        })
        .expect("decline it");

    assert_eq!(
        store
            .identity_link_state("client-rivera", "case-hit-run-001", "hr-entity-driver")
            .expect("state"),
        Some(IdentityLinkState::Dismissed),
        "the office remembers being told no"
    );

    // And a person may still change their mind later, which a note may not do.
    store
        .decide_identity_link(&ProposedIdentityLinkDecision {
            client_id: "client-rivera".to_owned(),
            evidence_case_id: "case-hit-run-001".to_owned(),
            evidence_entity_id: "hr-entity-driver".to_owned(),
            state: IdentityLinkState::Linked,
            matched_on: Some("Confirmed with the client on the 3rd.".to_owned()),
            author_user_id: author,
        })
        .expect("revisit the decision");
    assert_eq!(
        store
            .identity_link_state("client-rivera", "case-hit-run-001", "hr-entity-driver")
            .expect("state"),
        Some(IdentityLinkState::Linked)
    );
    assert_eq!(
        store.linked_entities("client-rivera").expect("links"),
        vec![("case-hit-run-001".to_owned(), "hr-entity-driver".to_owned())]
    );
}

/// A decision names whoever made it, and there is no way to record one that
/// does not.
#[test]
fn an_identity_decision_must_name_the_person_who_made_it() {
    let store = seeded();
    let error = store
        .decide_identity_link(&ProposedIdentityLinkDecision {
            client_id: "client-rivera".to_owned(),
            evidence_case_id: "case-hit-run-001".to_owned(),
            evidence_entity_id: "hr-entity-driver".to_owned(),
            state: IdentityLinkState::Linked,
            matched_on: None,
            author_user_id: "nobody-in-particular".to_owned(),
        })
        .expect_err("an unattributed decision must be refused");
    assert!(error.to_string().contains("was not found"), "{error}");
}

/// The office never invents a resemblance where there is none.
#[test]
fn an_unrelated_person_raises_no_prompt() {
    let store = seeded();
    let candidates = store
        .possible_client_duplicates(
            "Wilhelmina Oyelaran-Achebe",
            &["(555) 000-0000".to_owned()],
            None,
        )
        .expect("candidates");
    assert!(
        candidates.is_empty(),
        "nothing obvious matched, and nothing was invented: {candidates:#?}"
    );
}

/// An empty candidate list is not a claim that the person is new.
///
/// Nothing in the office layer records "checked, and this is definitely a new
/// person", because nothing can know that. Only decisions about specific pairs
/// are ever written.
#[test]
fn finding_no_candidate_writes_nothing() {
    let store = seeded();
    let before = store.clients().expect("clients").len();
    store
        .possible_client_duplicates("Nobody At All", &[], None)
        .expect("candidates");
    assert_eq!(
        store.clients().expect("clients").len(),
        before,
        "asking the question changed nothing"
    );
}
