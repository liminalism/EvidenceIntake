//! What the case-standing view may and may not say.

use evidence_intake::{
    CaseId, CaseStanding, DemoFixture, ElementAssessment, ElementStanding, ProposedElementMapping,
    ProposedProposition, Store,
};

fn hit_and_run() -> (Store, CaseId) {
    let mut store = Store::in_memory().expect("in-memory store");
    let case = DemoFixture::HitAndRun.seed(&mut store).expect("seed");
    (store, case)
}

fn element<'a>(standing: &'a CaseStanding, charge: &str, ordinal: u32) -> &'a ElementStanding {
    standing
        .charges
        .iter()
        .find(|item| item.charge.contains(charge))
        .unwrap_or_else(|| panic!("charge matching `{charge}`"))
        .elements
        .iter()
        .find(|item| item.ordinal == ordinal)
        .unwrap_or_else(|| panic!("element {ordinal}"))
}

/// The whole point of the view. An element whose support arrives through one
/// source fails entirely if that source does, and a defender deciding where to
/// spend a motion needs that named rather than inferred.
#[test]
fn an_element_supported_through_one_source_names_that_source() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");

    let damage = element(&standing, "property-damage collision", 3);
    assert_eq!(damage.sources_behind_support, 1);
    assert_eq!(
        damage.sole_source.as_deref(),
        Some("Damage assessment OCR.pdf")
    );

    let load_bearing = standing
        .load_bearing_sources
        .iter()
        .find(|source| source.source == "Damage assessment OCR.pdf")
        .expect("the sole source is reported as load-bearing");
    assert!(
        load_bearing
            .sole_support_for
            .iter()
            .any(|label| label.contains("element 3")),
        "{:?}",
        load_bearing.sole_support_for
    );
}

/// Several propositions quoting one report are not several sources. Counting
/// mappings instead of originals would report an element as broadly supported
/// when everything it rests on came from one page.
#[test]
fn propositions_are_counted_separately_from_the_sources_under_them() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");

    let failure_to_stop = element(&standing, "Leaving the scene", 4);
    assert_eq!(failure_to_stop.supporting, 1);
    assert_eq!(
        failure_to_stop.sources_behind_support, 2,
        "one proposition can rest on more than one original"
    );
    assert!(
        failure_to_stop.sole_source.is_none(),
        "an element resting on two sources has no single point of failure"
    );
}

/// Evidence nobody has opened is counted, not quietly treated as established.
#[test]
fn support_no_person_has_checked_is_counted() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");

    assert_eq!(
        element(&standing, "Leaving the scene", 4).unchecked_support,
        1
    );
}

/// A proposition mapped to an element with nothing source-grounded behind it is
/// an assertion, not evidence, and the element it is filed under should say so.
#[test]
fn a_mapped_proposition_with_no_evidence_is_reported_as_unbacked() {
    let (mut store, case) = hit_and_run();
    let bare = store
        .author_proposition(
            &case,
            &ProposedProposition {
                id: None,
                text: "Morgan's brakes had failed before the collision.".to_owned(),
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("author");
    store
        .map_element(
            &case,
            &ProposedElementMapping {
                id: None,
                element_id: "hr-el-fi-leave".to_owned(),
                proposition_id: bare.id,
                assessment: ElementAssessment::Opposes,
                notes: None,
                author: "A. Reyes".to_owned(),
            },
        )
        .expect("map");

    let standing = store.case_standing(&case).expect("standing");
    let stop = element(&standing, "Leaving the scene", 4);
    assert_eq!(stop.opposing, 1);
    assert_eq!(stop.unbacked, 1, "nothing source-grounded reaches it");
}

/// Evidence pulling both ways is the contested ground, not a defect to resolve.
/// It is reported with the elements it bears on so a defender can see which
/// part of the charge the fight is actually over.
#[test]
fn a_proposition_with_evidence_both_ways_is_reported_with_the_elements_it_bears_on() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");

    let knowledge = standing
        .live_disputes
        .iter()
        .find(|dispute| dispute.id == "hr-prop-knowledge")
        .expect("the fixture's contested knowledge proposition");
    assert!(knowledge.supporting_evidence > 0);
    assert!(knowledge.contradicting_evidence > 0);
    assert!(
        knowledge
            .bears_on
            .iter()
            .any(|label| label.contains("element 2")),
        "{:?}",
        knowledge.bears_on
    );
}

/// A gap in a corner of the file and a gap under a charged element are not the
/// same call on an hour of a defender's time.
#[test]
fn gaps_touching_a_charge_are_reported_before_gaps_that_touch_none() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");

    let first_untouched = standing
        .open_gaps
        .iter()
        .position(|gap| gap.bears_on.is_empty());
    let last_touching = standing
        .open_gaps
        .iter()
        .rposition(|gap| !gap.bears_on.is_empty());
    if let (Some(untouched), Some(touching)) = (first_untouched, last_touching) {
        assert!(
            touching < untouched,
            "every gap bearing on a charge must precede every gap bearing on none"
        );
    }
    assert!(
        standing
            .open_gaps
            .iter()
            .any(|gap| !gap.bears_on.is_empty()),
        "the fixture holds gaps that touch a charged element"
    );
}

/// The view reports structure, never a verdict. No field carries a score, a
/// strength, a likelihood, or a ranking of one element against another, because
/// the moment the tool says which element is weak it has made the argument for
/// the person whose job that is.
#[test]
fn nothing_in_the_standing_view_scores_the_case() {
    let (store, case) = hit_and_run();
    let standing = store.case_standing(&case).expect("standing");
    let rendered = serde_json::to_string(&standing).expect("serialize");

    for forbidden in [
        "score",
        "strength",
        "weak",
        "likelihood",
        "probability",
        "confidence",
        "rank",
        "winnable",
        "recommend",
    ] {
        assert!(
            !rendered.to_lowercase().contains(forbidden),
            "the standing view must not speak in terms of `{forbidden}`"
        );
    }
}

/// One case's charges never surface another's material, including through
/// `element_links`, which carries no case column of its own.
#[test]
fn standing_is_scoped_to_its_case() {
    let (mut store, hit_run) = hit_and_run();
    let vehicle_stop = DemoFixture::VehicleStop.seed(&mut store).expect("seed");

    let standing = store.case_standing(&vehicle_stop).expect("standing");
    assert!(
        standing
            .charges
            .iter()
            .all(|charge| !charge.charge.contains("Leaving the scene")),
        "another case's charges must not appear"
    );
    assert!(
        store
            .case_standing(&hit_run)
            .expect("standing")
            .live_disputes
            .iter()
            .all(|dispute| dispute.id.starts_with("hr-")),
        "another case's propositions must not appear"
    );
}
