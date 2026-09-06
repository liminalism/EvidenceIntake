//! Where the office layer meets the evidence kernel.
//!
//! This is the only module in the workspace that holds both databases open at
//! once, and it exists so that nothing else has to. `office_core` has no
//! dependency on this crate and no way to open an evidence database; that is
//! what makes the privileged boundary structural rather than a filter. Advocacy
//! items, annotations and decision briefs are not excluded from a docket row —
//! they are unreachable from the crate that builds one.
//!
//! What crosses the boundary is one identifier in one direction. A matter may
//! carry an `evidence_case_id`; this module resolves it against a [`Store`] and
//! folds the kernel's [`CaseStanding`](crate::views::CaseStanding) into a short
//! structural summary a docket row can carry.
//!
//! **Rule 35 governs here exactly as it governs the standing view.** The
//! summary counts what a case rests on and names where it is thin. It does not
//! score, rank, or predict, and a matter with no evidence case reports that it
//! has none rather than reporting zeroes, which would read as a clean case.

use std::path::Path;

use serde::Serialize;

use office_core::{
    CivilDate, DocketDay, DocketEntry, MatterProfile, OfficeStore, UpcomingDeadlines,
};

use crate::error::{Error, Result};
use crate::store::Store;

/// The office half of a workspace, and the bridge to the evidence kernel.
pub struct OfficeDesk {
    office: OfficeStore,
}

impl OfficeDesk {
    /// Opens or creates the office database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            office: OfficeStore::open(path)?,
        })
    }

    /// Creates a temporary in-memory office, primarily for fixtures and tests.
    pub fn in_memory() -> Result<Self> {
        Ok(Self {
            office: OfficeStore::in_memory()?,
        })
    }

    /// The office database beside an evidence database.
    ///
    /// `office.sqlite` sits next to `evidence.sqlite` rather than inside it,
    /// which is the file-level shape of the same boundary the schemas keep:
    /// two databases, no shared transaction, no cross-database foreign key.
    pub fn beside(evidence_database: &Path) -> std::path::PathBuf {
        evidence_database.with_file_name("office.sqlite")
    }

    /// The office store, for reads and writes that never touch the kernel.
    pub fn store(&self) -> &OfficeStore {
        &self.office
    }

    /// The office store, for writes that need a transaction.
    pub fn store_mut(&mut self) -> &mut OfficeStore {
        &mut self.office
    }

    /// Today's local date, in the stored `YYYY-MM-DD` form.
    pub fn today(&self) -> Result<String> {
        Ok(self.office.today()?)
    }

    /// One day's docket, each setting carrying the evidence posture of the
    /// cases it covers.
    ///
    /// The kernel is consulted once per distinct linked case, not once per
    /// matter, because three matters on one setting sharing a case would
    /// otherwise recompute the same standing three times.
    pub fn court_docket(&self, evidence: &Store, date: &str) -> Result<CourtDocket> {
        let day = self.office.docket_day(date)?;
        let rows = day
            .settings
            .iter()
            .map(|entry| self.docket_row(evidence, entry))
            .collect::<Result<Vec<_>>>()?;
        Ok(CourtDocket { day, rows })
    }

    /// The week containing a date, Monday through Sunday, with posture.
    pub fn court_week(&self, evidence: &Store, containing: &str) -> Result<Vec<CourtDocket>> {
        let day = CivilDate::require(containing).map_err(office_error)?;
        let mut week = Vec::with_capacity(7);
        let mut cursor = day.week_start();
        let end = day.week_end();
        while cursor <= end {
            week.push(self.court_docket(evidence, &cursor.to_text())?);
            cursor = cursor.add_days(1);
        }
        Ok(week)
    }

    /// Deadlines still owed, with no evidence involved.
    pub fn upcoming_deadlines(&self, as_of: &str, within_days: u32) -> Result<UpcomingDeadlines> {
        Ok(self.office.upcoming_deadlines(as_of, within_days)?)
    }

    /// One matter, with the standing of its evidence case when it has one.
    pub fn matter_view(&self, evidence: &Store, matter_id: &str) -> Result<MatterView> {
        let matter = self.office.matter_profile(matter_id)?;
        let posture = match matter.evidence_case_id.as_deref() {
            Some(case_id) => self.posture(evidence, case_id)?,
            None => None,
        };
        Ok(MatterView { matter, posture })
    }

    fn docket_row(&self, evidence: &Store, entry: &DocketEntry) -> Result<DocketRow> {
        let mut postures = Vec::new();
        // The kernel is asked once per distinct case, but the row counts
        // *matters*: two matters pointing at one missing case are two broken
        // links a person has to fix, not one.
        let mut resolved: Vec<(&str, bool)> = Vec::new();
        let mut unresolved = 0_usize;
        for case_id in entry
            .matters
            .iter()
            .filter_map(|line| line.evidence_case_id.as_deref())
        {
            let held = if let Some((_, held)) = resolved.iter().find(|(seen, _)| *seen == case_id) {
                *held
            } else {
                let posture = self.posture(evidence, case_id)?;
                let held = posture.is_some();
                if let Some(posture) = posture {
                    postures.push(posture);
                }
                resolved.push((case_id, held));
                held
            };
            if !held {
                unresolved += 1;
            }
        }
        let matters_without_evidence = counted(
            entry
                .matters
                .iter()
                .filter(|line| line.evidence_case_id.is_none())
                .count(),
        );
        Ok(DocketRow {
            entry: entry.clone(),
            postures,
            matters_without_evidence,
            matters_with_a_missing_case: counted(unresolved),
        })
    }

    /// The structural posture of one evidence case.
    ///
    /// `None` when the kernel does not hold the case at all. A matter pointing
    /// at a case that was never opened is a broken link, and reporting it as an
    /// empty posture would hide that.
    pub fn posture(&self, evidence: &Store, case_id: &str) -> Result<Option<EvidencePosture>> {
        let case = crate::model::CaseId(case_id.to_owned());
        let standing = match evidence.case_standing(&case) {
            Ok(standing) => standing,
            Err(Error::NotFound { .. }) => return Ok(None),
            Err(other) => return Err(other),
        };

        let elements = standing
            .charges
            .iter()
            .flat_map(|charge| charge.elements.iter());
        let mut contested_elements = 0;
        let mut sole_source_elements = 0;
        let mut unmapped_elements = 0;
        let mut unchecked_support = 0;
        let mut unbacked_mappings = 0;
        for element in elements {
            if element.opposing > 0 {
                contested_elements += 1;
            }
            if element.sole_source.is_some() {
                sole_source_elements += 1;
            }
            if element.supporting == 0 {
                unmapped_elements += 1;
            }
            unchecked_support += element.unchecked_support;
            unbacked_mappings += element.unbacked;
        }

        let missing_referenced_evidence = counted(
            standing
                .open_gaps
                .iter()
                .filter(|gap| gap.analyzer == "unresolved-reference")
                .count(),
        );
        let gaps_touching_a_charge = counted(
            standing
                .open_gaps
                .iter()
                .filter(|gap| !gap.bears_on.is_empty())
                .count(),
        );

        Ok(Some(EvidencePosture {
            evidence_case_id: case_id.to_owned(),
            case_name: standing.case_name,
            elements_in_all: standing
                .charges
                .iter()
                .map(|charge| counted(charge.elements.len()))
                .sum(),
            contested_elements,
            unmapped_elements,
            sole_source_elements,
            unchecked_support,
            unbacked_mappings,
            load_bearing_sources: counted(standing.load_bearing_sources.len()),
            live_disputes: counted(standing.live_disputes.len()),
            open_gaps: counted(standing.open_gaps.len()),
            gaps_touching_a_charge,
            missing_referenced_evidence,
        }))
    }
}

/// One day of court, each setting carrying what its cases rest on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CourtDocket {
    /// The office's own day, one entry per setting.
    pub day: DocketDay,
    /// The same settings, in the same order, with evidence posture attached.
    pub rows: Vec<DocketRow>,
}

/// One docket row: a setting, and the standing of whatever it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DocketRow {
    /// The setting and every matter it covers.
    pub entry: DocketEntry,
    /// One posture per distinct evidence case behind the setting's matters.
    ///
    /// Empty when none of them has a linked case, which is not the same as
    /// their evidence being in good order.
    pub postures: Vec<EvidencePosture>,
    /// Matters on this setting with no evidence case at all.
    pub matters_without_evidence: u32,
    /// Matters naming an evidence case the kernel does not hold.
    ///
    /// A third state, and never folded into either of the others: a matter that
    /// was never linked is ordinary, while a matter pointing at a case that is
    /// not there is a broken link somebody has to go and fix. The two databases
    /// have no foreign key between them precisely so that neither can be
    /// silently rewritten by the other, which makes reporting this the office
    /// layer's job.
    pub matters_with_a_missing_case: u32,
}

impl DocketRow {
    /// A short structural line for a grid column.
    ///
    /// Counts and absences only. There is deliberately no phrase here that
    /// ranks a case, calls one strong or weak, or suggests what to do — the
    /// same rule the standing view is written under.
    pub fn posture_line(&self) -> String {
        if self.postures.is_empty() {
            return match (
                self.matters_with_a_missing_case,
                self.matters_without_evidence,
            ) {
                (0, 0) => "no matters".to_owned(),
                (0, _) => "no evidence case linked".to_owned(),
                (missing, 0) => missing_case_phrase(missing),
                (missing, without) => {
                    format!("{}, {without} without a case", missing_case_phrase(missing))
                }
            };
        }
        let mut parts = Vec::new();
        let contested: u32 = self.postures.iter().map(|p| p.contested_elements).sum();
        let sole: u32 = self.postures.iter().map(|p| p.sole_source_elements).sum();
        let unchecked: u32 = self.postures.iter().map(|p| p.unchecked_support).sum();
        let missing: u32 = self
            .postures
            .iter()
            .map(|p| p.missing_referenced_evidence)
            .sum();
        if contested > 0 {
            parts.push(format!("{contested} contested"));
        }
        if sole > 0 {
            parts.push(format!("{sole} sole-source"));
        }
        if unchecked > 0 {
            parts.push(format!("{unchecked} unchecked"));
        }
        if missing > 0 {
            parts.push(format!("{missing} referenced, missing"));
        }
        if self.matters_with_a_missing_case > 0 {
            parts.push(missing_case_phrase(self.matters_with_a_missing_case));
        }
        if self.matters_without_evidence > 0 {
            parts.push(format!("{} without a case", self.matters_without_evidence));
        }
        if parts.is_empty() {
            "nothing outstanding".to_owned()
        } else {
            parts.join(", ")
        }
    }
}

/// How a broken link between the two databases is named on a docket row.
///
/// Stated as the fact it is — the case named is not in the evidence database —
/// rather than as an instruction or an alarm.
fn missing_case_phrase(matters: u32) -> String {
    if matters == 1 {
        "1 case linked but not found".to_owned()
    } else {
        format!("{matters} cases linked but not found")
    }
}

/// What one evidence case rests on, as structure.
///
/// Every field is a count or a name. None of them is a score, a rank, or a
/// prediction, and none of them can be combined into one: an element resting on
/// a single unchecked source is a different problem from six contested
/// elements, and collapsing the two into a number would be the verdict the
/// kernel refuses to give.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidencePosture {
    /// The kernel case this summarizes.
    pub evidence_case_id: String,
    /// Its name in the kernel.
    pub case_name: String,
    /// Statutory elements across every charge.
    pub elements_in_all: u32,
    /// Elements with evidence mapped both ways.
    pub contested_elements: u32,
    /// Elements nothing has been mapped to as supporting.
    pub unmapped_elements: u32,
    /// Elements whose support all traces to one source.
    pub sole_source_elements: u32,
    /// Supporting propositions nobody has checked any evidence for.
    pub unchecked_support: u32,
    /// Propositions mapped to an element that no source-grounded evidence reaches.
    pub unbacked_mappings: u32,
    /// Sources that alone carry the support for at least one element.
    pub load_bearing_sources: u32,
    /// Propositions with evidence pulling both ways.
    pub live_disputes: u32,
    /// Gaps the analyzers reported.
    pub open_gaps: u32,
    /// Of those, the ones bearing on a charge.
    pub gaps_touching_a_charge: u32,
    /// References to evidence that resolve to no source in the case.
    pub missing_referenced_evidence: u32,
}

/// One matter, with the standing of its evidence case when it has one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MatterView {
    /// Everything the office holds about the matter.
    pub matter: MatterProfile,
    /// What its discovery rests on. `None` when no case is linked, which is
    /// reported as such rather than as an empty posture.
    pub posture: Option<EvidencePosture>,
}

/// The office layer's failures, carried across as text.
///
/// Deliberately not a `#[from]` variant holding the office error itself. The
/// kernel's error type staying free of office types is the same boundary the
/// schemas keep, one level up: this module knows about both, and neither of
/// them knows about the other.
impl From<office_core::Error> for Error {
    fn from(error: office_core::Error) -> Self {
        Self::Office(error.to_string())
    }
}

fn office_error(error: office_core::Error) -> Error {
    Error::from(error)
}

/// A count, narrowed to the width the read models report in.
///
/// Saturating rather than wrapping: a posture that understated what a case
/// rests on because a count wrapped would be a quiet lie in exactly the view
/// that exists to prevent one. No case reaches four billion of anything.
fn counted(items: usize) -> u32 {
    u32::try_from(items).unwrap_or(u32::MAX)
}
