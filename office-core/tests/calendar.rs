//! One setting spans a client's matters and renders once.

use office_core::{
    AppearanceType, DeadlineOrigin, OfficeFixture, OfficeStore, ProposedAppearance, ProposedClient,
    ProposedDeadline, ProposedMatter,
};

/// The fixture anchors on a fixed Monday so a docket assertion cannot change
/// meaning overnight.
const ANCHOR: &str = "2026-08-31";

fn seeded() -> OfficeStore {
    let mut store = OfficeStore::in_memory().expect("in-memory store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut store, ANCHOR)
        .expect("seed the misdemeanor docket");
    store
}

/// The rule the whole calendar model exists for. A person called at nine on
/// three related cases has one place to be, and a docket that renders three
/// rows for it is the duplicate-setting error the office layer is meant to
/// make impossible.
#[test]
fn an_appearance_renders_one_row_across_every_linked_matter() {
    let store = seeded();
    let day = store.docket_day("2026-09-01").expect("tuesday docket");

    let rivera: Vec<_> = day
        .settings
        .iter()
        .filter(|entry| entry.client == "Alex Rivera")
        .collect();
    assert_eq!(
        rivera.len(),
        1,
        "three matters called together are one setting, not three: {:#?}",
        rivera.iter().map(|entry| &entry.id).collect::<Vec<_>>()
    );

    let entry = rivera[0];
    assert_eq!(
        entry.matters.len(),
        3,
        "the one row carries all three matters"
    );
    let numbers: Vec<&str> = entry
        .matters
        .iter()
        .filter_map(|line| line.court_number.as_deref())
        .collect();
    assert_eq!(
        numbers,
        ["CR-2026-491", "CR-2026-492", "CR-2026-517"],
        "the row reads in the order a courtroom calls the numbers"
    );
    assert_eq!(entry.time.as_deref(), Some("09:00"));
    assert_eq!(entry.appearance_type, "pretrial");
}

/// A matter inside a shared setting can be handled differently without the
/// setting splitting into two rows.
#[test]
fn a_matter_is_overridden_inside_a_shared_setting_rather_than_split_out() {
    let store = seeded();
    let day = store.docket_day("2026-09-01").expect("tuesday docket");
    let entry = day
        .settings
        .iter()
        .find(|entry| entry.client == "Alex Rivera")
        .expect("the consolidated setting");

    let passed = entry
        .matters
        .iter()
        .find(|line| line.court_number.as_deref() == Some("CR-2026-517"))
        .expect("the failure-to-appear matter");
    assert_eq!(
        passed.override_note.as_deref(),
        Some("Passed for plea; the other two proceed.")
    );
    assert!(
        entry
            .matters
            .iter()
            .filter(|line| line.override_note.is_some())
            .count()
            == 1,
        "only the overridden matter carries a note"
    );
}

/// Removing one matter leaves the setting and the others where they were.
#[test]
fn a_matter_can_be_individually_unlinked_from_a_setting() {
    let store = seeded();
    store
        .unlink_matter_from_appearance("appearance-rivera-consolidated", "matter-rivera-3")
        .expect("unlink the third matter");

    let day = store.docket_day("2026-09-01").expect("tuesday docket");
    let entry = day
        .settings
        .iter()
        .find(|entry| entry.client == "Alex Rivera")
        .expect("the setting survives losing one matter");
    assert_eq!(entry.matters.len(), 2);
    assert!(
        !entry
            .matters
            .iter()
            .any(|line| line.court_number.as_deref() == Some("CR-2026-517")),
        "the unlinked matter is gone from the row"
    );

    // The matter itself is untouched; only its place on that setting was.
    let profile = store
        .matter_profile("matter-rivera-3")
        .expect("matter survives");
    assert_eq!(profile.caption, "State v. Rivera (FTA)");
}

/// A setting emptied of every matter is a calendar entry that means nothing.
#[test]
fn a_setting_cannot_be_emptied_of_every_matter() {
    let store = seeded();
    for matter in ["matter-rivera-1", "matter-rivera-2"] {
        store
            .unlink_matter_from_appearance("appearance-rivera-consolidated", matter)
            .expect("unlinking down to one is allowed");
    }
    let error = store
        .unlink_matter_from_appearance("appearance-rivera-consolidated", "matter-rivera-3")
        .expect_err("emptying the setting must be refused");
    assert!(error.to_string().contains("cannot be emptied"), "{error}");
}

/// Cancelling a setting is a different act from emptying one. It strikes the
/// setting from every docket read without erasing it, because notes written on
/// it are append-only and would otherwise be deleted with it.
#[test]
fn a_cancelled_setting_leaves_the_docket_without_being_erased() {
    let store = seeded();
    store
        .cancel_appearance("appearance-rivera-consolidated", "2026-08-31")
        .expect("a setting can be cancelled outright");
    let day = store.docket_day("2026-09-01").expect("tuesday docket");
    assert!(
        !day.settings
            .iter()
            .any(|entry| entry.client == "Alex Rivera"),
        "the cancelled setting is off the docket"
    );
    assert!(
        store.matter_profile("matter-rivera-1").is_ok(),
        "cancelling a setting does not delete the matters it covered"
    );
    assert!(
        store.note("note-rivera-setting").is_ok(),
        "and it does not take the note written on it either"
    );
    assert!(
        store
            .cancel_appearance("appearance-rivera-consolidated", "2026-08-31")
            .is_err(),
        "an already-struck setting is not cancelled twice"
    );
}

/// A setting belongs to one person. Another client's matter on it would put
/// somebody else's case on a docket row, which is only ever found in court.
#[test]
fn a_setting_refuses_a_matter_belonging_to_another_client() {
    let mut store = seeded();
    let error = store
        .link_matter_to_appearance("appearance-rivera-consolidated", "matter-okonkwo-1", None)
        .expect_err("another client's matter must be refused");
    assert!(
        error.to_string().contains("belongs to another client"),
        "{error}"
    );

    let error = store
        .schedule_appearance(&ProposedAppearance {
            id: None,
            client_id: "client-rivera".to_owned(),
            matter_ids: vec!["matter-rivera-1".to_owned(), "matter-okonkwo-1".to_owned()],
            court_id: None,
            judge_id: None,
            appearance_date: "2026-09-03".to_owned(),
            appearance_time: Some("10:00".to_owned()),
            appearance_type: AppearanceType::Status,
            notes: None,
            author_user_id: store.users().expect("users")[0].0.clone(),
        })
        .expect_err("a mixed-client setting must be refused at write time");
    assert!(
        error.to_string().contains("belongs to another client"),
        "{error}"
    );
}

/// A deadline says where it comes from, which decides whether the date can move,
/// and how far off it is from the day being read.
#[test]
fn deadlines_report_their_origin_and_days_remaining() {
    let store = seeded();
    let due = store
        .upcoming_deadlines("2026-08-31", 14)
        .expect("deadline window");

    let speedy = due
        .overdue
        .iter()
        .find(|row| row.description.contains("Speedy trial"))
        .expect("the expired statutory deadline is overdue, not merely upcoming");
    assert_eq!(speedy.origin, "statutory");
    assert_eq!(speedy.due_date, "2026-08-27");
    assert_eq!(
        speedy.days_remaining, -4,
        "a past deadline counts backwards"
    );
    assert_eq!(speedy.client, "Alex Rivera");

    let motions = due
        .upcoming
        .iter()
        .find(|row| row.description.contains("Suppression"))
        .expect("the court-ordered deadline is inside the window");
    assert_eq!(motions.origin, "court_ordered");
    assert_eq!(motions.days_remaining, 9);

    assert!(
        due.upcoming
            .windows(2)
            .all(|pair| pair[0].due_date <= pair[1].due_date),
        "the window is ordered by what falls first"
    );
}

/// A satisfied deadline leaves the window; nothing is deleted to make it go.
#[test]
fn a_satisfied_deadline_leaves_the_window() {
    let store = seeded();
    store
        .satisfy_deadline("deadline-rivera-motions", "2026-09-02")
        .expect("record the motion as filed");
    let due = store
        .upcoming_deadlines("2026-08-31", 14)
        .expect("deadline window");
    assert!(
        !due.upcoming
            .iter()
            .any(|row| row.id == "deadline-rivera-motions"),
        "a met deadline is no longer owed"
    );
    let profile = store.matter_profile("matter-rivera-2").expect("matter");
    let recorded = profile
        .deadlines
        .iter()
        .find(|row| row.id == "deadline-rivera-motions")
        .expect("the deadline is still on the matter's record");
    assert!(recorded.satisfied, "it is kept, marked met");
}

/// A day view names its weekday, and a range returns empty days rather than
/// skipping them: a defender has to know Thursday is clear.
#[test]
fn a_docket_range_returns_every_day_including_the_empty_ones() {
    let store = seeded();
    let week = store
        .docket_week("2026-09-02")
        .expect("the week around Wednesday");
    assert_eq!(week.len(), 7, "Monday through Sunday");
    assert_eq!(week[0].date, "2026-08-31");
    assert_eq!(week[0].weekday, "Monday");
    assert_eq!(week[6].date, "2026-09-06");
    assert_eq!(week[6].weekday, "Sunday");
    assert!(
        week.iter().any(|day| day.settings.is_empty()),
        "the weekend is empty and still reported"
    );
    assert!(
        week.iter().any(|day| !day.settings.is_empty()),
        "the working days are not"
    );
}

/// Deadlines falling on a day a settings list would otherwise hide.
#[test]
fn a_day_carries_the_deadlines_that_fall_on_it() {
    let store = seeded();
    let author = store.users().expect("users")[0].0.clone();
    store
        .record_deadline(&ProposedDeadline {
            id: Some("deadline-on-the-day".to_owned()),
            matter_id: "matter-rivera-1".to_owned(),
            description: "Client meeting before the setting".to_owned(),
            due_date: "2026-09-01".to_owned(),
            origin: DeadlineOrigin::SelfImposed,
            author_user_id: author,
        })
        .expect("record a same-day deadline");

    let day = store.docket_day("2026-09-01").expect("tuesday docket");
    assert!(
        day.deadlines_due
            .iter()
            .any(|row| row.id == "deadline-on-the-day"),
        "a deadline due today shows on today"
    );
}

/// A date the schema's shape check would accept but the calendar does not.
#[test]
fn a_date_that_is_not_a_real_day_is_refused_before_it_reaches_sql() {
    let mut store = seeded();
    let author = store.users().expect("users")[0].0.clone();
    let error = store
        .schedule_appearance(&ProposedAppearance {
            id: None,
            client_id: "client-rivera".to_owned(),
            matter_ids: vec!["matter-rivera-1".to_owned()],
            court_id: None,
            judge_id: None,
            appearance_date: "2026-02-30".to_owned(),
            appearance_time: None,
            appearance_type: AppearanceType::Status,
            notes: None,
            author_user_id: author.clone(),
        })
        .expect_err("February has no thirtieth");
    assert!(error.to_string().contains("calendar date"), "{error}");

    let error = store
        .schedule_appearance(&ProposedAppearance {
            id: None,
            client_id: "client-rivera".to_owned(),
            matter_ids: vec!["matter-rivera-1".to_owned()],
            court_id: None,
            judge_id: None,
            appearance_date: "2026-09-04".to_owned(),
            appearance_time: Some("25:00".to_owned()),
            appearance_type: AppearanceType::Status,
            notes: None,
            author_user_id: author,
        })
        .expect_err("there is no twenty-fifth hour");
    assert!(error.to_string().contains("time of day"), "{error}");
}

/// A setting must cover something.
#[test]
fn a_setting_without_a_matter_is_refused() {
    let mut store = seeded();
    let author = store.users().expect("users")[0].0.clone();
    let error = store
        .schedule_appearance(&ProposedAppearance {
            id: None,
            client_id: "client-rivera".to_owned(),
            matter_ids: Vec::new(),
            court_id: None,
            judge_id: None,
            appearance_date: "2026-09-04".to_owned(),
            appearance_time: None,
            appearance_type: AppearanceType::Status,
            notes: None,
            author_user_id: author,
        })
        .expect_err("an empty setting must be refused");
    assert!(error.to_string().contains("at least one matter"), "{error}");
}

/// The caseload the docket gate is measured against actually exists, and a
/// working day is legible without opening anything else.
#[test]
fn the_fixture_carries_a_caseload_at_the_scale_the_gate_names() {
    let store = seeded();
    let matters = store.matters().expect("matters");
    assert!(
        matters.len() >= 150,
        "the docket gate names roughly 150 active misdemeanors, not {}",
        matters.len()
    );
    assert!(
        matters.iter().filter(|m| m.status == "open").count() >= 150,
        "and they are open"
    );

    let day = store.docket_day("2026-09-01").expect("tuesday docket");
    assert!(!day.settings.is_empty(), "a working day has settings on it");
    for entry in &day.settings {
        assert!(!entry.client.is_empty(), "every row names the client");
        assert!(!entry.matters.is_empty(), "and what is being called");
        assert!(
            entry
                .matters
                .iter()
                .all(|line| !line.custody_state.is_empty()),
            "and where the client is"
        );
    }
}

/// A client with no evidence case is reported as having none, which is
/// different from being reported as having nothing wrong.
#[test]
fn a_matter_can_exist_without_an_evidence_case() {
    let mut store = OfficeStore::in_memory().expect("store");
    let author = store.user_named("A. Defender", "attorney").expect("author");
    let client = store
        .create_client(&ProposedClient {
            id: None,
            display_name: "Dana Whitcomb".to_owned(),
            date_of_birth: None,
            sex: None,
            preferred_language: None,
            notes: None,
            aliases: Vec::new(),
            contacts: Vec::new(),
            author_user_id: author.clone(),
        })
        .expect("client");
    let matter = store
        .open_matter(&ProposedMatter {
            id: None,
            client_id: client.id,
            caption: "State v. Whitcomb".to_owned(),
            court_number: Some("CR-2026-990".to_owned()),
            court_id: None,
            status: None,
            custody_state: None,
            offer_state: None,
            offer_summary: None,
            charge_summary: Some("Disorderly conduct".to_owned()),
            opened_on: None,
            last_contact_on: None,
            evidence_case_id: None,
            author_user_id: author,
        })
        .expect("matter without discovery");

    let profile = store.matter_profile(&matter).expect("profile");
    assert_eq!(profile.evidence_case_id, None);
    assert_eq!(
        profile.custody_state, "unknown",
        "an unrecorded custody state is unknown, not out"
    );
}
