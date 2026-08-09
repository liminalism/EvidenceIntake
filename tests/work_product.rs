#![allow(missing_docs)]

use evidence_intake::{
    AdvocacyKind, CaseId, DemoFixture, NodeKind, NodeRef, ProposedAdvocacyItem, ProposedAnnotation,
    ProposedBrief, Store,
};

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("seed hit-and-run fixture");
    (store, case_id)
}

fn item(kind: AdvocacyKind, title: &str, body: &str) -> ProposedAdvocacyItem {
    ProposedAdvocacyItem {
        id: None,
        kind,
        title: title.to_owned(),
        body: body.to_owned(),
        status: None,
        author: "A. Reyes".to_owned(),
    }
}

fn note(target: NodeRef, body: &str) -> ProposedAnnotation {
    ProposedAnnotation {
        id: None,
        target,
        body: body.to_owned(),
        author: "A. Reyes".to_owned(),
    }
}

fn brief(posture: &str, summary: &str) -> ProposedBrief {
    ProposedBrief {
        id: None,
        posture: posture.to_owned(),
        summary: summary.to_owned(),
        strengths: "Departure is recorded.".to_owned(),
        risks: "Morgan admits driving.".to_owned(),
        unresolved_questions: "Can the route be fixed?".to_owned(),
        client_topics: "Discuss the camera clip.".to_owned(),
        author: "A. Reyes".to_owned(),
    }
}

/// Work product is the attorney's own analysis, privileged by default.
#[test]
fn work_product_is_privileged_and_starts_at_version_one() {
    let (mut store, case_id) = hit_and_run();
    let written = store
        .author_advocacy_item(
            &case_id,
            &item(
                AdvocacyKind::MotionIssue,
                "Timing of the stop",
                "Whether the interval can be bridged at all.",
            ),
        )
        .expect("author work product");

    assert_eq!(written.version, 1);
    assert!(written.privileged);
    assert!(written.current);
    assert_eq!(written.supersedes, None);
    assert_eq!(written.status, "open");
    assert_eq!(written.author, "A. Reyes");
}

/// An earlier reading is not a mistake to be erased: it is what the attorney
/// thought when they made a decision.
#[test]
fn revising_work_product_supersedes_rather_than_overwrites() {
    let (mut store, case_id) = hit_and_run();
    let first = store
        .author_advocacy_item(
            &case_id,
            &item(AdvocacyKind::MotionIssue, "Timing", "First reading."),
        )
        .expect("author");

    let second = store
        .revise_advocacy_item(
            &case_id,
            &first.id,
            &item(AdvocacyKind::MotionIssue, "Timing", "Second reading."),
        )
        .expect("revise");

    assert_eq!(second.version, 2);
    assert_eq!(second.supersedes.as_deref(), Some(first.id.as_str()));
    assert!(second.current);

    let history = store
        .advocacy_history(&case_id, &first.id)
        .expect("history");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].body, "First reading.");
    assert!(
        !history[0].current,
        "the earlier version is no longer current"
    );
    assert_eq!(history[1].body, "Second reading.");

    // Any version identifier returns the whole history, not a suffix of it.
    assert_eq!(
        store
            .advocacy_history(&case_id, &second.id)
            .expect("history"),
        history
    );
}

/// A forked history would leave no single current reading of an issue.
#[test]
fn only_the_current_version_may_be_revised() {
    let (mut store, case_id) = hit_and_run();
    let first = store
        .author_advocacy_item(
            &case_id,
            &item(AdvocacyKind::LegalIssue, "Identity", "First reading."),
        )
        .expect("author");
    let second = store
        .revise_advocacy_item(
            &case_id,
            &first.id,
            &item(AdvocacyKind::LegalIssue, "Identity", "Second reading."),
        )
        .expect("revise");

    let error = store
        .revise_advocacy_item(
            &case_id,
            &first.id,
            &item(AdvocacyKind::LegalIssue, "Identity", "Competing reading."),
        )
        .expect_err("revising a superseded version must be refused");
    assert!(error.to_string().contains("already been superseded"));
    assert!(error.to_string().contains(&second.id));
}

/// A superseded reading is still readable, but it is not a second issue.
#[test]
fn a_superseded_issue_does_not_appear_twice_in_the_workspace() {
    let (mut store, case_id) = hit_and_run();
    let before = store.issue_workspaces(&case_id).expect("issues").len();

    let first = store
        .author_advocacy_item(
            &case_id,
            &item(AdvocacyKind::MotionIssue, "Timing", "First reading."),
        )
        .expect("author");
    store
        .revise_advocacy_item(
            &case_id,
            &first.id,
            &item(AdvocacyKind::MotionIssue, "Timing", "Second reading."),
        )
        .expect("revise");

    let issues = store.issue_workspaces(&case_id).expect("issues");
    assert_eq!(issues.len(), before + 1);
    let timing = issues
        .iter()
        .find(|issue| issue.title == "Timing")
        .expect("the issue appears once");
    assert_eq!(timing.body, "Second reading.");
}

#[test]
fn work_product_must_name_the_person_writing_it() {
    let (mut store, case_id) = hit_and_run();
    let mut anonymous = item(AdvocacyKind::LegalIssue, "Identity", "A reading.");
    anonymous.author = "   ".to_owned();

    let error = store
        .author_advocacy_item(&case_id, &anonymous)
        .expect_err("anonymous work product must be refused");
    assert!(error.to_string().contains("must name the person"));
}

/// A note that has been rewritten is not a second note.
#[test]
fn annotations_report_only_their_current_version() {
    let (mut store, case_id) = hit_and_run();
    let target = NodeRef::new(NodeKind::Content, "hr-content-911-injury");

    let first = store
        .annotate(
            &case_id,
            &note(target.clone(), "Check this against the report."),
        )
        .expect("annotate");
    store
        .annotate(&case_id, &note(target.clone(), "A second, separate note."))
        .expect("annotate again");

    assert_eq!(
        store.annotations(&case_id, &target).expect("notes").len(),
        2
    );

    let revised = store
        .revise_annotation(
            &case_id,
            &first.id,
            &note(target.clone(), "Checked; it differs."),
        )
        .expect("revise");

    let current = store.annotations(&case_id, &target).expect("notes");
    assert_eq!(current.len(), 2, "revising a note does not add one");
    assert!(current.iter().all(|note| note.current));
    assert!(
        current
            .iter()
            .any(|note| note.body == "Checked; it differs.")
    );
    assert!(
        current
            .iter()
            .all(|note| note.body != "Check this against the report."),
        "the superseded version is not reported as current"
    );
    assert_eq!(revised.version, 2);
}

#[test]
fn a_note_cannot_be_attached_to_a_record_in_another_case() {
    let mut store = Store::in_memory().expect("store");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");
    DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle stop");

    let error = store
        .annotate(
            &hit_run,
            &note(
                NodeRef::new(NodeKind::Proposition, "prop-consent"),
                "Belongs to the other case.",
            ),
        )
        .expect_err("a cross-case note must be refused");
    assert!(error.to_string().contains("prop-consent"));
}

/// A brief is advice as of a moment; replacing one in place would destroy the
/// record of what the client was told and when.
#[test]
fn each_brief_is_written_as_the_next_version_for_its_posture() {
    let (mut store, case_id) = hit_and_run();
    // The fixture already holds a negotiation brief; a new one continues that
    // posture's numbering rather than restarting it or replacing what was there.
    let existing = store
        .decision_brief(&case_id, "negotiation")
        .expect("the fixture's brief");
    assert_eq!(existing.version, 1);

    let next = store
        .record_brief(
            &case_id,
            &brief("negotiation", "The video changes the picture."),
        )
        .expect("next brief");
    assert_eq!(next.version, 2);

    let current = store
        .decision_brief(&case_id, "negotiation")
        .expect("current brief");
    assert_eq!(current.version, 2);
    assert_eq!(current.summary, "The video changes the picture.");
    assert_ne!(
        current.summary, existing.summary,
        "the earlier advice is superseded, not overwritten"
    );

    // A different posture keeps its own numbering.
    let trial = store
        .record_brief(&case_id, &brief("trial", "Departure is the live question."))
        .expect("trial brief");
    assert_eq!(trial.version, 1);
}

/// Privileged work product stays out of the discovery ledger, which is the one
/// view that describes what the other side produced.
#[test]
fn work_product_never_enters_the_discovery_ledger() {
    let (mut store, case_id) = hit_and_run();
    store
        .author_advocacy_item(
            &case_id,
            &item(
                AdvocacyKind::AttorneyConclusion,
                "Do not disclose",
                "A privileged conclusion.",
            ),
        )
        .expect("author");
    store
        .annotate(
            &case_id,
            &note(
                NodeRef::new(NodeKind::Source, "hr-src-crash-report"),
                "A privileged note about a produced source.",
            ),
        )
        .expect("annotate");

    let ledger = store.discovery_ledger(&case_id).expect("ledger");
    let rendered = format!("{ledger:?}");
    assert!(!rendered.contains("Do not disclose"));
    assert!(!rendered.contains("A privileged conclusion."));
    assert!(!rendered.contains("A privileged note"));
}
