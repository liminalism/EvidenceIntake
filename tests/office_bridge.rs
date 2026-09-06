//! Where the office layer meets the kernel, and what may not cross.

use evidence_intake::{
    AdvocacyKind, CaseId, DemoFixture, NodeKind, NodeRef, OfficeDesk, ProposedAdvocacyItem,
    ProposedAnnotation, ProposedBrief, Store,
};
use office_core::{OfficeFixture, OfficeStore, ProposedMatter};

/// The office fixture anchors on a fixed Monday, and links one matter to the
/// kernel's hit-and-run case.
const ANCHOR: &str = "2026-08-31";

/// An evidence case carrying privileged work product, and an office holding a
/// matter that points at it.
fn desk_and_kernel() -> (OfficeDesk, Store) {
    let mut evidence = Store::in_memory().expect("evidence store");
    let case = DemoFixture::HitAndRun
        .seed(&mut evidence)
        .expect("seed case");
    assert_eq!(case.0, "case-hit-run-001");

    // Privileged material on the linked case: an issue, a note, and a brief.
    // If any of it can be reached from a docket row, the boundary has failed.
    evidence
        .author_advocacy_item(
            &case,
            &ProposedAdvocacyItem {
                id: Some("adv-suppression-theory".to_owned()),
                kind: AdvocacyKind::LegalIssue,
                title: "Stop was pretextual; move to suppress".to_owned(),
                body: "PRIVILEGED THEORY: the officer's stated basis collapses on the timeline."
                    .to_owned(),
                status: Some("open".to_owned()),
                author: "A. Defender".to_owned(),
            },
        )
        .expect("author a privileged issue");
    evidence
        .annotate(
            &case,
            &ProposedAnnotation {
                id: Some("ann-privileged".to_owned()),
                target: NodeRef::new(NodeKind::Source, "hr-src-crash-report".to_owned()),
                body: "PRIVILEGED NOTE: do not concede the clock discrepancy.".to_owned(),
                author: "A. Defender".to_owned(),
            },
        )
        .expect("attach a privileged note");
    evidence
        .record_brief(
            &case,
            &ProposedBrief {
                id: Some("brief-privileged".to_owned()),
                posture: "motions".to_owned(),
                summary: "PRIVILEGED BRIEF: recommend litigating the stop.".to_owned(),
                strengths: "The timeline does not close.".to_owned(),
                risks: "PRIVILEGED RISK: the client's own account moved.".to_owned(),
                unresolved_questions: "PRIVILEGED QUESTION: does the client testify?".to_owned(),
                client_topics: "PRIVILEGED TOPIC: the plea offer and its exposure.".to_owned(),
                author: "A. Defender".to_owned(),
            },
        )
        .expect("write a privileged brief");

    let mut desk = OfficeDesk::in_memory().expect("office desk");
    OfficeFixture::MisdemeanorDocket
        .seed_from(desk.store_mut(), ANCHOR)
        .expect("seed the office");
    (desk, evidence)
}

/// The acceptance check, stated as a test. Privileged kernel material must not
/// be reachable through the office database, any office read model, or any
/// office export.
///
/// It passes structurally: `office_core` has no dependency on the kernel, so a
/// docket row is built by code that cannot open the evidence database. What
/// this asserts is that the *bridge* — the one place both are open — does not
/// carry any of it across.
#[test]
fn the_office_layer_cannot_reach_privileged_kernel_material() {
    let (desk, evidence) = desk_and_kernel();

    let docket = desk
        .court_docket(&evidence, "2026-09-01")
        .expect("tuesday docket");
    let matter = desk
        .matter_view(&evidence, "matter-rivera-1")
        .expect("the matter linked to the case");

    for (what, rendered) in [
        (
            "the docket",
            serde_json::to_string(&docket).expect("docket JSON"),
        ),
        (
            "the matter view",
            serde_json::to_string(&matter).expect("matter JSON"),
        ),
    ] {
        for forbidden in [
            "PRIVILEGED",
            "adv-suppression-theory",
            "ann-privileged",
            "pretextual",
            "do not concede",
            "recommend litigating",
            "advocacy",
            "annotation",
            "brief",
        ] {
            assert!(
                !rendered.to_lowercase().contains(&forbidden.to_lowercase()),
                "{what} carried privileged material: {forbidden:?} appears in it"
            );
        }
    }

    // And the office database itself holds no such table to read from.
    let office = desk.store();
    for table in [
        "advocacy_items",
        "annotations",
        "decision_briefs",
        "content",
        "sources",
    ] {
        assert!(
            office.search(table, 5).expect("search").is_empty()
                || !office
                    .search(table, 5)
                    .expect("search")
                    .iter()
                    .any(|hit| hit.kind == table),
            "the office index has no {table} in it"
        );
    }
}

/// Rule 35 reaches the docket. The posture summary reports what a case rests on
/// and never says how it is going.
#[test]
fn nothing_in_the_docket_posture_scores_the_case() {
    let (desk, evidence) = desk_and_kernel();
    let docket = desk
        .court_docket(&evidence, "2026-09-01")
        .expect("tuesday docket");

    let rendered = serde_json::to_string(&docket)
        .expect("docket JSON")
        .to_lowercase();
    for verdict in [
        "score",
        "strength",
        "strong",
        "weak",
        "likely",
        "probability",
        "confidence",
        "recommend",
        "should ",
        "chance",
        "odds",
        "rating",
        "rank",
        "winnable",
        "favorable",
        "risk_level",
        "severity",
    ] {
        assert!(
            !rendered.contains(verdict),
            "a docket reports structure and never a verdict, but {verdict:?} appears in it"
        );
    }

    // The grid line a person actually reads is counts and absences only.
    for row in &docket.rows {
        let line = row.posture_line();
        assert!(
            !line.is_empty(),
            "every row says something about what it rests on"
        );
        for verdict in ["strong", "weak", "likely", "should", "recommend"] {
            assert!(!line.to_lowercase().contains(verdict), "{line:?}");
        }
    }
}

/// A matter with no evidence case says so. Reporting zeroes would read as a
/// case whose evidence is in good order, which is a different claim entirely.
#[test]
fn a_matter_without_an_evidence_case_reports_no_posture_rather_than_zeroes() {
    let (desk, evidence) = desk_and_kernel();

    let linked = desk
        .matter_view(&evidence, "matter-rivera-1")
        .expect("the linked matter");
    assert!(linked.posture.is_some(), "a linked matter has a posture");
    assert_eq!(
        linked.posture.as_ref().expect("posture").evidence_case_id,
        "case-hit-run-001"
    );

    let unlinked = desk
        .matter_view(&evidence, "matter-rivera-2")
        .expect("the unlinked matter");
    assert_eq!(unlinked.matter.evidence_case_id, None);
    assert_eq!(
        unlinked.posture, None,
        "no case linked is reported as none, not as a posture of zeroes"
    );

    let docket = desk
        .court_docket(&evidence, "2026-09-01")
        .expect("tuesday docket");
    let rivera = docket
        .rows
        .iter()
        .find(|row| row.entry.client == "Alex Rivera")
        .expect("the consolidated setting");
    assert_eq!(
        rivera.matters_without_evidence, 2,
        "two of the three matters on this setting carry no discovery"
    );
    assert_eq!(
        rivera.postures.len(),
        1,
        "and one posture, for the one case behind them"
    );
    assert!(
        rivera.posture_line().contains("without a case"),
        "which the row says out loud: {}",
        rivera.posture_line()
    );
}

/// A matter pointing at a case the kernel does not hold is a broken link, not
/// an empty one.
#[test]
fn a_matter_pointing_at_an_unknown_case_reports_no_posture() {
    let (mut desk, evidence) = desk_and_kernel();
    let author = desk
        .store_mut()
        .user_named("A. Defender", "attorney")
        .expect("author");
    let client = desk.store().clients().expect("clients")[0].0.clone();
    let matter = desk
        .store()
        .open_matter(&ProposedMatter {
            id: Some("matter-dangling".to_owned()),
            client_id: client,
            caption: "State v. Dangling".to_owned(),
            court_number: None,
            court_id: None,
            status: None,
            custody_state: None,
            offer_state: None,
            offer_summary: None,
            charge_summary: None,
            opened_on: None,
            last_contact_on: None,
            evidence_case_id: Some("case-that-was-never-opened".to_owned()),
            author_user_id: author,
        })
        .expect("a matter may point anywhere; nothing validates it here");

    let view = desk
        .matter_view(&evidence, &matter)
        .expect("the dangling matter still reads");
    assert_eq!(
        view.matter.evidence_case_id.as_deref(),
        Some("case-that-was-never-opened"),
        "the office keeps what it was told"
    );
    assert_eq!(
        view.posture, None,
        "and the kernel simply has nothing to say about it"
    );
}

/// A broken link is a third state, and a docket row has to say so.
///
/// A matter that was never linked and a matter naming a case the evidence
/// database does not hold look identical if a row only counts postures: both
/// come back empty. They are not the same problem — the first is ordinary, the
/// second is a link somebody has to go and repair — and there is no foreign key
/// between the two databases to catch it, which is exactly why the row reports
/// it rather than assuming it away.
#[test]
fn a_setting_naming_a_case_the_kernel_does_not_hold_says_so() {
    let (desk, evidence) = desk_and_kernel();
    desk.store()
        .link_evidence_case("matter-okonkwo-1", "case-that-was-never-opened")
        .expect("point the matter at nothing");

    let docket = desk
        .court_docket(&evidence, "2026-09-01")
        .expect("tuesday docket");
    let okonkwo = docket
        .rows
        .iter()
        .find(|row| row.entry.client_id == "client-okonkwo")
        .expect("the setting is on the day");

    assert!(okonkwo.postures.is_empty());
    assert_eq!(
        okonkwo.matters_with_a_missing_case, 1,
        "the broken link is counted"
    );
    assert_eq!(
        okonkwo.matters_without_evidence, 0,
        "and is not confused with a matter that never had a case"
    );
    let line = okonkwo.posture_line();
    assert_eq!(line, "1 case linked but not found");
    assert_ne!(
        line, "no matters",
        "a setting with a matter on it never reports itself as empty"
    );

    // The row that genuinely has no linked case still reads the other way.
    let unlinked = docket
        .rows
        .iter()
        .find(|row| row.matters_without_evidence > 0 && row.postures.is_empty())
        .expect("the caseload has matters with no evidence case at all");
    assert_eq!(unlinked.matters_with_a_missing_case, 0);
    assert_eq!(unlinked.posture_line(), "no evidence case linked");
}

/// The posture is derived from the standing view, so it moves with the case.
#[test]
fn the_posture_counts_what_the_standing_view_reports() {
    let (desk, evidence) = desk_and_kernel();
    let standing = evidence
        .case_standing(&CaseId("case-hit-run-001".to_owned()))
        .expect("standing");
    let posture = desk
        .posture(&evidence, "case-hit-run-001")
        .expect("posture")
        .expect("the case exists");

    assert_eq!(posture.case_name, standing.case_name);
    assert_eq!(
        u64::from(posture.elements_in_all),
        standing
            .charges
            .iter()
            .map(|charge| charge.elements.len() as u64)
            .sum::<u64>()
    );
    assert_eq!(
        u64::from(posture.load_bearing_sources),
        standing.load_bearing_sources.len() as u64
    );
    assert_eq!(
        u64::from(posture.live_disputes),
        standing.live_disputes.len() as u64
    );
    assert_eq!(
        u64::from(posture.open_gaps),
        standing.open_gaps.len() as u64
    );
    assert!(
        posture.sole_source_elements <= posture.elements_in_all,
        "a count of elements cannot exceed the elements"
    );
}

/// The office database is a separate file beside the evidence one.
#[test]
fn the_office_database_sits_beside_the_evidence_database() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let evidence = directory.path().join("evidence.sqlite");
    let office = OfficeDesk::beside(&evidence);
    assert_eq!(office, directory.path().join("office.sqlite"));

    // Two files, two connections, no shared transaction and no cross-database
    // foreign key. That is the boundary, made of the filesystem.
    let _kernel = Store::open(&evidence).expect("evidence database");
    let _desk = OfficeDesk::open(&office).expect("office database");
    assert!(evidence.exists() && office.exists());
    assert_ne!(evidence, office);
}

/// Two matters of one client sharing an evidence case are one posture, not two.
#[test]
fn one_setting_consults_the_kernel_once_per_case() {
    let (desk, evidence) = desk_and_kernel();
    desk.store()
        .link_evidence_case("matter-rivera-2", "case-hit-run-001")
        .expect("point the second matter at the same case");

    let docket = desk
        .court_docket(&evidence, "2026-09-01")
        .expect("tuesday docket");
    let rivera = docket
        .rows
        .iter()
        .find(|row| row.entry.client == "Alex Rivera")
        .expect("the consolidated setting");
    assert_eq!(
        rivera.postures.len(),
        1,
        "one case behind two matters is one posture"
    );
    assert_eq!(rivera.matters_without_evidence, 1);
}

/// A store built through the office alone still cannot see the kernel.
#[test]
fn the_office_store_has_no_way_to_open_an_evidence_database() {
    let mut office = OfficeStore::in_memory().expect("office store");
    OfficeFixture::MisdemeanorDocket
        .seed_from(&mut office, ANCHOR)
        .expect("seed");

    // The matter knows an identifier and nothing else. There is no method on
    // this type that resolves it, because the crate has no kernel to resolve
    // it against — that is the boundary, and it is a compile-time fact rather
    // than a runtime check.
    let profile = office.matter_profile("matter-rivera-1").expect("matter");
    assert_eq!(
        profile.evidence_case_id.as_deref(),
        Some("case-hit-run-001"),
        "an opaque string is all the office holds"
    );
}
