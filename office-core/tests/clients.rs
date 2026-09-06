//! Client demographics: recorded when asked, absent when not, never invented.

use office_core::{OfficeFixture, OfficeStore, ProposedClient, Sex};

const ANCHOR: &str = "2026-08-31";

fn seeded() -> OfficeStore {
    let mut store = OfficeStore::in_memory().expect("in-memory store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut store, ANCHOR)
        .expect("seed the misdemeanor docket");
    store
}

fn proposed(name: &str) -> ProposedClient {
    ProposedClient {
        id: None,
        display_name: name.to_owned(),
        date_of_birth: None,
        sex: None,
        preferred_language: None,
        notes: None,
        aliases: Vec::new(),
        contacts: Vec::new(),
        author_user_id: String::new(),
    }
}

/// The intake sheet's two demographic lines survive the round trip.
#[test]
fn a_client_carries_the_sex_and_language_the_office_recorded() {
    let mut store = seeded();
    let author = store
        .user_named("A. Defender", "attorney")
        .expect("the fixture attorney");

    let profile = store
        .create_client(&ProposedClient {
            sex: Some(Sex::Female),
            preferred_language: Some("Somali".to_owned()),
            author_user_id: author,
            ..proposed("Hodan Warsame")
        })
        .expect("open the client");

    assert_eq!(profile.sex.as_deref(), Some("female"));
    assert_eq!(profile.preferred_language.as_deref(), Some("Somali"));
}

/// Absence is a value: nobody asked is not the same as an answer.
#[test]
fn a_client_with_nothing_recorded_reads_as_not_recorded_rather_than_unknown() {
    let mut store = seeded();
    let author = store
        .user_named("A. Defender", "attorney")
        .expect("the fixture attorney");

    let profile = store
        .create_client(&ProposedClient {
            author_user_id: author,
            ..proposed("Quiet Intake")
        })
        .expect("open the client");

    assert_eq!(profile.sex, None);
    assert_eq!(profile.preferred_language, None);
}

/// `Sex::from_db` mirrors the schema trigger's vocabulary exactly. The raw-SQL
/// refusal itself is asserted in the store's `schema` module, which is the one
/// place with connection access.
#[test]
fn the_enum_and_the_schema_agree_on_the_sex_vocabulary() {
    for value in ["female", "male", "another"] {
        assert!(Sex::from_db(value).is_some(), "{value} is in both");
    }
    assert!(Sex::from_db("unspecified").is_none());
    assert_eq!(Sex::ALL.len(), 3);
}

/// A language is a name or nothing: blank is refused as a stored value.
#[test]
fn a_language_is_a_name_or_nothing() {
    let mut store = seeded();
    let author = store
        .user_named("A. Defender", "attorney")
        .expect("the fixture attorney");

    let profile = store
        .create_client(&ProposedClient {
            preferred_language: Some("   ".to_owned()),
            author_user_id: author.clone(),
            ..proposed("Blank Answer")
        })
        .expect("a blank answer is stored as no answer");
    assert_eq!(profile.preferred_language, None);

    let error = store
        .create_client(&ProposedClient {
            preferred_language: Some("x".repeat(80)),
            author_user_id: author,
            ..proposed("Long Answer")
        })
        .expect_err("a paragraph is not a language name");
    assert!(error.to_string().contains("language"), "{error}");
}
