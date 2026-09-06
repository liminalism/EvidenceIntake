//! Hand-authored office fixtures.
//!
//! The docket gate the office layer is measured against is an attorney with
//! roughly a hundred and fifty active misdemeanors working out what they must
//! do tomorrow. A fixture of three clients cannot demonstrate that, so this one
//! seeds a caseload at that scale — and then hand-writes, on top of it, the
//! specific tensions the read models have to survive:
//!
//! - one client called on three related matters at a single setting, which must
//!   render as one docket row and not three;
//! - a matter with no evidence case at all, which must report that rather than
//!   an empty posture that looks like a clean case;
//! - a client in custody, whose settings a defender reads differently;
//! - a note that has been revised, so the superseded version is present in the
//!   history and absent from every view;
//! - two clients similar enough to raise a possible-duplicate prompt, which
//!   must be offered and must not be resolved by the software;
//! - an overdue statutory deadline, so the deadline view has something past to
//!   separate from what is merely coming.
//!
//! Seeding is transactional at the statement level and idempotent: a second
//! seed of the same fixture returns the same identifiers without writing twice.

use crate::authoring::{
    ProposedAppearance, ProposedAssignment, ProposedClient, ProposedClientContact, ProposedCourt,
    ProposedDeadline, ProposedMatter, ProposedNote,
};
use crate::civil_date::CivilDate;
use crate::error::Result;
use crate::model::{
    AppearanceType, AssignmentRole, ContactKind, CustodyState, DeadlineOrigin, MatterStatus,
    OfferState, Sex,
};
use crate::store::OfficeStore;

/// Permanent office fixtures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeFixture {
    /// A misdemeanor caseload at the scale the docket gate is measured against.
    MisdemeanorDocket,
}

/// Given names used to build the bulk caseload, paired with the surnames below.
const GIVEN_NAMES: [&str; 20] = [
    "Alex", "Bianca", "Cyrus", "Dara", "Elena", "Fabian", "Grace", "Hector", "Imani", "Jonas",
    "Kalila", "Luis", "Mara", "Nikhil", "Omar", "Priya", "Quinn", "Rosa", "Samir", "Tova",
];

/// Surnames used to build the bulk caseload.
const SURNAMES: [&str; 20] = [
    "Alvarez",
    "Boateng",
    "Castellanos",
    "Duarte",
    "Ellison",
    "Farhi",
    "Gallego",
    "Haddad",
    "Iversen",
    "Jankowski",
    "Kimura",
    "Larkin",
    "Mbeki",
    "Novak",
    "Oyelaran",
    "Petrov",
    "Quiroga",
    "Reyes",
    "Sandoval",
    "Tremblay",
];

/// Misdemeanor charges the bulk caseload draws from.
const CHARGES: [&str; 10] = [
    "Driving while license suspended",
    "Petit larceny",
    "Criminal trespass, second degree",
    "Possession of a controlled substance",
    "Disorderly conduct",
    "Assault, third degree",
    "Reckless driving",
    "Criminal mischief",
    "Obstructing a peace officer",
    "Violation of a protective order",
];

/// How many bulk matters the fixture writes, before the hand-authored ones.
const BULK_MATTERS: usize = 148;

impl OfficeFixture {
    /// Seeds the fixture, anchored on today's local date.
    pub fn seed(self, store: &mut OfficeStore) -> Result<String> {
        let today = store.today()?;
        self.seed_from(store, &today)
    }

    /// Seeds the fixture with the calendar anchored on a given date.
    ///
    /// Tests anchor on a fixed day so a docket assertion does not change meaning
    /// overnight; the CLI and the workspace anchor on today so the fixture is
    /// something a person can actually look at.
    pub fn seed_from(self, store: &mut OfficeStore, anchor: &str) -> Result<String> {
        let Self::MisdemeanorDocket = self;
        let anchor = CivilDate::require(anchor)?;

        // Settings land on court days. An anchor that falls on a weekend would
        // put the whole demonstration on a day no court sits.
        let monday = anchor.week_start();

        let defender = store.user_named("A. Defender", "attorney")?;
        let paralegal = store.user_named("R. Ocampo", "paralegal")?;
        let investigator = store.user_named("T. Whitfield", "investigator")?;
        let supervisor = store.user_named("M. Iqbal", "supervisor")?;

        let district = store.create_court(&ProposedCourt {
            id: Some("court-district-1".to_owned()),
            name: "District Court".to_owned(),
            division: Some("Criminal Division".to_owned()),
            address: Some("400 Court Street".to_owned()),
            room: Some("Courtroom 3B".to_owned()),
        })?;
        let municipal = store.create_court(&ProposedCourt {
            id: Some("court-municipal-1".to_owned()),
            name: "Municipal Court".to_owned(),
            division: None,
            address: Some("18 Civic Plaza".to_owned()),
            room: Some("Courtroom 1".to_owned()),
        })?;
        let judge = store.create_judge(Some(&district), "Hon. P. Nakashima")?;

        // ---- the caseload at scale ---------------------------------------

        for index in 0..BULK_MATTERS {
            let given = GIVEN_NAMES[index % GIVEN_NAMES.len()];
            let surname = SURNAMES[(index / GIVEN_NAMES.len() + index) % SURNAMES.len()];
            let client_id = format!("client-bulk-{index:03}");
            let matter_id = format!("matter-bulk-{index:03}");

            store.create_client(&ProposedClient {
                id: Some(client_id.clone()),
                display_name: format!("{given} {surname}"),
                date_of_birth: Some(
                    CivilDate::new(
                        1968 + (index as i32 % 34),
                        1 + (index as u32 % 12),
                        1 + (index as u32 % 28),
                    )
                    .map_or_else(|| "1990-01-01".to_owned(), CivilDate::to_text),
                ),
                sex: None,
                preferred_language: None,
                notes: None,
                aliases: Vec::new(),
                contacts: vec![ProposedClientContact {
                    kind: ContactKind::Phone,
                    value: format!(
                        "(555) {:03}-{:04}",
                        100 + index % 900,
                        1000 + index * 7 % 9000
                    ),
                    label: Some("mobile".to_owned()),
                    is_primary: true,
                }],
                author_user_id: paralegal.clone(),
            })?;

            let custody = match index % 11 {
                0 => CustodyState::InCustody,
                1 | 2 => CustodyState::ReleasedOnBond,
                3 => CustodyState::Unknown,
                _ => CustodyState::Out,
            };
            store.open_matter(&ProposedMatter {
                id: Some(matter_id.clone()),
                client_id: client_id.clone(),
                caption: format!("State v. {surname}"),
                court_number: Some(format!("CR-2026-{:03}", 100 + index)),
                court_id: Some(if index % 3 == 0 {
                    municipal.clone()
                } else {
                    district.clone()
                }),
                status: Some(MatterStatus::Open),
                custody_state: Some(custody),
                offer_state: Some(match index % 7 {
                    0 => OfferState::Extended,
                    1 => OfferState::UnderAdvisement,
                    2 => OfferState::Rejected,
                    _ => OfferState::None,
                }),
                offer_summary: (index % 7 == 0)
                    .then(|| "Plead to the lesser; 12 months unsupervised".to_owned()),
                charge_summary: Some(CHARGES[index % CHARGES.len()].to_owned()),
                opened_on: Some(monday.add_days(-((index % 90) as i64)).to_text()),
                last_contact_on: Some(monday.add_days(-((index % 21) as i64)).to_text()),
                evidence_case_id: None,
                author_user_id: defender.clone(),
            })?;
            store.assign_to_matter(&ProposedAssignment {
                matter_id: matter_id.clone(),
                user_id: defender.clone(),
                role: AssignmentRole::Attorney,
                assigned_by_user_id: supervisor.clone(),
            })?;

            // Settings spread across the anchor week and the two after it, so
            // "today" and "tomorrow" are never empty and a week view has shape.
            let offset = (index % 15) as i64;
            let day = monday.add_days(offset + offset / 5 * 2);
            store.schedule_appearance(&ProposedAppearance {
                id: Some(format!("appearance-bulk-{index:03}")),
                client_id: client_id.clone(),
                matter_ids: vec![matter_id.clone()],
                court_id: Some(if index % 3 == 0 {
                    municipal.clone()
                } else {
                    district.clone()
                }),
                judge_id: (index % 3 != 0).then(|| judge.clone()),
                appearance_date: day.to_text(),
                appearance_time: Some(format!(
                    "{:02}:{:02}",
                    9 + index % 4,
                    if index % 2 == 0 { 0 } else { 30 }
                )),
                appearance_type: match index % 5 {
                    0 => AppearanceType::Arraignment,
                    1 => AppearanceType::Pretrial,
                    2 => AppearanceType::Motion,
                    3 => AppearanceType::Plea,
                    _ => AppearanceType::Status,
                },
                notes: None,
                author_user_id: paralegal.clone(),
            })?;

            if index % 6 == 0 {
                store.record_deadline(&ProposedDeadline {
                    id: Some(format!("deadline-bulk-{index:03}")),
                    matter_id: matter_id.clone(),
                    description: "File discovery motion".to_owned(),
                    due_date: monday.add_days((index % 12) as i64).to_text(),
                    origin: DeadlineOrigin::CourtOrdered,
                    author_user_id: defender.clone(),
                })?;
            }
        }

        // ---- one client, three matters, one setting -----------------------
        //
        // The row that proves the calendar model: Rivera is called at nine on
        // three related cases, and the docket must show that once.

        let rivera = store.create_client(&ProposedClient {
            id: Some("client-rivera".to_owned()),
            display_name: "Alex Rivera".to_owned(),
            date_of_birth: Some("1994-03-17".to_owned()),
            sex: Some(Sex::Male),
            preferred_language: Some("Spanish".to_owned()),
            notes: Some("Works nights; reach after 14:00.".to_owned()),
            aliases: vec!["Alejandro Rivera".to_owned()],
            contacts: vec![
                ProposedClientContact {
                    kind: ContactKind::Phone,
                    value: "(555) 481-2290".to_owned(),
                    label: Some("mobile".to_owned()),
                    is_primary: true,
                },
                ProposedClientContact {
                    kind: ContactKind::Email,
                    value: "a.rivera@example.org".to_owned(),
                    label: None,
                    is_primary: true,
                },
            ],
            author_user_id: paralegal.clone(),
        })?;

        let rivera_matters = [
            (
                "matter-rivera-1",
                "State v. Rivera (theft)",
                "CR-2026-491",
                "Petit larceny",
            ),
            (
                "matter-rivera-2",
                "State v. Rivera (trespass)",
                "CR-2026-492",
                "Criminal trespass, second degree",
            ),
            (
                "matter-rivera-3",
                "State v. Rivera (FTA)",
                "CR-2026-517",
                "Failure to appear",
            ),
        ];
        for (index, (id, caption, number, charge)) in rivera_matters.iter().enumerate() {
            store.open_matter(&ProposedMatter {
                id: Some((*id).to_owned()),
                client_id: rivera.id.clone(),
                caption: (*caption).to_owned(),
                court_number: Some((*number).to_owned()),
                court_id: Some(district.clone()),
                status: Some(MatterStatus::Open),
                custody_state: Some(CustodyState::Out),
                offer_state: Some(if index == 0 {
                    OfferState::Extended
                } else {
                    OfferState::None
                }),
                offer_summary: (index == 0).then(|| "Time served plus restitution".to_owned()),
                charge_summary: Some((*charge).to_owned()),
                opened_on: Some(monday.add_days(-40).to_text()),
                last_contact_on: Some(monday.add_days(-3).to_text()),
                // Only the first carries discovery worth collating. The other
                // two must report that they have none, not an empty posture.
                evidence_case_id: (index == 0).then(|| "case-hit-run-001".to_owned()),
                author_user_id: defender.clone(),
            })?;
            store.assign_to_matter(&ProposedAssignment {
                matter_id: (*id).to_owned(),
                user_id: defender.clone(),
                role: AssignmentRole::Attorney,
                assigned_by_user_id: supervisor.clone(),
            })?;
        }
        store.link_matters("matter-rivera-1", "matter-rivera-2", "companion")?;
        store.link_matters("matter-rivera-1", "matter-rivera-3", "related")?;
        store.assign_to_matter(&ProposedAssignment {
            matter_id: "matter-rivera-1".to_owned(),
            user_id: investigator.clone(),
            role: AssignmentRole::Investigator,
            assigned_by_user_id: defender.clone(),
        })?;

        let rivera_setting = store.schedule_appearance(&ProposedAppearance {
            id: Some("appearance-rivera-consolidated".to_owned()),
            client_id: rivera.id.clone(),
            matter_ids: rivera_matters
                .iter()
                .map(|(id, ..)| (*id).to_owned())
                .collect(),
            court_id: Some(district.clone()),
            judge_id: Some(judge.clone()),
            appearance_date: monday.add_days(1).to_text(),
            appearance_time: Some("09:00".to_owned()),
            appearance_type: AppearanceType::Pretrial,
            notes: Some("All three called together.".to_owned()),
            author_user_id: paralegal.clone(),
        })?;
        // One of the three is being handled differently inside the shared
        // setting, which is an override rather than a second row.
        store.link_matter_to_appearance(
            &rivera_setting,
            "matter-rivera-3",
            Some("Passed for plea; the other two proceed."),
        )?;

        store.record_deadline(&ProposedDeadline {
            id: Some("deadline-rivera-speedy".to_owned()),
            matter_id: "matter-rivera-1".to_owned(),
            description: "Speedy trial demand expires".to_owned(),
            due_date: monday.add_days(-4).to_text(),
            origin: DeadlineOrigin::Statutory,
            author_user_id: defender.clone(),
        })?;
        store.record_deadline(&ProposedDeadline {
            id: Some("deadline-rivera-motions".to_owned()),
            matter_id: "matter-rivera-2".to_owned(),
            description: "Suppression motion due".to_owned(),
            due_date: monday.add_days(9).to_text(),
            origin: DeadlineOrigin::CourtOrdered,
            author_user_id: defender.clone(),
        })?;

        // ---- a client in custody -----------------------------------------

        let okonkwo = store.create_client(&ProposedClient {
            id: Some("client-okonkwo".to_owned()),
            display_name: "Ngozi Okonkwo".to_owned(),
            date_of_birth: Some("1988-11-02".to_owned()),
            sex: Some(Sex::Female),
            preferred_language: None,
            notes: None,
            aliases: Vec::new(),
            contacts: vec![ProposedClientContact {
                kind: ContactKind::Emergency,
                value: "(555) 902-4417".to_owned(),
                label: Some("sister".to_owned()),
                is_primary: true,
            }],
            author_user_id: paralegal.clone(),
        })?;
        store.open_matter(&ProposedMatter {
            id: Some("matter-okonkwo-1".to_owned()),
            client_id: okonkwo.id.clone(),
            caption: "State v. Okonkwo".to_owned(),
            court_number: Some("CR-2026-604".to_owned()),
            court_id: Some(district.clone()),
            status: Some(MatterStatus::Open),
            custody_state: Some(CustodyState::InCustody),
            offer_state: Some(OfferState::UnderAdvisement),
            offer_summary: Some("Ninety days, credit for time served".to_owned()),
            charge_summary: Some("Assault, third degree".to_owned()),
            opened_on: Some(monday.add_days(-9).to_text()),
            last_contact_on: Some(monday.add_days(-1).to_text()),
            evidence_case_id: Some("case-vehicle-stop-001".to_owned()),
            author_user_id: defender.clone(),
        })?;
        store.assign_to_matter(&ProposedAssignment {
            matter_id: "matter-okonkwo-1".to_owned(),
            user_id: defender.clone(),
            role: AssignmentRole::Attorney,
            assigned_by_user_id: supervisor.clone(),
        })?;
        store.schedule_appearance(&ProposedAppearance {
            id: Some("appearance-okonkwo-bond".to_owned()),
            client_id: okonkwo.id.clone(),
            matter_ids: vec!["matter-okonkwo-1".to_owned()],
            court_id: Some(district.clone()),
            judge_id: Some(judge.clone()),
            appearance_date: monday.add_days(1).to_text(),
            appearance_time: Some("08:30".to_owned()),
            appearance_type: AppearanceType::Motion,
            notes: Some("Bond review.".to_owned()),
            author_user_id: paralegal.clone(),
        })?;

        // ---- a revised note -----------------------------------------------

        let first = store.write_note(&ProposedNote {
            id: Some("note-rivera-plan-v1".to_owned()),
            client_id: None,
            matter_id: Some("matter-rivera-1".to_owned()),
            appearance_id: None,
            body: "Client says he was at work. Ask @investigator for the timesheet.".to_owned(),
            author_user_id: defender.clone(),
        })?;
        store.revise_note(
            &first.id,
            &ProposedNote {
                id: Some("note-rivera-plan-v2".to_owned()),
                client_id: None,
                matter_id: Some("matter-rivera-1".to_owned()),
                appearance_id: None,
                body: "Client says he was at work until 22:00. Timesheet obtained; \
                       @investigator to confirm the supervisor will testify. \
                       @supervisor flagged for a second chair."
                    .to_owned(),
                author_user_id: defender.clone(),
            },
        )?;
        store.write_note(&ProposedNote {
            id: Some("note-rivera-client".to_owned()),
            client_id: Some(rivera.id.clone()),
            matter_id: None,
            appearance_id: None,
            body: "Follows the person, not the case: reachable after 14:00 only.".to_owned(),
            author_user_id: paralegal.clone(),
        })?;
        store.write_note(&ProposedNote {
            id: Some("note-rivera-setting".to_owned()),
            client_id: None,
            matter_id: None,
            appearance_id: Some(rivera_setting.clone()),
            body: "Three cases called together; confirm the FTA is passed.".to_owned(),
            author_user_id: paralegal.clone(),
        })?;

        // ---- somebody who might already be in the system -------------------
        //
        // Same name, different date of birth, overlapping phone number. The
        // office must offer this as a question and must not answer it.

        store.create_client(&ProposedClient {
            id: Some("client-rivera-possible".to_owned()),
            display_name: "Alex Rivera".to_owned(),
            date_of_birth: Some("1994-03-18".to_owned()),
            sex: None,
            preferred_language: None,
            notes: Some("Walked in on a new charge; may be the same person.".to_owned()),
            aliases: Vec::new(),
            contacts: vec![ProposedClientContact {
                kind: ContactKind::Phone,
                value: "555-481-2290".to_owned(),
                label: None,
                is_primary: true,
            }],
            author_user_id: paralegal.clone(),
        })?;

        Ok(rivera.id)
    }
}
