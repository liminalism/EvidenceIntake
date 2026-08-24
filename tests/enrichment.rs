#![allow(missing_docs)]

use evidence_intake::{
    ContentForm, DemoFixture, EntityKind, InterpretationBatch, PerceptionBasis,
    ProposedSourceProfile, ReviewState, SourceRole, TemporalStance,
    gui::{Workspace, WorkspaceView},
};

fn workspace() -> Workspace {
    let mut workspace = Workspace::in_memory().expect("workspace");
    workspace.seed(DemoFixture::VehicleStop).expect("fixture");
    workspace
        .import_interpretations_json(
            &serde_json::to_string(&InterpretationBatch {
                case_id: evidence_intake::CaseId("case-vehicle-stop-001".to_owned()),
                source_profiles: vec![ProposedSourceProfile {
                    id: Some("profile-report-v1".to_owned()),
                    source_id: "src-report-v1".to_owned(),
                    source_role: SourceRole::PoliceReport,
                    author_entity_id: Some("person-chen".to_owned()),
                    created_at_claim: Some("2026-01-09T08:00:00".to_owned()),
                    default_content_form: Some(ContentForm::AuthoredAssertion),
                    default_temporal_stance: Some(TemporalStance::RetrospectiveRecollection),
                    default_perception_basis: Some(PerceptionBasis::ReadInSource),
                    clock_offset_ms: None,
                    clock_offset_basis: None,
                    review_state: ReviewState::Reviewed,
                    created_by: "P. Paralegal".to_owned(),
                    supersedes_profile_id: None,
                }],
                content_groups: Vec::new(),
                interpretations: Vec::new(),
            })
            .expect("batch JSON"),
        )
        .expect("profile import");
    workspace
        .open_enrichment("src-report-v1", "P. Paralegal")
        .expect("open enrichment");
    workspace
}

#[test]
fn source_profile_inheritance_meets_the_material_passage_budget_without_a_mouse_path() {
    let workspace = workspace();
    let session = workspace.enrichment_session().expect("session");
    assert!(session.rows.len() >= 2);
    assert!(session.rows.iter().all(|row| {
        row.effective.content_form.is_some()
            && row.effective.speaker_entity_id.is_some()
            && row.effective.temporal_stance.is_some()
    }));
    let rows = u32::try_from(session.rows.len()).expect("a source has a countable number of rows");
    let keys_per_passage = f64::from(session.keystrokes) / f64::from(rows);
    assert!(keys_per_passage <= 1.6, "{keys_per_passage}");
}

#[test]
fn attributed_person_and_reporting_parent_cost_no_more_than_four_keys() {
    let mut workspace = workspace();
    workspace.enrichment_key("j").expect("second passage");
    workspace.enrichment_key("F4").expect("attributed sweep");
    let before = workspace.enrichment_session().expect("session").keystrokes;
    workspace.enrichment_key("@").expect("autocomplete");
    workspace
        .enrichment_submit_entity("person-patel")
        .expect("select entity");
    workspace.enrichment_key("k").expect("return to statement");
    workspace.enrichment_key("p").expect("reporting parent");
    let session = workspace.enrichment_session().expect("session");
    assert!(session.keystrokes - before <= 4);
    assert!(
        session.rows[1]
            .current
            .as_ref()
            .and_then(|item| item.attributed_entity_id.as_deref())
            .is_some_and(|id| id == "person-patel")
    );
    assert!(
        session.rows[1]
            .current
            .as_ref()
            .and_then(|item| item.reporting_parent_interpretation_id.as_ref())
            .is_some()
    );
}

#[test]
fn value_repeat_range_and_command_mode_undo_write_new_versions() {
    let mut workspace = workspace();
    workspace.enrichment_key("q").expect("quoted");
    workspace.enrichment_key("k").expect("back");
    workspace.enrichment_key("a").expect("assertion");
    workspace.enrichment_key("k").expect("back");
    workspace.enrichment_key("Esc").expect("command mode");
    workspace.enrichment_key("u").expect("undo");
    let current = workspace.enrichment_session().expect("session").rows[0]
        .current
        .as_ref()
        .expect("current");
    assert_eq!(current.content_form, Some(ContentForm::QuotedStatement));
    assert!(
        current
            .basis
            .as_deref()
            .is_some_and(|basis| basis.starts_with("Undo"))
    );

    workspace.enrichment_key("j").expect("next");
    workspace.enrichment_key(".").expect("repeat");
    assert_eq!(
        workspace.enrichment_session().expect("session").rows[1]
            .current
            .as_ref()
            .and_then(|item| item.content_form),
        Some(ContentForm::AuthoredAssertion),
        "repeat retains the last entered value, not the value restored by undo"
    );
}

#[test]
fn the_source_picker_counts_what_a_person_has_still_to_read() {
    let mut workspace = workspace();
    let sources = workspace.enrichment_sources().expect("sources");
    let report = sources
        .iter()
        .find(|source| source.source_id == "src-report-v1")
        .expect("the report is a source of the case");
    assert_eq!(report.profile_role.as_deref(), Some("police_report"));
    assert_eq!(report.profile_author.as_deref(), Some("Officer Mei Chen"));
    assert!(report.has_profile());
    assert_eq!(report.decided, 0, "an inherited default is not a decision");
    assert_eq!(report.outstanding(), report.passages);

    workspace.enrichment_key("a").expect("authored assertion");
    let after = workspace.enrichment_sources().expect("sources");
    let report = after
        .iter()
        .find(|source| source.source_id == "src-report-v1")
        .expect("report");
    assert_eq!(report.decided, 1);
    assert_eq!(report.outstanding(), report.passages - 1);

    let unprofiled = after
        .iter()
        .find(|source| source.source_id == "src-bodycam")
        .expect("the body camera is a source of the case");
    assert!(!unprofiled.has_profile());
}

#[test]
fn entity_autocomplete_offers_the_cases_own_names_nearest_first() {
    let workspace = workspace();
    let matches = workspace.entity_candidates("pat", 10).expect("candidates");
    assert!(
        matches
            .first()
            .is_some_and(|entity| entity.id == "person-patel"),
        "a name that begins with what was typed comes first: {matches:?}"
    );
    assert!(
        workspace
            .entity_candidates("", 10)
            .expect("candidates")
            .len()
            > matches.len(),
        "an empty query offers every name the case holds"
    );
    assert!(
        workspace
            .entity_candidates("no such person", 10)
            .expect("candidates")
            .is_empty()
    );
}

#[test]
fn a_new_entity_is_recorded_beside_the_old_one_and_never_merged_into_it() {
    let mut workspace = workspace();
    let written = workspace
        .create_entity(
            "Patel, A.",
            EntityKind::Person,
            "P. Paralegal",
            Some("person-patel"),
        )
        .expect("entity");
    assert_ne!(written.id, "person-patel");

    let names = workspace
        .entity_candidates("patel", 10)
        .expect("candidates");
    assert_eq!(
        names.len(),
        2,
        "both records survive; the question is recorded, not resolved"
    );

    let queue = workspace
        .render(WorkspaceView::ReviewQueue)
        .expect("review queue");
    assert!(
        queue.contains("possibly_same_person"),
        "the doubt joins the queue for a person to decide: {queue}"
    );
}

#[test]
fn saving_a_profile_supersedes_the_current_version_rather_than_overwriting_it() {
    let mut workspace = workspace();
    let before = workspace
        .source_profile("src-report-v1")
        .expect("profile")
        .expect("the fixture wrote one");

    workspace
        .save_source_profile(&ProposedSourceProfile {
            id: None,
            source_id: "src-report-v1".to_owned(),
            source_role: SourceRole::SupplementalReport,
            author_entity_id: Some("person-chen".to_owned()),
            created_at_claim: Some("2026-01-09T08:00:00".to_owned()),
            default_content_form: Some(ContentForm::AuthoredAssertion),
            default_temporal_stance: Some(TemporalStance::RetrospectiveRecollection),
            default_perception_basis: Some(PerceptionBasis::ReadInSource),
            clock_offset_ms: None,
            clock_offset_basis: None,
            review_state: ReviewState::Reviewed,
            created_by: "P. Paralegal".to_owned(),
            supersedes_profile_id: None,
        })
        .expect("second version");

    let after = workspace
        .source_profile("src-report-v1")
        .expect("profile")
        .expect("current version");
    assert_ne!(after.id, before.id);
    assert_eq!(
        after.supersedes_profile_id.as_deref(),
        Some(before.id.as_str())
    );
    assert_eq!(after.source_role, SourceRole::SupplementalReport);
}

#[test]
fn the_grid_shows_the_swept_field_and_where_its_value_came_from() {
    let mut workspace = workspace();
    let rows = workspace.enrichment_rows().expect("rows");
    assert_eq!(
        rows.len(),
        workspace.enrichment_session().unwrap().rows.len()
    );
    let first = &rows[0];
    assert_eq!(first.number, 1);
    assert!(!first.locator.is_empty(), "every row cites its own locator");
    assert_eq!(first.value, "authored_assertion");
    assert!(
        first.provenance.starts_with("inherited:"),
        "the inheritance is visible, not silent: {}",
        first.provenance
    );

    workspace.enrichment_key("F8").expect("perception sweep");
    let basis = &workspace.enrichment_rows().expect("rows")[0];
    assert_eq!(basis.value, "read_in_source");
    assert!(basis.provenance.starts_with("inherited:"));

    workspace.enrichment_key("F6").expect("time sweep");
    let time = &workspace.enrichment_rows().expect("rows")[0];
    assert_eq!(time.value, "not set", "nothing invents a time");
    assert!(time.needs);
}

#[test]
fn a_deterministic_candidate_is_proposed_before_the_sweep_and_costs_one_key() {
    let mut workspace = workspace();
    workspace.suggest_enrichment().expect("candidate rules");
    workspace
        .open_enrichment("src-report-v1", "P. Paralegal")
        .expect("reopen");

    let rows = workspace.enrichment_rows().expect("rows");
    let proposed = rows
        .iter()
        .find(|row| row.candidate)
        .expect("a cue verb proposes a reported statement");
    assert_eq!(proposed.badge, "MACHINE SUGGESTION");
    assert!(
        proposed.provenance.starts_with("suggest:"),
        "a candidate names the rule that made it: {}",
        proposed.provenance
    );

    let index = usize::try_from(proposed.number - 1).expect("row number");
    workspace.enrichment_select(index).expect("select");
    let before = workspace.enrichment_session().expect("session").keystrokes;
    workspace.enrichment_key("Enter").expect("accept");
    let session = workspace.enrichment_session().expect("session");
    assert_eq!(session.keystrokes - before, 1, "accepting costs one key");
    let accepted = &workspace.enrichment_rows().expect("rows")[index];
    assert!(
        !accepted.candidate,
        "the candidate is now a person's reading"
    );
    assert_ne!(accepted.badge, "MACHINE SUGGESTION");
}

#[test]
fn pointing_at_a_passage_costs_what_walking_to_it_costs() {
    let mut workspace = workspace();
    let before = workspace.enrichment_session().expect("session").keystrokes;
    workspace.enrichment_select(2).expect("third passage");
    let session = workspace.enrichment_session().expect("session");
    assert_eq!(session.cursor, 2);
    assert_eq!(session.keystrokes - before, 1);
    assert!(workspace.enrichment_select(99).is_err());
}

#[test]
fn the_assembled_views_render_their_sources_rather_than_a_conclusion() {
    let workspace = workspace();
    let packets = workspace.render(WorkspaceView::Packets).expect("packets");
    assert!(packets.contains("page 3, paragraph 4"), "{packets}");
    let digest = workspace.render(WorkspaceView::Digest).expect("digest");
    assert!(digest.contains("Recorded sequence"), "{digest}");
    for verdict in ["strong", "weak", "likely", "score", "credible"] {
        assert!(
            !digest.to_lowercase().contains(verdict),
            "the digest reports structure, never a verdict: {verdict}"
        );
    }
}
