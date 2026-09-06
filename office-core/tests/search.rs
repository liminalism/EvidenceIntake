//! Office search answers what evidence search deliberately cannot.

use office_core::{ContactKind, NoteScope, OfficeFixture, OfficeStore, ProposedClientContact};

const ANCHOR: &str = "2026-08-31";

fn seeded() -> OfficeStore {
    let mut store = OfficeStore::in_memory().expect("in-memory store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut store, ANCHOR)
        .expect("seed the misdemeanor docket");
    store
}

/// Evidence search is case-isolated by design. This is the question it cannot
/// answer: where does this person appear across everything the office holds?
#[test]
fn office_search_finds_a_client_across_matters() {
    let store = seeded();
    let hits = store.search("Rivera", 25).expect("search for a person");

    let kinds: Vec<&str> = hits.iter().map(|hit| hit.kind.as_str()).collect();
    assert!(kinds.contains(&"client"), "the person: {kinds:?}");
    assert!(kinds.contains(&"matter"), "and their cases: {kinds:?}");

    let matters: Vec<&str> = hits
        .iter()
        .filter(|hit| hit.kind == "matter")
        .map(|hit| hit.title.as_str())
        .collect();
    assert!(
        matters.len() >= 3,
        "all three of Rivera's matters are reachable by the name: {matters:?}"
    );
    assert!(
        hits.iter()
            .filter(|hit| hit.kind == "matter")
            .all(|hit| hit.client.as_deref() == Some("Alex Rivera")),
        "and every one names whose it is"
    );
}

/// A tokenizer that splits on punctuation cannot match an unpunctuated query
/// against a punctuated record, so the index carries both forms of a number.
#[test]
fn office_search_finds_a_client_by_phone_number() {
    let store = seeded();

    for query in ["(555) 481-2290", "555-481-2290", "5554812290"] {
        let hits = store.search(query, 25).expect("search by number");
        assert!(
            hits.iter()
                .any(|hit| hit.kind == "client" && hit.title == "Alex Rivera"),
            "a person is reachable by their number however it is typed: {query:?} found {hits:#?}"
        );
    }
}

/// A number added after the client record was written is indexed too.
#[test]
fn a_contact_added_later_is_searchable() {
    let store = seeded();
    store
        .add_client_contact(
            "client-okonkwo",
            &ProposedClientContact {
                kind: ContactKind::Phone,
                value: "(555) 733-0186".to_owned(),
                label: Some("jail line".to_owned()),
                is_primary: true,
            },
        )
        .expect("add a number");

    let hits = store.search("5557330186", 10).expect("search by number");
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "client" && hit.title == "Ngozi Okonkwo"),
        "the index follows the write: {hits:#?}"
    );
}

/// The number that gets read aloud in court.
#[test]
fn office_search_finds_a_matter_by_court_number() {
    let store = seeded();
    let hits = store
        .search("CR-2026-491", 10)
        .expect("search by court number");
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "matter" && hit.title == "State v. Rivera (theft)"),
        "a court number reaches its matter: {hits:#?}"
    );
}

/// A colleague's client, which is the point of an office-wide index rather
/// than a per-user one.
#[test]
fn office_search_reaches_a_colleagues_client() {
    let store = seeded();
    let hits = store
        .search("Okonkwo", 10)
        .expect("search for another client");
    assert!(
        hits.iter().any(|hit| hit.kind == "client"),
        "every client is reachable, not only one's own: {hits:#?}"
    );
}

/// Notes are indexed with everything else, so a phrase somebody wrote down is
/// findable months later.
#[test]
fn office_search_reaches_what_somebody_wrote_in_a_note() {
    let store = seeded();
    let hits = store.search("timesheet", 10).expect("search notes");
    assert!(
        hits.iter().any(|hit| hit.kind == "note"),
        "the note is reachable by its words: {hits:#?}"
    );
}

/// A superseded note is out of every view. It is still in the database and
/// still in its own history — but a search that surfaced the old wording
/// beside the new one would put a reader back where the revision took them out.
#[test]
fn a_revised_note_is_found_by_its_current_words() {
    let store = seeded();
    let current = store
        .notes_for(NoteScope::Matter, "matter-rivera-1")
        .expect("matter notes");
    assert_eq!(current.len(), 1);

    let hits = store
        .search("supervisor", 25)
        .expect("search the new wording");
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "note" && hit.subject_id == current[0].id),
        "the current version is findable by what it now says: {hits:#?}"
    );
}

/// Search spans the office and nothing else. There is no path from here into
/// the evidence database, because this crate has no connection to one.
#[test]
fn office_search_reaches_office_records_only() {
    let store = seeded();
    let hits = store
        .search("Rivera OR Okonkwo OR District", 50)
        .expect("search");
    for hit in &hits {
        assert!(
            ["client", "matter", "note", "user", "court"].contains(&hit.kind.as_str()),
            "an office index returns office records, not {:?}",
            hit.kind
        );
    }
}

/// A search needs a term.
#[test]
fn an_empty_search_is_refused_rather_than_returning_the_office() {
    let store = seeded();
    let error = store
        .search("   ", 10)
        .expect_err("an empty query must be refused");
    assert!(error.to_string().contains("needs a term"), "{error}");
}

/// A person typing a case number is not writing a query. Punctuation that
/// full-text syntax would read as an operator is searched for as text, so a
/// mistyped quote returns nothing rather than an error about syntax.
#[test]
fn punctuation_is_searched_for_rather_than_parsed_as_syntax() {
    let store = seeded();

    let stray = store
        .search("\"unterminated", 10)
        .expect("a stray quote is text, not a syntax error");
    assert!(stray.is_empty(), "and it matches nothing: {stray:#?}");

    // The characters that would otherwise be operators.
    for query in ["CR-2026-491", "(555) 481-2290", "State v. Rivera (theft)"] {
        store
            .search(query, 10)
            .unwrap_or_else(|error| panic!("{query:?} must be searchable: {error}"));
    }

    // The two things a person might reasonably expect to work still do.
    let prefix = store.search("Riv*", 25).expect("a prefix search");
    assert!(
        prefix.iter().any(|hit| hit.title.contains("Rivera")),
        "a trailing star is still a prefix: {prefix:#?}"
    );
    let either = store.search("Okonkwo OR Rivera", 50).expect("an OR search");
    assert!(
        either.iter().any(|hit| hit.title.contains("Okonkwo"))
            && either.iter().any(|hit| hit.title.contains("Rivera")),
        "and bare uppercase OR is still an operator"
    );
}

/// "The Somali-speaking client from Tuesday" is a real question an office
/// asks, so the language a person asks to be spoken to in is indexed.
#[test]
fn a_client_is_found_by_the_language_they_speak() {
    let store = seeded();
    let hits = store.search("Spanish", 25).expect("search by language");
    assert!(
        hits.iter()
            .any(|hit| hit.kind == "client" && hit.title == "Alex Rivera"),
        "the fixture records Rivera's preferred language: {hits:#?}"
    );
}
