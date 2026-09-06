//! Office notes record their author immutably and are revised by superseding.

use office_core::{NoteScope, OfficeFixture, OfficeStore, ProposedNote};

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

/// The public surface offers no way to change a note that already exists.
///
/// The schema refuses an in-place update or delete outright — that is asserted
/// against the migrations themselves, in `store.rs`'s `schema` module. What
/// this proves is the half a caller can see: every path to changed text is a
/// new version, and the words already written stay exactly as they were.
#[test]
fn the_only_way_to_change_a_note_is_to_write_another_version() {
    let mut store = seeded();
    let original = store.note("note-rivera-client").expect("the client note");
    let author = user(&mut store, "A. Defender");

    let revision = store
        .revise_note(
            &original.id,
            &ProposedNote {
                id: None,
                client_id: Some("client-rivera".to_owned()),
                matter_id: None,
                appearance_id: None,
                body: "Reachable after 14:00; new number as of this week.".to_owned(),
                author_user_id: author,
            },
        )
        .expect("write the next version");

    assert_ne!(revision.id, original.id, "a revision is a different record");
    assert_eq!(
        store.note(&original.id).expect("the original").body,
        original.body,
        "the words already written are exactly as they were"
    );
    assert_eq!(
        store.note(&original.id).expect("the original").author,
        original.author,
        "and so is the name on them"
    );
}

/// Revising writes a new version. The old text is still readable through the
/// history and gone from every view, which is what makes an edit visible.
#[test]
fn revising_a_note_supersedes_it_and_views_show_only_the_current_version() {
    let store = seeded();

    let current = store
        .notes_for(NoteScope::Matter, "matter-rivera-1")
        .expect("matter notes");
    assert_eq!(
        current.len(),
        1,
        "one current note, not two versions of one"
    );
    assert_eq!(current[0].id, "note-rivera-plan-v2");
    assert_eq!(current[0].version, 2);
    assert_eq!(
        current[0].supersedes_note_id.as_deref(),
        Some("note-rivera-plan-v1")
    );
    assert!(
        !current[0]
            .body
            .contains("Ask @investigator for the timesheet"),
        "the superseded wording is not what the view shows"
    );

    let history = store.note_history("note-rivera-plan-v1").expect("history");
    assert_eq!(history.versions.len(), 2, "both versions survive");
    assert_eq!(history.current_id, "note-rivera-plan-v2");
    assert_eq!(history.versions[0].id, "note-rivera-plan-v1");
    assert!(
        history.versions[0]
            .body
            .contains("Ask @investigator for the timesheet"),
        "the original words are still readable, with the name of who wrote them"
    );
    assert_eq!(history.versions[0].author, "A. Defender");
}

/// Two revisions of one note would fork its history and leave no single
/// current reading.
#[test]
fn an_already_superseded_note_cannot_be_revised_again() {
    let mut store = seeded();
    let author = user(&mut store, "A. Defender");
    let error = store
        .revise_note(
            "note-rivera-plan-v1",
            &ProposedNote {
                id: None,
                client_id: None,
                matter_id: Some("matter-rivera-1".to_owned()),
                appearance_id: None,
                body: "A second fork of the same version.".to_owned(),
                author_user_id: author,
            },
        )
        .expect_err("revising a superseded version must be refused");
    assert!(
        error.to_string().contains("already been superseded"),
        "{error}"
    );
    assert!(error.to_string().contains("note-rivera-plan-v2"), "{error}");
}

/// A second author revising somebody else's note writes a new version under
/// their own name. The first author's words and attribution are untouched.
#[test]
fn a_second_author_cannot_alter_the_first_authors_note() {
    let mut store = seeded();
    let colleague = user(&mut store, "J. Okafor");

    let revision = store
        .revise_note(
            "note-rivera-plan-v2",
            &ProposedNote {
                id: None,
                client_id: None,
                matter_id: Some("matter-rivera-1".to_owned()),
                appearance_id: None,
                body: "Spoke to the supervisor; she will testify.".to_owned(),
                author_user_id: colleague,
            },
        )
        .expect("a colleague may add a version");

    assert_eq!(revision.author, "J. Okafor", "the new version is theirs");
    assert_eq!(revision.version, 3);

    let original = store
        .note("note-rivera-plan-v2")
        .expect("the earlier version");
    assert_eq!(
        original.author, "A. Defender",
        "the earlier version still names who wrote it"
    );
    assert!(
        original.body.contains("Timesheet obtained"),
        "and still says what they said"
    );

    let history = store.note_history("note-rivera-plan-v1").expect("history");
    let authors: Vec<&str> = history.versions.iter().map(|v| v.author.as_str()).collect();
    assert_eq!(
        authors,
        ["A. Defender", "A. Defender", "J. Okafor"],
        "the chain shows who said what, in order"
    );
}

/// A note belongs to a person, a case, or a setting — never to two of them.
#[test]
fn a_note_belongs_to_exactly_one_of_client_matter_or_appearance() {
    let mut store = seeded();
    let author = user(&mut store, "A. Defender");

    for (client, matter, appearance) in [
        (Some("client-rivera"), Some("matter-rivera-1"), None),
        (None, None, None),
        (
            Some("client-rivera"),
            None,
            Some("appearance-rivera-consolidated"),
        ),
    ] {
        let error = store
            .write_note(&ProposedNote {
                id: None,
                client_id: client.map(str::to_owned),
                matter_id: matter.map(str::to_owned),
                appearance_id: appearance.map(str::to_owned),
                body: "Filed under what, exactly?".to_owned(),
                author_user_id: author.clone(),
            })
            .expect_err("an ambiguous scope must be refused");
        assert!(error.to_string().contains("exactly one"), "{error}");
    }
}

/// A client note follows the person; a matter note stays with the case.
#[test]
fn a_client_note_follows_the_person_and_a_matter_note_stays_with_the_case() {
    let store = seeded();

    let on_person = store
        .notes_for(NoteScope::Client, "client-rivera")
        .expect("client notes");
    assert_eq!(on_person.len(), 1);
    assert!(on_person[0].body.contains("Follows the person"));
    assert_eq!(on_person[0].scope, "client");

    let on_case = store
        .notes_for(NoteScope::Matter, "matter-rivera-1")
        .expect("matter notes");
    assert!(
        !on_case
            .iter()
            .any(|note| note.body.contains("Follows the person")),
        "a client note is not filed under one of that client's cases"
    );

    // The client note is reachable from every matter the person has, because
    // it is attached to them rather than to a case.
    for matter in ["matter-rivera-1", "matter-rivera-2", "matter-rivera-3"] {
        let profile = store.matter_profile(matter).expect("matter");
        assert_eq!(profile.client_id, "client-rivera");
    }
}

/// Mentions are read out of the text once and are queryable across the office.
#[test]
fn mentions_ride_on_a_note_and_are_re_derived_on_revision() {
    let store = seeded();

    let current = store
        .notes_for(NoteScope::Matter, "matter-rivera-1")
        .expect("matter notes");
    assert_eq!(
        current[0].mentions,
        ["investigator", "supervisor"],
        "the current version's mentions come from the current version's words"
    );

    let history = store.note_history("note-rivera-plan-v1").expect("history");
    assert_eq!(
        history.versions[0].mentions,
        ["investigator"],
        "and the first version keeps its own"
    );

    let flagged = store
        .notes_mentioning(office_core::MentionTag::Supervisor)
        .expect("notes mentioning a supervisor");
    assert_eq!(flagged.len(), 1);
    assert_eq!(flagged[0].id, "note-rivera-plan-v2");
    assert!(
        store
            .notes_mentioning(office_core::MentionTag::Immigration)
            .expect("immigration mentions")
            .is_empty(),
        "a role nobody called on has nothing waiting"
    );
}

/// A note has to say something and has to name somebody real.
#[test]
fn a_note_must_say_something_and_name_a_real_author() {
    let mut store = seeded();
    let author = user(&mut store, "A. Defender");

    let error = store
        .write_note(&ProposedNote {
            id: None,
            client_id: Some("client-rivera".to_owned()),
            matter_id: None,
            appearance_id: None,
            body: "   ".to_owned(),
            author_user_id: author,
        })
        .expect_err("an empty note must be refused");
    assert!(error.to_string().contains("must say something"), "{error}");

    let error = store
        .write_note(&ProposedNote {
            id: None,
            client_id: Some("client-rivera".to_owned()),
            matter_id: None,
            appearance_id: None,
            body: "Written by nobody in particular.".to_owned(),
            author_user_id: "user-who-does-not-exist".to_owned(),
        })
        .expect_err("an unknown author must be refused");
    assert!(error.to_string().contains("was not found"), "{error}");
}
