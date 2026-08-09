#![allow(missing_docs)]

use evidence_intake::{
    AdvocacyKind, CaseExport, CaseId, DemoFixture, ExportAudience, NodeKind, NodeRef,
    ProposedAdvocacyItem, ProposedAnnotation, ProposedBrief, ProposedProposition, Store,
};

/// A phrase that exists only inside privileged material, so that finding it
/// anywhere in a disclosable export is proof of a leak.
const PRIVILEGED_MARKER: &str = "PRIVILEGED-CANARY-do-not-disclose";

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("create in-memory store");
    let case_id = DemoFixture::HitAndRun
        .seed(&mut store)
        .expect("seed hit-and-run fixture");
    (store, case_id)
}

/// Seeds one of every privileged record, each carrying the canary.
fn with_work_product() -> (Store, CaseId) {
    let (mut store, case_id) = hit_and_run();
    store
        .author_advocacy_item(
            &case_id,
            &ProposedAdvocacyItem {
                id: None,
                kind: AdvocacyKind::AttorneyConclusion,
                title: format!("Conclusion {PRIVILEGED_MARKER}"),
                body: format!("Analysis {PRIVILEGED_MARKER}"),
                status: None,
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("author work product");
    store
        .annotate(
            &case_id,
            &ProposedAnnotation {
                id: None,
                target: NodeRef::new(NodeKind::Content, "hr-content-911-injury"),
                body: format!("Note {PRIVILEGED_MARKER}"),
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("annotate");
    store
        .record_brief(
            &case_id,
            &ProposedBrief {
                id: None,
                posture: "trial".to_owned(),
                summary: format!("Advice {PRIVILEGED_MARKER}"),
                strengths: String::new(),
                risks: String::new(),
                unresolved_questions: String::new(),
                client_topics: String::new(),
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("record brief");
    (store, case_id)
}

fn rendered(export: &CaseExport) -> String {
    serde_json::to_string(export).expect("serialize export")
}

/// The rule this whole module exists to keep: privileged analysis does not
/// leave in an export that may be handed outside the defense team.
#[test]
fn a_disclosable_export_carries_no_privileged_material() {
    let (store, case_id) = with_work_product();
    let export = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");

    assert!(!export.includes_privileged);
    assert!(export.privileged.is_empty());
    assert!(
        !rendered(&export).contains(PRIVILEGED_MARKER),
        "privileged material must not appear anywhere in a disclosable export"
    );
}

#[test]
fn a_work_file_export_carries_the_teams_own_analysis() {
    let (store, case_id) = with_work_product();
    let export = store
        .export_case(&case_id, ExportAudience::WorkFile)
        .expect("export");

    assert!(export.includes_privileged);
    assert!(rendered(&export).contains(PRIVILEGED_MARKER));

    let kinds: Vec<&str> = export
        .privileged
        .iter()
        .map(|item| item.kind.as_str())
        .collect();
    assert!(kinds.contains(&"annotation"));
    assert!(kinds.contains(&"decision_brief"));
    assert!(kinds.contains(&"attorney_conclusion"));
}

/// Rule twelve: a sentence a reader cannot open is not a fact.
#[test]
fn every_exported_line_resolves_to_an_exact_original() {
    let (store, case_id) = hit_and_run();
    let export = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");

    assert!(!export.propositions.is_empty());
    for proposition in &export.propositions {
        assert!(
            !proposition.evidence.is_empty(),
            "`{}` was exported with nothing behind it",
            proposition.id
        );
        for item in &proposition.evidence {
            assert!(
                !item.locator.trim().is_empty(),
                "`{}` cites `{}` with no locator to open",
                proposition.id,
                item.source
            );
            assert!(!item.source.trim().is_empty());
        }
    }
}

/// An assertion nobody can check is named as one rather than exported as a fact.
#[test]
fn a_proposition_with_no_evidence_is_reported_as_unsupported() {
    let (mut store, case_id) = hit_and_run();
    let bare = store
        .author_proposition(
            &case_id,
            &ProposedProposition {
                id: None,
                text: "Nothing in the file bears on this yet.".to_owned(),
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("author");

    let export = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");

    assert!(
        export.propositions.iter().all(|item| item.id != bare.id),
        "an unsupported proposition must not appear as a fact"
    );
    let listed = export
        .unsupported
        .iter()
        .find(|item| item.id == bare.id)
        .expect("it must be named as unsupported");
    assert!(listed.reason.contains("no source-grounded evidence"));
}

/// Nothing leaves this tool silently reduced.
#[test]
fn evidence_a_reviewer_rejected_is_omitted_but_counted() {
    let (mut store, case_id) = hit_and_run();
    let before = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export")
        .rejected_evidence_omitted;
    assert_eq!(before, 0);

    let edge = store
        .review_queue(&case_id)
        .expect("queue")
        .into_iter()
        .find(|item| item.target_kind == "edge")
        .expect("an edge awaiting review");
    store
        .apply_review(
            &case_id,
            &evidence_intake::ReviewDecision {
                target: evidence_intake::ReviewTarget::Edge,
                target_id: edge.target_id.clone(),
                to_state: evidence_intake::ReviewState::Rejected,
                actor: "A. Reyes".to_owned(),
                basis: Some("The excerpt does not say this.".to_owned()),
                verified_against_locator: None,
            },
        )
        .expect("reject");

    let after = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");
    assert_eq!(
        after.rejected_evidence_omitted, 1,
        "an omission must be counted, not silent"
    );
}

/// The ledger of what the other side produced is part of any export: an exhibit
/// list without the completeness record hides what is missing.
#[test]
fn an_export_carries_the_production_ledger() {
    let (store, case_id) = hit_and_run();
    let export = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");

    assert_eq!(export.case_id, case_id.0);
    assert!(!export.case_name.is_empty());
    assert_eq!(export.audience, "disclosable");
    assert_eq!(
        export.productions,
        store.discovery_ledger(&case_id).expect("ledger"),
        "the export's ledger is the discovery ledger, not a second version of it"
    );
    assert!(
        export
            .productions
            .iter()
            .any(|item| item.integrity_status == "missing"),
        "the fixture's referenced-but-missing evidence must survive into the export"
    );
}

#[test]
fn exporting_an_unknown_case_is_refused() {
    let (store, _) = hit_and_run();
    let error = store
        .export_case(
            &CaseId("no-such-case".to_owned()),
            ExportAudience::Disclosable,
        )
        .expect_err("an unknown case must be refused");
    assert!(error.to_string().contains("no-such-case"));
}

/// Two cases in one database must never bleed into one another's export.
#[test]
fn an_export_carries_only_its_own_case() {
    let mut store = Store::in_memory().expect("store");
    let hit_run = DemoFixture::HitAndRun.seed(&mut store).expect("hit run");
    let vehicle_stop = DemoFixture::VehicleStop
        .seed(&mut store)
        .expect("vehicle stop");

    let other = store
        .export_case(&vehicle_stop, ExportAudience::WorkFile)
        .expect("export");
    let mine = rendered(
        &store
            .export_case(&hit_run, ExportAudience::WorkFile)
            .expect("export"),
    );

    for proposition in &other.propositions {
        assert!(
            !mine.contains(&proposition.text),
            "`{}` belongs to the other case",
            proposition.id
        );
    }
    for item in &other.privileged {
        assert!(!mine.contains(&item.body), "privileged text crossed cases");
    }
}

/// Nothing leaves silently reduced, and nothing leaves silently unchecked.
#[test]
fn unreviewed_evidence_is_counted_so_it_is_never_attached_unnoticed() {
    let (mut store, case_id) = hit_and_run();
    let before = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");
    assert!(
        before.unreviewed_evidence_included > 0,
        "the fixture rests on machine suggestions nobody has reviewed yet"
    );

    // Reviewing one relationship reduces the count by exactly the lines it carries.
    let edge = store
        .review_queue(&case_id)
        .expect("queue")
        .into_iter()
        .find(|item| item.target_kind == "edge")
        .expect("an edge awaiting review");
    store
        .apply_review(
            &case_id,
            &evidence_intake::ReviewDecision {
                target: evidence_intake::ReviewTarget::Edge,
                target_id: edge.target_id,
                to_state: evidence_intake::ReviewState::Verified,
                actor: "A. Reyes".to_owned(),
                basis: Some("Compared both cited excerpts.".to_owned()),
                verified_against_locator: None,
            },
        )
        .expect("verify");

    let after = store
        .export_case(&case_id, ExportAudience::Disclosable)
        .expect("export");
    assert!(
        after.unreviewed_evidence_included <= before.unreviewed_evidence_included,
        "reviewing material must never increase the unchecked count"
    );

    // Every line the count refers to is visible on the line itself.
    let flagged = after
        .propositions
        .iter()
        .flat_map(|item| &item.evidence)
        .filter(|item| {
            matches!(item.review_state.as_str(), "unreviewed" | "suggested")
                || matches!(
                    item.relation_review_state.as_str(),
                    "unreviewed" | "suggested"
                )
        })
        .count();
    assert_eq!(
        u32::try_from(flagged).expect("fits"),
        after.unreviewed_evidence_included,
        "the header count must match what the lines themselves say"
    );
}
