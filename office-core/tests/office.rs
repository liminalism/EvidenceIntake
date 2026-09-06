//! Clients and matters: what a person carries across their cases, and what
//! closing one does and does not do.

use office_core::{MatterStatus, OfficeFixture, OfficeStore};

/// The fixture anchors on a fixed Monday so a docket assertion cannot change
/// meaning overnight.
const ANCHOR: &str = "2026-08-31";

/// The Tuesday the fixture puts the consolidated Rivera setting on.
const RIVERA_SETTING_DAY: &str = "2026-09-01";

fn seeded() -> OfficeStore {
    let mut store = OfficeStore::in_memory().expect("in-memory store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut store, ANCHOR)
        .expect("seed the misdemeanor docket");
    store
}

/// A client is the person, not the case. Aliases and contacts belong to them
/// and are reachable from any of their matters — the difference between an
/// office client record and the kernel's per-case entity mentions, which are
/// deliberately conservative and never merged across cases.
#[test]
fn a_client_carries_aliases_and_contacts_across_matters() {
    let store = seeded();
    let profile = store.client_profile("client-rivera").expect("the client");

    assert_eq!(profile.display_name, "Alex Rivera");
    assert!(
        profile
            .aliases
            .iter()
            .any(|alias| alias == "Alejandro Rivera"),
        "an alias the office recorded is part of the person, not of one case"
    );
    assert!(
        profile
            .contacts
            .iter()
            .any(|contact| contact.kind == "phone" && contact.value.contains("481-2290")),
        "a contact is written down once and reachable from every matter"
    );
    assert!(
        profile.matters.len() >= 3,
        "the fixture gives this client three related matters, got {}",
        profile.matters.len()
    );

    // The same aliases and contacts are what a search across the office finds,
    // which is the point of holding them on the person.
    let hits = store.search("Alejandro", 10).expect("search the office");
    assert!(
        hits.iter().any(|hit| hit.subject_id == "client-rivera"),
        "the alias reaches the person it belongs to"
    );
}

/// Closing a matter is an office bookkeeping change, not a calendar one.
///
/// The setting stays on the docket, carrying the matter's new status, because
/// somebody still has to be in that courtroom — to appear, or to have the
/// setting struck. A docket that quietly dropped the row the moment a matter
/// was closed would be a missed appearance waiting to happen. A setting leaves
/// the calendar exactly one way: by being cancelled.
#[test]
fn a_closed_matter_stays_on_the_docket_until_its_setting_is_cancelled() {
    let store = seeded();
    store
        .update_matter_status("matter-rivera-3", MatterStatus::Closed)
        .expect("close one of the three");

    let day = store
        .docket_day(RIVERA_SETTING_DAY)
        .expect("the day the setting falls on");
    let setting = day
        .settings
        .iter()
        .find(|entry| entry.client_id == "client-rivera")
        .expect("the consolidated setting");

    let closed = setting
        .matters
        .iter()
        .find(|line| line.id == "matter-rivera-3")
        .expect("the closed matter is still on the row");
    assert_eq!(closed.status, "closed");
    assert_eq!(
        setting.matters.len(),
        3,
        "closing a matter does not silently shrink a setting"
    );

    store
        .cancel_appearance("appearance-rivera-consolidated", RIVERA_SETTING_DAY)
        .expect("strike the setting");
    let after = store
        .docket_day(RIVERA_SETTING_DAY)
        .expect("the same day again");
    assert!(
        !after
            .settings
            .iter()
            .any(|entry| entry.client_id == "client-rivera"),
        "a cancelled setting leaves the docket"
    );
}

/// The matter chooser puts what the office is still carrying first, so a
/// closed case never sits above an open one in a list somebody works from.
#[test]
fn closing_a_matter_moves_it_behind_the_open_ones() {
    let store = seeded();
    store
        .update_matter_status("matter-rivera-1", MatterStatus::Closed)
        .expect("close a matter");

    let matters = store.matters().expect("list matters");
    let closed_at = matters
        .iter()
        .position(|summary| summary.id == "matter-rivera-1")
        .expect("the closed matter is still listed");
    let last_open = matters
        .iter()
        .rposition(|summary| summary.status == "open")
        .expect("the office is still carrying something");

    assert!(
        closed_at > last_open,
        "a closed matter sorts behind every open one, at {closed_at} against {last_open}"
    );
    assert_eq!(matters[closed_at].status, "closed");
}
