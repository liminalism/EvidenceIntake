//! Hand-authored case fixtures used to validate collation before ingestion.

use rusqlite::{OptionalExtension, params};

use crate::{CaseId, Error, Result, Store};

/// A permanent, manually curated case fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoFixture {
    /// A vehicle stop with inconsistent clocks, a missing recording, and a
    /// contested possession proposition.
    VehicleStop,
    /// A hit-and-run reconstruction with post-event reports, transcript
    /// evidence, scene-model observations, and an unsupported DUI theory.
    HitAndRun,
}

impl DemoFixture {
    /// Stable case identifier used by the fixture.
    pub const fn case_id(self) -> &'static str {
        match self {
            Self::VehicleStop => "case-vehicle-stop-001",
            Self::HitAndRun => "case-hit-run-001",
        }
    }

    /// Inserts the fixture atomically. Re-seeding an existing fixture is safe.
    pub fn seed(self, store: &mut Store) -> Result<CaseId> {
        let id = self.case_id();
        let exists = store
            .connection
            .query_row("SELECT 1 FROM cases WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some();
        if exists {
            return Ok(CaseId(id.to_owned()));
        }

        let transaction = store.connection.transaction()?;
        match self {
            Self::VehicleStop => seed_vehicle_stop(&transaction)?,
            Self::HitAndRun => seed_hit_and_run(&transaction)?,
        }
        transaction.commit()?;
        Ok(CaseId(id.to_owned()))
    }
}

fn seed_hit_and_run(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction
        .execute_batch(include_str!("../fixtures/hit_and_run.sql"))
        .map_err(|error| Error::InvalidFixture(error.to_string()))?;

    let proposition_count: u32 = transaction.query_row(
        "SELECT count(*) FROM propositions WHERE case_id = ?1",
        params!["case-hit-run-001"],
        |row| row.get(0),
    )?;
    if proposition_count < 8 {
        return Err(Error::InvalidFixture(
            "hit-and-run fixture lost required contested propositions".to_owned(),
        ));
    }
    Ok(())
}

fn seed_vehicle_stop(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute_batch(
        r"
        INSERT INTO cases (id, name, reference, jurisdiction)
        VALUES ('case-vehicle-stop-001', 'State v. Rivera — fixture', 'PD-2026-0042', 'Example');

        INSERT INTO productions (id, case_id, label, received_at, producing_party, notes) VALUES
          ('prod-01', 'case-vehicle-stop-001', 'Initial production', '2026-01-12T09:00:00Z', 'Prosecution', 'Files 1–3'),
          ('prod-02', 'case-vehicle-stop-001', 'Supplemental production', '2026-01-19T15:30:00Z', 'Prosecution', 'Late supplemental narrative');

        INSERT INTO sources
          (id, case_id, production_id, logical_name, media_type, sha256, byte_length, review_state, integrity_status, supersedes_source_id)
        VALUES
          ('src-report-v1', 'case-vehicle-stop-001', 'prod-01', 'Officer Chen report.pdf', 'application/pdf',
           'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa', 281044, 'verified', 'available', NULL),
          ('src-bodycam', 'case-vehicle-stop-001', 'prod-01', 'Chen BWC 0042.mp4', 'video/mp4',
           'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb', 18441440, 'reviewed', 'truncated', NULL),
          ('src-dispatch', 'case-vehicle-stop-001', 'prod-01', 'CAD event 26-110.txt', 'text/plain',
           'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc', 4096, 'verified', 'available', NULL),
          ('src-report-v2', 'case-vehicle-stop-001', 'prod-02', 'Officer Chen supplemental.pdf', 'application/pdf',
           'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd', 91341, 'unreviewed', 'available', 'src-report-v1'),
          ('src-backup-missing', 'case-vehicle-stop-001', 'prod-01', 'Officer Chen backup BWC', 'video/unknown',
           'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee', 0, 'unreviewed', 'missing', NULL);

        INSERT INTO source_segments (id, source_id, locator, page, start_ms, end_ms) VALUES
          ('seg-report-p3', 'src-report-v1', 'page 3, paragraph 4', 3, NULL, NULL),
          ('seg-report-p4', 'src-report-v1', 'page 4, paragraph 2', 4, NULL, NULL),
          ('seg-bodycam-consent', 'src-bodycam', '00:04:10–00:04:32', NULL, 250000, 272000),
          ('seg-bodycam-gap', 'src-bodycam', '00:04:33–00:05:41', NULL, 273000, 341000),
          ('seg-dispatch', 'src-dispatch', 'lines 18–23', NULL, NULL, NULL),
          ('seg-supp-p2', 'src-report-v2', 'page 2, paragraph 1', 2, NULL, NULL),
          ('seg-missing', 'src-backup-missing', 'entire expected source', NULL, NULL, NULL);

        INSERT INTO entities (id, case_id, kind, display_name, is_client, notes) VALUES
          ('person-client', 'case-vehicle-stop-001', 'person', 'Alex Rivera', 1, 'Client'),
          ('person-chen', 'case-vehicle-stop-001', 'person', 'Officer Mei Chen', 0, 'Stopping officer'),
          ('person-patel', 'case-vehicle-stop-001', 'person', 'Jordan Patel', 0, 'Passenger and witness'),
          ('location-stop', 'case-vehicle-stop-001', 'location', '400 block of Oak Street', 0, NULL),
          ('object-handgun', 'case-vehicle-stop-001', 'object', 'Recovered handgun', 0, NULL);

        INSERT INTO content
          (id, case_id, segment_id, kind, text, speaker_entity_id, attributed_to_entity_id,
           parent_content_id, raw_time, asserted_time, normalized_start, normalized_end,
           time_basis, location_text, extractor, extractor_confidence, review_state)
        VALUES
          ('content-report-consent', 'case-vehicle-stop-001', 'seg-report-p3', 'document_assertion',
           'Rivera gave verbal consent to search the vehicle.', 'person-chen', 'person-client',
           NULL, NULL, '2026-01-08T22:14:00', NULL, NULL,
           'Officer report narrative; device clock not stated', '400 block of Oak Street',
           'human_fixture', 1.0, 'verified'),
          ('content-bodycam-question', 'case-vehicle-stop-001', 'seg-bodycam-consent', 'statement',
           'You do not mind if I take a quick look, right?', 'person-chen', NULL,
           NULL, 'BWC 22:18:10', '2026-01-08T22:18:10', '2026-01-08T22:14:08Z', '2026-01-08T22:14:12Z',
           'proposed -00:04:02 BWC clock correction from dispatch tone', '400 block of Oak Street',
           'human_fixture', 1.0, 'verified'),
          ('content-client-answer', 'case-vehicle-stop-001', 'seg-bodycam-consent', 'statement',
           'I guess I cannot stop you.', 'person-client', NULL,
           NULL, 'BWC 22:18:14', '2026-01-08T22:18:14', '2026-01-08T22:14:12Z', '2026-01-08T22:14:16Z',
           'proposed -00:04:02 BWC clock correction from dispatch tone', '400 block of Oak Street',
           'human_fixture', 1.0, 'verified'),
          ('content-gap', 'case-vehicle-stop-001', 'seg-bodycam-gap', 'recording_gap',
           'The body-camera file ends during the search and resumes in no produced file.',
           NULL, NULL, NULL, 'BWC 22:18:33–22:19:41', NULL, '2026-01-08T22:14:31Z', '2026-01-08T22:15:39Z',
           'same proposed BWC clock correction', '400 block of Oak Street',
           'human_fixture', 1.0, 'verified'),
          ('content-dispatch', 'case-vehicle-stop-001', 'seg-dispatch', 'document_assertion',
           'Officer reported a traffic stop at 22:13:51; no reason for stop was entered until 22:17:09.',
           'person-chen', NULL, NULL, 'CAD server time', '2026-01-08T22:13:51Z',
           '2026-01-08T22:13:51Z', '2026-01-08T22:17:09Z', 'CAD server clock',
           'Oak St', 'human_fixture', 1.0, 'verified'),
          ('content-passenger-first', 'case-vehicle-stop-001', 'seg-report-p4', 'statement',
           'Patel said the handgun belonged to Rivera.', 'person-chen', 'person-patel',
           NULL, NULL, '2026-01-08T22:31:00', NULL, NULL, 'report narrative',
           '400 block of Oak Street', 'human_fixture', 1.0, 'reviewed'),
          ('content-passenger-later', 'case-vehicle-stop-001', 'seg-supp-p2', 'statement',
           'Patel said they did not see Rivera with a handgun that evening.', 'person-chen', 'person-patel',
           NULL, NULL, '2026-01-15T10:20:00', NULL, NULL, 'supplemental report',
           'Station interview room', 'human_fixture', 1.0, 'unreviewed'),
          ('content-backup-ref', 'case-vehicle-stop-001', 'seg-report-p4', 'evidence_reference',
           'Backup officer body-camera footage was tagged as BWC-0042-B.',
           'person-chen', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL,
           'human_fixture', 1.0, 'verified');

        INSERT INTO propositions (id, case_id, text, status, review_state, created_by) VALUES
          ('prop-possession', 'case-vehicle-stop-001', 'Rivera knowingly possessed the handgun at approximately 22:14.', 'contested', 'reviewed', 'fixture-attorney'),
          ('prop-consent', 'case-vehicle-stop-001', 'Rivera voluntarily consented to the vehicle search.', 'contested', 'reviewed', 'fixture-attorney'),
          ('prop-stop-reason', 'case-vehicle-stop-001', 'Chen had an articulable basis for the traffic stop before initiating it.', 'contested', 'reviewed', 'fixture-attorney');

        INSERT INTO edges
          (id, case_id, source_kind, source_id, relation, target_kind, target_id, rationale, review_state, created_by)
        VALUES
          ('edge-report-consent', 'case-vehicle-stop-001', 'content', 'content-report-consent', 'supports', 'proposition', 'prop-consent', 'Report expressly characterizes the response as consent.', 'verified', 'fixture-attorney'),
          ('edge-answer-consent', 'case-vehicle-stop-001', 'content', 'content-client-answer', 'contradicts', 'proposition', 'prop-consent', 'Recorded words may be acquiescence rather than voluntary permission.', 'reviewed', 'fixture-attorney'),
          ('edge-gap-consent', 'case-vehicle-stop-001', 'content', 'content-gap', 'qualifies', 'proposition', 'prop-consent', 'Recording ends before the material search sequence can be reviewed.', 'verified', 'fixture-attorney'),
          ('edge-patel-first', 'case-vehicle-stop-001', 'content', 'content-passenger-first', 'supports', 'proposition', 'prop-possession', 'Attributed statement in initial report.', 'reviewed', 'fixture-attorney'),
          ('edge-patel-later', 'case-vehicle-stop-001', 'content', 'content-passenger-later', 'contradicts', 'proposition', 'prop-possession', 'Later attributed statement disclaims observation.', 'suggested', 'fixture-attorney'),
          ('edge-patel-impeach', 'case-vehicle-stop-001', 'content', 'content-passenger-later', 'impeaches', 'content', 'content-passenger-first', 'The two attributed accounts differ materially.', 'suggested', 'fixture-attorney'),
          ('edge-dispatch-stop', 'case-vehicle-stop-001', 'content', 'content-dispatch', 'qualifies', 'proposition', 'prop-stop-reason', 'CAD records the stated reason more than three minutes after initiation.', 'reviewed', 'fixture-attorney'),
          ('edge-missing', 'case-vehicle-stop-001', 'content', 'content-backup-ref', 'expected_but_missing', 'source', 'src-backup-missing', 'Referenced tag is absent from both productions.', 'verified', 'fixture-attorney'),
          ('edge-issue-consent', 'case-vehicle-stop-001', 'proposition', 'prop-consent', 'relevant_to', 'advocacy', 'issue-suppression', 'Voluntariness is a factual predicate for attorney analysis.', 'reviewed', 'fixture-attorney'),
          ('edge-issue-stop', 'case-vehicle-stop-001', 'proposition', 'prop-stop-reason', 'relevant_to', 'advocacy', 'issue-suppression', 'Basis and timing of detention require review.', 'reviewed', 'fixture-attorney'),
          ('edge-issue-task-video', 'case-vehicle-stop-001', 'advocacy', 'issue-suppression', 'requires_follow_up', 'advocacy', 'task-backup-video', 'The missing search recording is a factual predicate for the motion.', 'reviewed', 'fixture-attorney'),
          ('edge-issue-task-clock', 'case-vehicle-stop-001', 'advocacy', 'issue-suppression', 'requires_follow_up', 'advocacy', 'task-clock', 'Detention duration turns on which clock is right.', 'reviewed', 'fixture-attorney');

        INSERT INTO charges (id, case_id, label, citation) VALUES
          ('charge-possession', 'case-vehicle-stop-001', 'Unlawful possession of a firearm', 'Example Code § 10-201');
        INSERT INTO elements (id, charge_id, ordinal, text) VALUES
          ('element-object', 'charge-possession', 1, 'The recovered object was a firearm.'),
          ('element-possession', 'charge-possession', 2, 'Rivera possessed or exercised control over the firearm.'),
          ('element-knowing', 'charge-possession', 3, 'The possession was knowing.');
        INSERT INTO element_links (id, element_id, proposition_id, assessment, notes, created_by) VALUES
          ('elink-possession', 'element-possession', 'prop-possession', 'uncertain', 'Conflicting attributed Patel accounts; recovery sequence is not produced.', 'fixture-attorney'),
          ('elink-knowing', 'element-knowing', 'prop-possession', 'uncertain', 'Same disputed inference bears on knowledge.', 'fixture-attorney');

        INSERT INTO events
          (id, case_id, label, lane, raw_time, normalized_start, normalized_end, time_basis, location_text, proposition_id, review_state)
        VALUES
          ('event-cad-stop', 'case-vehicle-stop-001', 'Traffic stop opened in CAD', 'recorded', 'CAD 22:13:51', '2026-01-08T22:13:51Z', NULL, 'CAD server time', 'Oak Street', 'prop-stop-reason', 'verified'),
          ('event-report-consent', 'case-vehicle-stop-001', 'Officer narrative: verbal consent', 'police_narrative', 'approximately 22:14', '2026-01-08T22:14:00Z', NULL, 'report estimate', '400 block of Oak Street', 'prop-consent', 'reviewed'),
          ('event-recorded-answer', 'case-vehicle-stop-001', 'Rivera: “I guess I cannot stop you”', 'client_account', 'BWC 22:18:14', '2026-01-08T22:14:12Z', NULL, 'proposed BWC correction', '400 block of Oak Street', 'prop-consent', 'verified'),
          ('event-search-gap', 'case-vehicle-stop-001', 'Unrecorded portion of search', 'recorded', 'BWC 22:18:33–22:19:41', '2026-01-08T22:14:31Z', '2026-01-08T22:15:39Z', 'proposed BWC correction', '400 block of Oak Street', NULL, 'verified'),
          ('event-possession-hypothesis', 'case-vehicle-stop-001', 'Prosecution possession hypothesis', 'attorney_hypothesis', 'approximately 22:14', '2026-01-08T22:14:00Z', '2026-01-08T22:16:00Z', 'bounded from competing narratives', 'vehicle', 'prop-possession', 'reviewed');

        INSERT INTO advocacy_items
          (id, case_id, kind, title, body, status, author)
        VALUES
          ('issue-suppression', 'case-vehicle-stop-001', 'motion_issue', 'Potential vehicle-search suppression issue',
           'Evaluate the inception and duration of detention, the exact exchange characterized as consent, voluntariness, and the missing search recording. No legal conclusion has been entered.',
           'open', 'fixture-attorney'),
          ('task-backup-video', 'case-vehicle-stop-001', 'investigation_task', 'Request backup BWC-0042-B',
           'Confirm whether it exists, seek native file and metadata, and preserve the tag history.', 'open', 'fixture-attorney'),
          ('task-clock', 'case-vehicle-stop-001', 'investigation_task', 'Verify body-camera clock offset',
           'Compare dispatch tone, CAD server time, and original container metadata.', 'open', 'fixture-attorney'),
          ('cross-patel', 'case-vehicle-stop-001', 'cross_examination_point', 'Patel account changed',
           'Verify both attributions and obtain any underlying interview recording before use.', 'open', 'fixture-attorney');

        INSERT INTO decision_briefs
          (id, case_id, posture, summary, strengths, risks, unresolved_questions, client_topics, version, author)
        VALUES
          ('brief-motions-v1', 'case-vehicle-stop-001', 'motions',
           'The search sequence should be evaluated for a suppression motion; the fixture does not encode a legal conclusion.',
           'Recorded response differs from the report characterization, the search recording is incomplete, and CAD timing raises a detention question.',
           'The report asserts consent and the currently produced record may omit context favorable to the prosecution.',
           'What did the missing backup camera record? Why was the stop reason entered later? Is the proposed four-minute clock correction sound?',
           'Review the recorded exchange with Rivera; ask what occurred during the gap and whether any permission was given off camera.',
           1, 'fixture-attorney');

        UPDATE content SET content_created_at = '2026-01-08T22:31:00Z'
          WHERE id = 'content-passenger-first';
        UPDATE content SET content_created_at = '2026-01-15T10:20:00Z'
          WHERE id = 'content-passenger-later';
        ",
    )
    .map_err(|error| Error::InvalidFixture(error.to_string()))?;

    let source_count: u32 = transaction.query_row(
        "SELECT count(*) FROM sources WHERE case_id = ?1",
        params!["case-vehicle-stop-001"],
        |row| row.get(0),
    )?;
    if source_count < 5 {
        return Err(Error::InvalidFixture(
            "vehicle-stop fixture lost required discovery states".to_owned(),
        ));
    }
    Ok(())
}
