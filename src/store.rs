//! SQLite persistence and decision-oriented queries.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{
    AuthoredCharge, AuthoredElement, AuthoredElementMapping, AuthoredLink, AuthoredProposition,
    CaseId, DecisionBrief, DiscoveryItem, ElementAssessment, ElementCoverage, ElementRow, Error,
    IssueWorkspace, NodeKind, NodeRef, NormalizedBatch, OffenseComparison, Overview,
    ProposedAdvocacyItem, ProposedAnnotation, ProposedBrief, ProposedCharge,
    ProposedElementMapping, ProposedLink, ProposedProposition, PropositionEvidence, Result,
    ReviewDecision, ReviewEvent, ReviewQueueItem, ReviewState, ReviewTarget, TimelineEntry,
    WitnessStatement, WorkProductVersion, review::transition_allowed,
};

/// A local SQLite case store.
///
/// Foreign keys are enabled for every connection. The schema uses strict tables
/// and keeps originals, factual hypotheses, and privileged work product separate.
pub struct Store {
    pub(crate) connection: Connection,
}

impl Store {
    /// Opens or creates a store and applies all embedded migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    /// Creates a temporary in-memory store, primarily for fixtures and tests.
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;",
        )?;
        connection.execute_batch(include_str!("../migrations/0001_collation.sql"))?;
        connection.execute_batch(include_str!("../migrations/0002_review.sql"))?;
        connection.execute_batch(include_str!("../migrations/0003_authoring.sql"))?;
        Self::add_column_if_missing(&connection, "element_links", "created_by", "TEXT")?;
        connection.execute_batch(include_str!("../migrations/0004_element_mapping.sql"))?;
        Self::add_column_if_missing(
            &connection,
            "advocacy_items",
            "supersedes_advocacy_id",
            "TEXT REFERENCES advocacy_items(id)",
        )?;
        connection.execute_batch(include_str!("../migrations/0005_work_product.sql"))?;
        Ok(Self { connection })
    }

    /// Adds a column only when it is absent, so migrations stay re-runnable.
    ///
    /// Every migration here executes on every open, which `CREATE ... IF NOT
    /// EXISTS` makes safe. SQLite has no such form of `ALTER TABLE ADD COLUMN`
    /// and cannot retrofit a `NOT NULL` constraint onto an existing table, so a
    /// column added this way is nullable and its guarantee is enforced forward
    /// by a trigger. Prefer a new table over this when the choice exists.
    fn add_column_if_missing(
        connection: &Connection,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<()> {
        let present = connection
            .query_row(
                "SELECT 1 FROM pragma_table_info(?1) WHERE name = ?2",
                params![table, column],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !present {
            connection.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    /// Returns all cases in stable name order.
    pub fn cases(&self) -> Result<Vec<(CaseId, String)>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name FROM cases ORDER BY name, id")?;
        let rows = statement.query_map([], |row| Ok((CaseId(row.get(0)?), row.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Atomically imports adapter-normalized sources and extracted content.
    ///
    /// Machine-generated records must enter as `suggested`; an adapter cannot
    /// confer human verification. Existing identifiers or source hashes are
    /// rejected rather than silently replacing evidence.
    pub fn import_normalized(&mut self, batch: &NormalizedBatch) -> Result<()> {
        self.require_case(&batch.case_id)?;
        validate_batch(batch)?;
        let transaction = self.connection.transaction()?;

        for source in &batch.sources {
            let production_case: Option<String> = transaction
                .query_row(
                    "SELECT case_id FROM productions WHERE id = ?1",
                    [&source.production_id],
                    |row| row.get(0),
                )
                .optional()?;
            if production_case.as_deref() != Some(batch.case_id.0.as_str()) {
                return Err(Error::InvalidFixture(format!(
                    "production `{}` does not belong to case `{}`",
                    source.production_id, batch.case_id
                )));
            }

            transaction.execute(
                "INSERT INTO sources
                   (id, case_id, production_id, logical_name, media_type, source_kind,
                    temporal_relation, sha256, byte_length)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    source.id,
                    batch.case_id.0,
                    source.production_id,
                    source.logical_name,
                    source.media_type,
                    source.source_kind.as_str(),
                    source.temporal_relation.as_str(),
                    source.sha256,
                    to_sql_integer(source.byte_length, "source byte length")?
                ],
            )?;

            for segment in &source.segments {
                let bounding_box = segment
                    .bounding_box
                    .map(|value| serde_json::to_string(&value))
                    .transpose()?;
                transaction.execute(
                    "INSERT INTO source_segments
                       (id, source_id, locator, page, start_ms, end_ms, bbox_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        segment.id,
                        source.id,
                        segment.locator,
                        segment.page,
                        segment
                            .start_ms
                            .map(|value| to_sql_integer(value, "segment start"))
                            .transpose()?,
                        segment
                            .end_ms
                            .map(|value| to_sql_integer(value, "segment end"))
                            .transpose()?,
                        bounding_box
                    ],
                )?;

                for content in &segment.content {
                    transaction.execute(
                        "INSERT INTO content
                           (id, case_id, segment_id, kind, text, speaker_entity_id,
                            attributed_to_entity_id, parent_content_id, raw_time,
                            content_created_at, asserted_time, normalized_start, normalized_end, time_basis,
                            location_text, extractor, extractor_version, machine_generated,
                            extractor_confidence, review_state)
                         VALUES
                           (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                            ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
                        params![
                            content.id,
                            batch.case_id.0,
                            segment.id,
                            content.kind.as_str(),
                            content.text,
                            content.speaker_entity_id,
                            content.attributed_to_entity_id,
                            content.parent_content_id,
                            content.raw_time,
                            content.content_created_at,
                            content.asserted_time,
                            content.normalized_start,
                            content.normalized_end,
                            content.time_basis,
                            content.location_text,
                            content.extraction.extractor,
                            content.extraction.version,
                            content.extraction.machine_generated,
                            content.extraction.confidence,
                            content.extraction.review_state.as_str(),
                        ],
                    )?;
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Returns a non-evaluative case overview.
    pub fn overview(&self, case_id: &CaseId) -> Result<Overview> {
        let name = self.case_name(case_id)?;
        let count = |sql: &str| -> Result<u32> {
            self.connection
                .query_row(sql, [&case_id.0], |row| row.get(0))
                .map_err(Into::into)
        };
        Ok(Overview {
            case_id: case_id.0.clone(),
            case_name: name,
            productions: count("SELECT count(*) FROM productions WHERE case_id = ?1")?,
            sources: count("SELECT count(*) FROM sources WHERE case_id = ?1")?,
            unreviewed_sources: count(
                "SELECT count(*) FROM sources WHERE case_id = ?1 AND review_state = 'unreviewed'",
            )?,
            missing_references: count(
                "SELECT count(*) FROM content
                 WHERE case_id = ?1 AND kind = 'evidence_reference'
                   AND id IN (
                     SELECT source_id FROM edges
                     WHERE case_id = ?1 AND source_kind = 'content'
                       AND relation = 'expected_but_missing'
                   )",
            )?,
            propositions: count("SELECT count(*) FROM propositions WHERE case_id = ?1")?,
            pending_review: count(
                "SELECT count(*) FROM (
                   SELECT id FROM content WHERE case_id = ?1
                     AND review_state IN ('unreviewed','suggested')
                   UNION ALL
                   SELECT id FROM sources WHERE case_id = ?1
                     AND review_state IN ('unreviewed','suggested')
                   UNION ALL
                   SELECT id FROM edges WHERE case_id = ?1
                     AND review_state IN ('unreviewed','suggested')
                   UNION ALL
                   SELECT id FROM propositions WHERE case_id = ?1
                     AND review_state IN ('unreviewed','suggested')
                   UNION ALL
                   SELECT id FROM events WHERE case_id = ?1
                     AND review_state IN ('unreviewed','suggested')
                 )",
            )?,
            open_advocacy_items: count(
                "SELECT count(*) FROM advocacy_items
                 WHERE case_id = ?1 AND status NOT IN ('complete', 'closed')",
            )?,
        })
    }

    /// Builds the discovery ledger, including referenced-but-missing evidence.
    pub fn discovery_ledger(&self, case_id: &CaseId) -> Result<Vec<DiscoveryItem>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT p.label, p.received_at, s.logical_name, s.media_type,
                    s.source_kind, s.temporal_relation, s.integrity_status,
                    s.review_state, prior.logical_name
             FROM sources s
             LEFT JOIN productions p ON p.id = s.production_id
             LEFT JOIN sources prior ON prior.id = s.supersedes_source_id
             WHERE s.case_id = ?1
             ORDER BY COALESCE(p.received_at, ''), p.label, s.logical_name",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(DiscoveryItem {
                production: row
                    .get::<_, Option<String>>(0)?
                    .unwrap_or_else(|| "Unassigned".to_owned()),
                received_at: row.get(1)?,
                source: row.get(2)?,
                media_type: row.get(3)?,
                source_kind: row.get(4)?,
                temporal_relation: row.get(5)?,
                integrity_status: row.get(6)?,
                review_state: row.get(7)?,
                supersedes: row.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Builds the charge-element matrix without reducing contested links to a score.
    pub fn element_matrix(&self, case_id: &CaseId) -> Result<Vec<ElementRow>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            // `prop.case_id` is constrained as well as `ch.case_id`: elements
            // reach propositions through a table with no case column of its
            // own, and one case's matrix must never surface another's text.
            "SELECT ch.label, ch.citation, el.ordinal, el.text,
                    link.assessment, prop.text, link.notes, link.created_by
             FROM charges ch
             JOIN elements el ON el.charge_id = ch.id
             LEFT JOIN element_links link ON link.element_id = el.id
             LEFT JOIN propositions prop
               ON prop.id = link.proposition_id AND prop.case_id = ch.case_id
             WHERE ch.case_id = ?1
             ORDER BY ch.label, el.ordinal,
               CASE link.assessment
                 WHEN 'supports' THEN 1 WHEN 'opposes' THEN 2
                 WHEN 'uncertain' THEN 3 WHEN 'excluded' THEN 4 ELSE 5 END",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(ElementRow {
                charge: row.get(0)?,
                citation: row.get(1)?,
                ordinal: row.get(2)?,
                element: row.get(3)?,
                assessment: row.get(4)?,
                proposition: row.get(5)?,
                notes: row.get(6)?,
                mapped_by: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Returns statements by or attributed to a witness, ordered by when made.
    pub fn witness_dossier(
        &self,
        case_id: &CaseId,
        entity_id: &str,
    ) -> Result<Vec<WitnessStatement>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT c.id, reporter.display_name, attributed.display_name, c.text,
                    COALESCE(c.content_created_at, c.raw_time), seg.locator, src.logical_name,
                    c.review_state
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             LEFT JOIN entities reporter ON reporter.id = c.speaker_entity_id
             LEFT JOIN entities attributed ON attributed.id = c.attributed_to_entity_id
             WHERE c.case_id = ?1 AND c.kind IN ('statement','document_assertion')
               AND (c.speaker_entity_id = ?2 OR c.attributed_to_entity_id = ?2)
             ORDER BY COALESCE(c.content_created_at, c.raw_time, ''), src.logical_name, seg.locator",
        )?;
        let base = statement
            .query_map(params![case_id.0, entity_id], |row| {
                Ok(WitnessStatement {
                    id: row.get(0)?,
                    reporting_person: row.get(1)?,
                    attributed_to: row.get(2)?,
                    text: row.get(3)?,
                    statement_time: row.get(4)?,
                    locator: row.get(5)?,
                    source: row.get(6)?,
                    review_state: row.get(7)?,
                    credibility_links: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        base.into_iter()
            .map(|mut item| {
                let mut links = self.connection.prepare(
                    "SELECT relation || ': ' || COALESCE(rationale, target_kind || ' ' || target_id)
                     FROM edges
                     WHERE case_id = ?1 AND source_kind = 'content' AND source_id = ?2
                       AND relation IN ('contradicts','corroborates','impeaches','qualifies','explains')
                     ORDER BY relation, id",
                )?;
                item.credibility_links = links
                    .query_map(params![case_id.0, item.id], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(item)
            })
            .collect()
    }

    /// Returns the contested timeline with source accounts kept in separate lanes.
    pub fn contested_timeline(&self, case_id: &CaseId) -> Result<Vec<TimelineEntry>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT ev.id, ev.lane, ev.label, ev.raw_time, ev.normalized_start,
                    ev.time_basis, ev.location_text, prop.text
             FROM events ev
             LEFT JOIN propositions prop ON prop.id = ev.proposition_id
             WHERE ev.case_id = ?1
             ORDER BY COALESCE(ev.normalized_start, ev.raw_time, ''), ev.lane, ev.id",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(TimelineEntry {
                id: row.get(0)?,
                lane: row.get(1)?,
                label: row.get(2)?,
                raw_time: row.get(3)?,
                normalized_start: row.get(4)?,
                time_basis: row.get(5)?,
                location: row.get(6)?,
                proposition: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Returns all issue workspaces and their linked factual material.
    ///
    /// Only the current version of each issue appears. A superseded reading is
    /// still readable through `advocacy_history`, but it is not a second issue.
    pub fn issue_workspaces(&self, case_id: &CaseId) -> Result<Vec<IssueWorkspace>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, title, body, status FROM advocacy_items item
             WHERE case_id = ?1 AND kind IN ('legal_issue','motion_issue')
               AND NOT EXISTS (SELECT 1 FROM advocacy_items later
                               WHERE later.supersedes_advocacy_id = item.id)
             ORDER BY title",
        )?;
        let issues = statement
            .query_map([&case_id.0], |row| {
                Ok(IssueWorkspace {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    body: row.get(2)?,
                    status: row.get(3)?,
                    linked_material: Vec::new(),
                    follow_up: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Tasks belong to the issue that raised them. Listing every open task in
        // the case under every issue told a defender reading the suppression
        // workspace to chase work that belongs to an unrelated question.
        let mut tasks = self.connection.prepare(
            "SELECT task.title || ': ' || task.body
             FROM edges link
             JOIN advocacy_items task
               ON task.id = link.target_id AND task.case_id = link.case_id
             WHERE link.case_id = ?1 AND link.relation = 'requires_follow_up'
               AND link.source_kind = 'advocacy' AND link.source_id = ?2
               AND link.target_kind = 'advocacy'
               AND task.kind = 'investigation_task'
               AND task.status NOT IN ('complete','closed')
               AND NOT EXISTS (SELECT 1 FROM advocacy_items later
                               WHERE later.supersedes_advocacy_id = task.id)
             ORDER BY task.title",
        )?;

        issues
            .into_iter()
            .map(|mut issue| {
                issue.linked_material = self.edge_descriptions(case_id, &issue.id)?;
                issue.follow_up = tasks
                    .query_map(params![case_id.0, issue.id], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok(issue)
            })
            .collect()
    }

    /// Returns the latest privileged brief for a decision posture.
    pub fn decision_brief(&self, case_id: &CaseId, posture: &str) -> Result<DecisionBrief> {
        self.require_case(case_id)?;
        self.connection
            .query_row(
                "SELECT posture, summary, strengths, risks, unresolved_questions,
                        client_topics, version, author
                 FROM decision_briefs
                 WHERE case_id = ?1 AND posture = ?2
                 ORDER BY version DESC LIMIT 1",
                params![case_id.0, posture],
                |row| {
                    Ok(DecisionBrief {
                        posture: row.get(0)?,
                        summary: row.get(1)?,
                        strengths: row.get(2)?,
                        risks: row.get(3)?,
                        unresolved_questions: row.get(4)?,
                        client_topics: row.get(5)?,
                        version: row.get(6)?,
                        author: row.get(7)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "decision brief",
                id: format!("{}:{posture}", case_id.0),
            })
    }

    /// Reconstructs the source-grounded record bearing on one proposition.
    pub fn proposition_evidence(
        &self,
        case_id: &CaseId,
        proposition_id: &str,
    ) -> Result<Vec<PropositionEvidence>> {
        self.require_case(case_id)?;
        let proposition_exists = self
            .connection
            .query_row(
                "SELECT 1 FROM propositions WHERE id = ?1 AND case_id = ?2",
                params![proposition_id, case_id.0],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !proposition_exists {
            return Err(Error::NotFound {
                kind: "proposition",
                id: proposition_id.to_owned(),
            });
        }

        let mut statement = self.connection.prepare(
            "SELECT edge.relation, content.text, source.logical_name, segment.locator,
                    COALESCE(content.content_created_at, content.raw_time),
                    content.asserted_time, content.normalized_start,
                    content.extractor, content.extractor_version, content.machine_generated,
                    content.extractor_confidence, content.review_state, edge.rationale,
                    edge.review_state
             FROM edges edge
             JOIN content ON edge.source_kind = 'content' AND content.id = edge.source_id
             JOIN source_segments segment ON segment.id = content.segment_id
             JOIN sources source ON source.id = segment.source_id
             WHERE edge.case_id = ?1 AND edge.target_kind = 'proposition'
               AND edge.target_id = ?2 AND edge.review_state != 'rejected'
             ORDER BY COALESCE(content.normalized_start, content.asserted_time, content.raw_time, ''),
                      source.logical_name, segment.locator",
        )?;
        let rows = statement.query_map(params![case_id.0, proposition_id], |row| {
            Ok(PropositionEvidence {
                relation: row.get(0)?,
                text: row.get(1)?,
                source: row.get(2)?,
                locator: row.get(3)?,
                source_time: row.get(4)?,
                asserted_time: row.get(5)?,
                normalized_start: row.get(6)?,
                extractor: row.get(7)?,
                extractor_version: row.get(8)?,
                machine_generated: row.get(9)?,
                extractor_confidence: row.get(10)?,
                review_state: row.get(11)?,
                rationale: row.get(12)?,
                relation_review_state: row.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Compares charged offenses and lesser candidates element by element.
    pub fn offense_comparison(&self, case_id: &CaseId) -> Result<Vec<OffenseComparison>> {
        self.require_case(case_id)?;
        let mut charges = self.connection.prepare(
            "SELECT id, label, citation, posture, grade
             FROM charges WHERE case_id = ?1
             ORDER BY CASE posture
               WHEN 'charged' THEN 1 WHEN 'lesser_candidate' THEN 2
               WHEN 'alternative' THEN 3 ELSE 4 END, label",
        )?;
        let base = charges
            .query_map([&case_id.0], |row| {
                Ok(OffenseComparison {
                    id: row.get(0)?,
                    charge: row.get(1)?,
                    citation: row.get(2)?,
                    posture: row.get(3)?,
                    grade: row.get(4)?,
                    elements: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        base.into_iter()
            .map(|mut charge| {
                let mut elements = self.connection.prepare(
                    "SELECT id, ordinal, text FROM elements
                     WHERE charge_id = ?1 ORDER BY ordinal",
                )?;
                let element_rows = elements
                    .query_map([&charge.id], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            ElementCoverage {
                                ordinal: row.get(1)?,
                                element: row.get(2)?,
                                supporting: Vec::new(),
                                opposing: Vec::new(),
                                uncertain: Vec::new(),
                                excluded: Vec::new(),
                            },
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;

                charge.elements = element_rows
                    .into_iter()
                    .map(|(element_id, mut coverage)| {
                        let mut links = self.connection.prepare(
                            "SELECT link.assessment, prop.text
                             FROM element_links link
                             JOIN propositions prop ON prop.id = link.proposition_id
                             WHERE link.element_id = ?1 AND prop.case_id = ?2
                             ORDER BY link.assessment, prop.text",
                        )?;
                        let assessments = links
                            .query_map(params![element_id, case_id.0], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })?
                            .collect::<std::result::Result<Vec<_>, _>>()?;
                        for (assessment, proposition) in assessments {
                            let direction =
                                ElementAssessment::from_db(&assessment).ok_or_else(|| {
                                    Error::InvalidAuthoring(format!(
                                        "element `{element_id}` holds unrecognized \
                                         assessment `{assessment}`"
                                    ))
                                })?;
                            match direction {
                                ElementAssessment::Supports => &mut coverage.supporting,
                                ElementAssessment::Opposes => &mut coverage.opposing,
                                ElementAssessment::Uncertain => &mut coverage.uncertain,
                                ElementAssessment::Excluded => &mut coverage.excluded,
                            }
                            .push(proposition);
                        }
                        Ok(coverage)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(charge)
            })
            .collect()
    }

    /// Lists records still in an intake state, machine suggestions first.
    ///
    /// This is the work queue implied by the rule that only a person can move
    /// an item out of `unreviewed` or `suggested`. Items carry the exact
    /// original locator so the reviewer can open the source in one action.
    pub fn review_queue(&self, case_id: &CaseId) -> Result<Vec<ReviewQueueItem>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT 'content', c.id, c.review_state, c.machine_generated, c.text,
                    src.logical_name || ' @ ' || seg.locator, c.extractor
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.case_id = ?1 AND c.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'source', s.id, s.review_state, 0, s.logical_name,
                    s.logical_name || ' @ sha256:' || s.sha256, NULL
             FROM sources s
             WHERE s.case_id = ?1 AND s.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'edge', e.id, e.review_state, 0,
                    e.source_kind || ' ' || e.source_id || ' ' || e.relation || ' '
                      || e.target_kind || ' ' || e.target_id,
                    NULL, NULL
             FROM edges e
             WHERE e.case_id = ?1 AND e.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'proposition', p.id, p.review_state, 0, p.text, NULL, NULL
             FROM propositions p
             WHERE p.case_id = ?1 AND p.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'event', ev.id, ev.review_state, 0, ev.label, NULL, NULL
             FROM events ev
             WHERE ev.case_id = ?1 AND ev.review_state IN ('unreviewed','suggested')
             ORDER BY 4 DESC, 1, 2",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(ReviewQueueItem {
                target_kind: row.get(0)?,
                target_id: row.get(1)?,
                review_state: row.get(2)?,
                machine_generated: row.get(3)?,
                summary: row.get(4)?,
                locator: row.get(5)?,
                extractor: row.get(6)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Records one human review decision and updates the record's cached state.
    ///
    /// The decision and the state change are written in a single transaction,
    /// so the audit trail can never disagree with the record it describes. A
    /// reviewer may not move anything *into* `unreviewed` or `suggested`;
    /// those are intake states that only import produces.
    pub fn apply_review(
        &mut self,
        case_id: &CaseId,
        decision: &ReviewDecision,
    ) -> Result<ReviewEvent> {
        self.require_case(case_id)?;
        let target = decision.target;
        let actor = decision.actor.trim();
        if actor.is_empty() {
            return Err(Error::InvalidReview(
                "a decision must name the person accountable for it".to_owned(),
            ));
        }

        let from = self.review_state_of(case_id, target, &decision.target_id)?;
        let to = decision.to_state;
        if to.is_intake_state() {
            return Err(Error::InvalidTransition {
                target: target.as_str(),
                id: decision.target_id.clone(),
                from: from.as_str().to_owned(),
                to: to.as_str().to_owned(),
                reason: "intake states are produced by import, not by a reviewer",
            });
        }
        if !transition_allowed(from, to) {
            return Err(Error::InvalidTransition {
                target: target.as_str(),
                id: decision.target_id.clone(),
                from: from.as_str().to_owned(),
                to: to.as_str().to_owned(),
                reason: "the record already holds that state",
            });
        }

        let basis = decision
            .basis
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let cited = decision
            .verified_against_locator
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let locator = self.canonical_locator(target, &decision.target_id)?;
        require_justification(decision, locator.as_deref(), basis, cited)?;

        let id = Uuid::now_v7().to_string();
        let update = format!(
            "UPDATE {} SET review_state = ?1 WHERE id = ?2 AND case_id = ?3",
            target.table()
        );
        let transaction = self.connection.transaction()?;
        transaction.execute(&update, params![to.as_str(), decision.target_id, case_id.0])?;
        transaction.execute(
            "INSERT INTO review_events
               (id, case_id, target_kind, target_id, from_state, to_state,
                actor, basis, verified_against_locator)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                case_id.0,
                target.as_str(),
                decision.target_id,
                from.as_str(),
                to.as_str(),
                actor,
                basis,
                cited
            ],
        )?;
        let decided_at: String = transaction.query_row(
            "SELECT decided_at FROM review_events WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )?;
        transaction.commit()?;

        Ok(ReviewEvent {
            id,
            target_kind: target.as_str().to_owned(),
            target_id: decision.target_id.clone(),
            from_state: from.as_str().to_owned(),
            to_state: to.as_str().to_owned(),
            actor: actor.to_owned(),
            basis: basis.map(ToOwned::to_owned),
            verified_against_locator: cited.map(ToOwned::to_owned),
            decided_at,
        })
    }

    /// Writes down a contested proposition on a named person's authority.
    ///
    /// The proposition enters `contested` and `unreviewed`. Authoring is not
    /// review: a person who states a proposition has not thereby checked it,
    /// and it waits in the same queue as anything an adapter produced. Nor can
    /// authoring settle it — `undisputed` is a conclusion about the state of the
    /// evidence, and nothing in this kernel calculates one.
    pub fn author_proposition(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedProposition,
    ) -> Result<AuthoredProposition> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let text = proposal.text.trim();
        if text.is_empty() {
            return Err(Error::InvalidAuthoring(
                "a proposition must say something".to_owned(),
            ));
        }

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_id(NodeKind::Proposition, supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO propositions (id, case_id, text, status, review_state, created_by)
             VALUES (?1, ?2, ?3, 'contested', 'unreviewed', ?4)",
            params![id, case_id.0, text, author],
        )?;

        Ok(AuthoredProposition {
            id,
            text: text.to_owned(),
            status: "contested".to_owned(),
            review_state: "unreviewed".to_owned(),
            created_by: author.to_owned(),
        })
    }

    /// Asserts a typed relationship between two nodes on a named person's authority.
    ///
    /// The relationship enters `unreviewed` and carries a written rationale.
    /// That rationale is required rather than optional: an edge is drawn across
    /// several sources and has no original of its own, so it is the only thing a
    /// later reader — or the reviewer who has to verify it — can weigh. Both
    /// endpoints must already exist inside the case; an unknown identifier is
    /// refused rather than quietly creating the node it names.
    pub fn link_evidence(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedLink,
    ) -> Result<AuthoredLink> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let rationale = proposal.rationale.trim();
        if rationale.is_empty() {
            return Err(Error::InvalidAuthoring(format!(
                "asserting that {} {} {} requires a written rationale; \
                 a relationship has no original of its own to check it against",
                proposal.from,
                proposal.relation.as_str(),
                proposal.to
            )));
        }
        if proposal.from == proposal.to {
            return Err(Error::InvalidAuthoring(format!(
                "{} cannot stand in a relationship to itself",
                proposal.from
            )));
        }

        for endpoint in [&proposal.from, &proposal.to] {
            self.require_node(case_id, endpoint)?;
        }

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_id(NodeKind::Edge, supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        let written = self.connection.execute(
            "INSERT INTO edges
               (id, case_id, source_kind, source_id, relation, target_kind, target_id,
                rationale, review_state, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'unreviewed', ?9)
             ON CONFLICT (case_id, source_kind, source_id, relation, target_kind, target_id)
               DO NOTHING",
            params![
                id,
                case_id.0,
                proposal.from.kind.as_str(),
                proposal.from.id,
                proposal.relation.as_str(),
                proposal.to.kind.as_str(),
                proposal.to.id,
                rationale,
                author
            ],
        )?;
        if written == 0 {
            return Err(Error::AlreadyExists {
                kind: "relationship",
                id: format!(
                    "{} {} {}",
                    proposal.from.id,
                    proposal.relation.as_str(),
                    proposal.to.id
                ),
            });
        }

        Ok(AuthoredLink {
            id,
            from_kind: proposal.from.kind.as_str().to_owned(),
            from_id: proposal.from.id.clone(),
            relation: proposal.relation.as_str().to_owned(),
            to_kind: proposal.to.kind.as_str().to_owned(),
            to_id: proposal.to.id.clone(),
            rationale: rationale.to_owned(),
            review_state: "unreviewed".to_owned(),
            created_by: author.to_owned(),
        })
    }

    /// Writes a privileged work-product item at version one.
    ///
    /// Work product is privileged by default and stays out of the discovery
    /// ledger and any routine export. It carries no review state: review is a
    /// claim about whether an extraction faithfully represents an original, and
    /// an attorney's own analysis is not an extraction of anything.
    pub fn author_advocacy_item(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedAdvocacyItem,
    ) -> Result<WorkProductVersion> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let title = require_text(&proposal.title, "a work-product item must have a title")?;
        let body = require_text(&proposal.body, "a work-product item must say something")?;
        let status = trimmed(proposal.status.as_deref()).unwrap_or_else(|| "open".to_owned());

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("advocacy item", "advocacy_items", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO advocacy_items
               (id, case_id, kind, title, body, status, privileged, version, author)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, 1, ?7)",
            params![
                id,
                case_id.0,
                proposal.kind.as_str(),
                title,
                body,
                status,
                author
            ],
        )?;
        self.advocacy_version(case_id, &id)
    }

    /// Replaces a work-product item with a new version, keeping the old one.
    ///
    /// Nothing is overwritten. An attorney's earlier reading of an issue is not
    /// a mistake to be erased: it is what they thought when they made a
    /// decision, and a later reader — including the same attorney — has to be
    /// able to see that it changed. Only the current version may be revised; a
    /// superseded one names its replacement rather than forking the history.
    pub fn revise_advocacy_item(
        &mut self,
        case_id: &CaseId,
        item_id: &str,
        proposal: &ProposedAdvocacyItem,
    ) -> Result<WorkProductVersion> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let title = require_text(&proposal.title, "a work-product item must have a title")?;
        let body = require_text(&proposal.body, "a work-product item must say something")?;

        let previous = self.advocacy_version(case_id, item_id)?;
        if !previous.current {
            return Err(Error::Superseded {
                kind: "advocacy item",
                id: item_id.to_owned(),
                by: self.superseding_id("advocacy_items", "supersedes_advocacy_id", item_id)?,
            });
        }
        let status = trimmed(proposal.status.as_deref()).unwrap_or(previous.status);

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("advocacy item", "advocacy_items", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO advocacy_items
               (id, case_id, kind, title, body, status, privileged, version, author,
                supersedes_advocacy_id)
             VALUES (?1, ?2,
                     (SELECT kind FROM advocacy_items WHERE id = ?7),
                     ?3, ?4, ?5, 1, ?6, ?8, ?7)",
            params![
                id,
                case_id.0,
                title,
                body,
                status,
                previous.version + 1,
                item_id,
                author
            ],
        )?;
        self.advocacy_version(case_id, &id)
    }

    /// Returns every version of a work-product item, oldest first.
    ///
    /// The chain is walked from the requested version in both directions, so any
    /// version identifier returns the whole history rather than a suffix of it.
    pub fn advocacy_history(
        &self,
        case_id: &CaseId,
        item_id: &str,
    ) -> Result<Vec<WorkProductVersion>> {
        self.require_case(case_id)?;
        self.advocacy_version(case_id, item_id)?;
        let mut root = item_id.to_owned();
        while let Some(earlier) = self
            .connection
            .query_row(
                "SELECT supersedes_advocacy_id FROM advocacy_items WHERE id = ?1",
                [&root],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten()
        {
            root = earlier;
        }

        let mut history = vec![self.advocacy_version(case_id, &root)?];
        while let Some(later) =
            self.superseding_id("advocacy_items", "supersedes_advocacy_id", &root)?
        {
            history.push(self.advocacy_version(case_id, &later)?);
            root = later;
        }
        Ok(history)
    }

    /// Attaches a privileged note to one record.
    pub fn annotate(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedAnnotation,
    ) -> Result<WorkProductVersion> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let body = require_text(&proposal.body, "an annotation must say something")?;
        self.require_node(case_id, &proposal.target)?;

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("annotation", "annotations", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO annotations
               (id, case_id, target_kind, target_id, body, version, author, privileged)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, 1)",
            params![
                id,
                case_id.0,
                proposal.target.kind.as_str(),
                proposal.target.id,
                body,
                author
            ],
        )?;
        self.annotation_version(case_id, &id)
    }

    /// Replaces an annotation with a new version, keeping the old one.
    pub fn revise_annotation(
        &mut self,
        case_id: &CaseId,
        annotation_id: &str,
        proposal: &ProposedAnnotation,
    ) -> Result<WorkProductVersion> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let body = require_text(&proposal.body, "an annotation must say something")?;

        let previous = self.annotation_version(case_id, annotation_id)?;
        if !previous.current {
            return Err(Error::Superseded {
                kind: "annotation",
                id: annotation_id.to_owned(),
                by: self.superseding_id(
                    "annotations",
                    "supersedes_annotation_id",
                    annotation_id,
                )?,
            });
        }

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("annotation", "annotations", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO annotations
               (id, case_id, target_kind, target_id, body, version, author, privileged,
                supersedes_annotation_id)
             VALUES (?1, ?2,
                     (SELECT target_kind FROM annotations WHERE id = ?6),
                     (SELECT target_id FROM annotations WHERE id = ?6),
                     ?3, ?4, ?5, 1, ?6)",
            params![
                id,
                case_id.0,
                body,
                previous.version + 1,
                author,
                annotation_id
            ],
        )?;
        self.annotation_version(case_id, &id)
    }

    /// Returns the current annotations attached to one record, oldest first.
    ///
    /// Superseded versions are omitted: they remain in the database and remain
    /// reachable, but a note that has been rewritten is not a second note.
    pub fn annotations(
        &self,
        case_id: &CaseId,
        target: &NodeRef,
    ) -> Result<Vec<WorkProductVersion>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, version, supersedes_annotation_id, body, author, privileged, created_at
             FROM annotations current
             WHERE case_id = ?1 AND target_kind = ?2 AND target_id = ?3
               AND NOT EXISTS (
                 SELECT 1 FROM annotations later
                 WHERE later.supersedes_annotation_id = current.id)
             ORDER BY created_at, rowid",
        )?;
        let rows =
            statement.query_map(params![case_id.0, target.kind.as_str(), target.id], |row| {
                Ok(WorkProductVersion {
                    id: row.get(0)?,
                    version: row.get(1)?,
                    supersedes: row.get(2)?,
                    current: true,
                    title: format!("annotation on {target}"),
                    body: row.get(3)?,
                    status: "open".to_owned(),
                    privileged: row.get(5)?,
                    author: row.get(4)?,
                    created_at: row.get(6)?,
                })
            })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Writes the next version of the decision brief for a posture.
    ///
    /// A brief is advice as of a moment. Replacing one in place would destroy
    /// the record of what the client was told and when, so each is written as
    /// the next version and the earlier ones stay readable.
    pub fn record_brief(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedBrief,
    ) -> Result<WorkProductVersion> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;
        let summary = require_text(&proposal.summary, "a brief must say something")?;

        let next: u32 = self.connection.query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM decision_briefs
             WHERE case_id = ?1 AND posture = ?2",
            params![case_id.0, proposal.posture],
            |row| row.get(0),
        )?;

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("decision brief", "decision_briefs", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        self.connection.execute(
            "INSERT INTO decision_briefs
               (id, case_id, posture, summary, strengths, risks, unresolved_questions,
                client_topics, version, author, privileged)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1)",
            params![
                id,
                case_id.0,
                proposal.posture,
                summary,
                proposal.strengths.trim(),
                proposal.risks.trim(),
                proposal.unresolved_questions.trim(),
                proposal.client_topics.trim(),
                next,
                author
            ],
        )?;

        Ok(WorkProductVersion {
            id,
            version: next,
            supersedes: None,
            current: true,
            title: proposal.posture.clone(),
            body: summary,
            status: "current".to_owned(),
            privileged: true,
            author: author.to_owned(),
            created_at: String::new(),
        })
    }

    /// Reads one version of a work-product item and whether it is the current one.
    fn advocacy_version(&self, case_id: &CaseId, id: &str) -> Result<WorkProductVersion> {
        self.connection
            .query_row(
                "SELECT id, version, supersedes_advocacy_id, title, body, status,
                        privileged, author, created_at,
                        NOT EXISTS (SELECT 1 FROM advocacy_items later
                                    WHERE later.supersedes_advocacy_id = item.id)
                 FROM advocacy_items item WHERE id = ?1 AND case_id = ?2",
                params![id, case_id.0],
                |row| {
                    Ok(WorkProductVersion {
                        id: row.get(0)?,
                        version: row.get(1)?,
                        supersedes: row.get(2)?,
                        title: row.get(3)?,
                        body: row.get(4)?,
                        status: row.get(5)?,
                        privileged: row.get(6)?,
                        author: row.get(7)?,
                        created_at: row.get(8)?,
                        current: row.get(9)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "advocacy item",
                id: id.to_owned(),
            })
    }

    /// Reads one version of an annotation and whether it is the current one.
    fn annotation_version(&self, case_id: &CaseId, id: &str) -> Result<WorkProductVersion> {
        self.connection
            .query_row(
                "SELECT id, version, supersedes_annotation_id, target_kind, target_id,
                        body, privileged, author, created_at,
                        NOT EXISTS (SELECT 1 FROM annotations later
                                    WHERE later.supersedes_annotation_id = note.id)
                 FROM annotations note WHERE id = ?1 AND case_id = ?2",
                params![id, case_id.0],
                |row| {
                    Ok(WorkProductVersion {
                        id: row.get(0)?,
                        version: row.get(1)?,
                        supersedes: row.get(2)?,
                        title: format!(
                            "annotation on {} `{}`",
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?
                        ),
                        body: row.get(5)?,
                        status: "open".to_owned(),
                        privileged: row.get(6)?,
                        author: row.get(7)?,
                        created_at: row.get(8)?,
                        current: row.get(9)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "annotation",
                id: id.to_owned(),
            })
    }

    /// Returns the identifier of the version replacing this one, if any.
    fn superseding_id(&self, table: &str, column: &str, id: &str) -> Result<Option<String>> {
        let sql = format!("SELECT id FROM {table} WHERE {column} = ?1");
        self.connection
            .query_row(&sql, [id], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    /// Records a charge and its statutory elements in statutory order.
    ///
    /// A charge with no elements cannot be reasoned about — the element matrix,
    /// the offense comparison, and every question a defender asks of a charge
    /// are element-by-element — so at least one is required, and they are
    /// written in one transaction with it. Ordinals are assigned from the given
    /// order rather than accepted from the caller, because a statute's elements
    /// have an order and a gap in it would be a transcription error.
    pub fn record_charge(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedCharge,
    ) -> Result<AuthoredCharge> {
        self.require_case(case_id)?;
        let label = proposal.label.trim();
        if label.is_empty() {
            return Err(Error::InvalidAuthoring(
                "a charge must name an offense".to_owned(),
            ));
        }
        if proposal.elements.is_empty() {
            return Err(Error::InvalidAuthoring(format!(
                "charge `{label}` needs at least one element; a charge with none \
                 cannot be reasoned about element by element"
            )));
        }

        let charge_id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("charge", "charges", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };

        let mut elements = Vec::with_capacity(proposal.elements.len());
        for (index, element) in proposal.elements.iter().enumerate() {
            let text = element.text.trim();
            if text.is_empty() {
                return Err(Error::InvalidAuthoring(format!(
                    "element {} of charge `{label}` must say something",
                    index + 1
                )));
            }
            let id = match element.id.as_deref().map(str::trim) {
                Some(supplied) if !supplied.is_empty() => {
                    self.refuse_existing_row("element", "elements", supplied)?;
                    supplied.to_owned()
                }
                _ => Uuid::now_v7().to_string(),
            };
            let ordinal = u32::try_from(index + 1).map_err(|_| {
                Error::InvalidAuthoring("a charge cannot have that many elements".to_owned())
            })?;
            elements.push(AuthoredElement {
                id,
                ordinal,
                text: text.to_owned(),
            });
        }

        let citation = trimmed(proposal.citation.as_deref());
        let grade = trimmed(proposal.grade.as_deref());
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO charges (id, case_id, label, citation, posture, grade)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                charge_id,
                case_id.0,
                label,
                citation,
                proposal.posture.as_str(),
                grade
            ],
        )?;
        for element in &elements {
            transaction.execute(
                "INSERT INTO elements (id, charge_id, ordinal, text) VALUES (?1, ?2, ?3, ?4)",
                params![element.id, charge_id, element.ordinal, element.text],
            )?;
        }
        transaction.commit()?;

        Ok(AuthoredCharge {
            id: charge_id,
            label: label.to_owned(),
            citation,
            posture: proposal.posture.as_str().to_owned(),
            grade,
            elements,
        })
    }

    /// Records how one proposition bears on one statutory element.
    ///
    /// The assessment is a direction, not a weight, and `uncertain` is a
    /// first-class answer rather than an unfinished one. Nothing aggregates
    /// these: an element with three supporting and three opposing propositions
    /// is reported as exactly that.
    ///
    /// One proposition bears on one element in one direction, so a proposition
    /// already mapped to the element is refused rather than filed a second time
    /// under a contradictory heading.
    pub fn map_element(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedElementMapping,
    ) -> Result<AuthoredElementMapping> {
        self.require_case(case_id)?;
        let author = require_named_person(&proposal.author)?;

        // Elements are scoped to a case through their charge, not directly.
        self.connection
            .query_row(
                "SELECT 1 FROM elements el
                 JOIN charges ch ON ch.id = el.charge_id
                 WHERE el.id = ?1 AND ch.case_id = ?2",
                params![proposal.element_id, case_id.0],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "element",
                id: proposal.element_id.clone(),
            })?;
        self.require_node(
            case_id,
            &NodeRef::new(NodeKind::Proposition, &proposal.proposition_id),
        )?;

        if let Some(existing) =
            self.existing_assessment(&proposal.element_id, &proposal.proposition_id)?
        {
            return Err(Error::ElementAlreadyMapped {
                element: proposal.element_id.clone(),
                proposition: proposal.proposition_id.clone(),
                assessment: existing,
            });
        }

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("element mapping", "element_links", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };
        let notes = trimmed(proposal.notes.as_deref());

        self.connection.execute(
            "INSERT INTO element_links
               (id, element_id, proposition_id, assessment, notes, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                proposal.element_id,
                proposal.proposition_id,
                proposal.assessment.as_str(),
                notes,
                author
            ],
        )?;

        Ok(AuthoredElementMapping {
            id,
            element_id: proposal.element_id.clone(),
            proposition_id: proposal.proposition_id.clone(),
            assessment: proposal.assessment.as_str().to_owned(),
            notes,
            created_by: author.to_owned(),
        })
    }

    /// Returns the direction a proposition is already filed under, if any.
    fn existing_assessment(
        &self,
        element_id: &str,
        proposition_id: &str,
    ) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT assessment FROM element_links
                 WHERE element_id = ?1 AND proposition_id = ?2",
                params![element_id, proposition_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Refuses an identifier already in use in a table that is not a graph node.
    fn refuse_existing_row(&self, kind: &'static str, table: &str, id: &str) -> Result<()> {
        let sql = format!("SELECT 1 FROM {table} WHERE id = ?1");
        if self
            .connection
            .query_row(&sql, [id], |_| Ok(()))
            .optional()?
            .is_some()
        {
            return Err(Error::AlreadyExists {
                kind,
                id: id.to_owned(),
            });
        }
        Ok(())
    }

    /// Refuses a node identifier already in use, rather than replacing the record.
    fn refuse_existing_id(&self, kind: NodeKind, id: &str) -> Result<()> {
        self.refuse_existing_row(kind.as_str(), kind.table(), id)
    }

    /// Requires that a node exists and belongs to the case being worked on.
    fn require_node(&self, case_id: &CaseId, node: &NodeRef) -> Result<()> {
        let sql = format!(
            "SELECT 1 FROM {} WHERE id = ?1 AND case_id = ?2",
            node.kind.table()
        );
        self.connection
            .query_row(&sql, params![node.id, case_id.0], |_| Ok(()))
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: node.kind.as_str(),
                id: node.id.clone(),
            })
    }

    /// Returns the append-only review history in the order it was written.
    ///
    /// Insertion order, not the recorded timestamp, defines the sequence: two
    /// decisions can share a millisecond, and a corrected system clock must not
    /// be able to reorder what a reviewer actually did.
    pub fn review_history(
        &self,
        case_id: &CaseId,
        target_id: Option<&str>,
    ) -> Result<Vec<ReviewEvent>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, target_kind, target_id, from_state, to_state, actor,
                    basis, verified_against_locator, decided_at
             FROM review_events
             WHERE case_id = ?1 AND (?2 IS NULL OR target_id = ?2)
             ORDER BY rowid",
        )?;
        let rows = statement.query_map(params![case_id.0, target_id], |row| {
            Ok(ReviewEvent {
                id: row.get(0)?,
                target_kind: row.get(1)?,
                target_id: row.get(2)?,
                from_state: row.get(3)?,
                to_state: row.get(4)?,
                actor: row.get(5)?,
                basis: row.get(6)?,
                verified_against_locator: row.get(7)?,
                decided_at: row.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn review_state_of(
        &self,
        case_id: &CaseId,
        target: ReviewTarget,
        target_id: &str,
    ) -> Result<ReviewState> {
        let sql = format!(
            "SELECT review_state FROM {} WHERE id = ?1 AND case_id = ?2",
            target.table()
        );
        let raw: Option<String> = self
            .connection
            .query_row(&sql, params![target_id, case_id.0], |row| row.get(0))
            .optional()?;
        let raw = raw.ok_or_else(|| Error::NotFound {
            kind: "review target",
            id: target_id.to_owned(),
        })?;
        ReviewState::from_db(&raw).ok_or_else(|| {
            Error::InvalidReview(format!(
                "{} `{target_id}` holds unrecognized review state `{raw}`",
                target.as_str()
            ))
        })
    }

    /// Returns the single original locator a reviewer must open, when one exists.
    fn canonical_locator(&self, target: ReviewTarget, target_id: &str) -> Result<Option<String>> {
        let Some(sql) = locator_sql(target) else {
            return Ok(None);
        };
        self.connection
            .query_row(sql, [target_id], |row| row.get(0))
            .optional()
            .map_err(Into::into)
    }

    /// Returns the factual material bearing on one issue.
    ///
    /// Follow-up edges are excluded: they carry the issue's open tasks, which
    /// the workspace reports separately, and listing them here would show the
    /// same work twice under two headings.
    fn edge_descriptions(&self, case_id: &CaseId, issue_id: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT relation || ': ' || COALESCE(rationale, source_kind || ' ' || source_id)
             FROM edges
             WHERE case_id = ?1 AND relation <> 'requires_follow_up'
               AND ((target_kind = 'advocacy' AND target_id = ?2)
                 OR (source_kind = 'advocacy' AND source_id = ?2))
             ORDER BY relation, id",
        )?;
        statement
            .query_map(params![case_id.0, issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn require_case(&self, case_id: &CaseId) -> Result<()> {
        self.case_name(case_id).map(|_| ())
    }

    fn case_name(&self, case_id: &CaseId) -> Result<String> {
        self.connection
            .query_row(
                "SELECT name FROM cases WHERE id = ?1",
                [&case_id.0],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "case",
                id: case_id.0.clone(),
            })
    }
}

/// Returns an optional free-text field with surrounding space and blanks removed.
///
/// A field a person left blank and a field they filled with spaces mean the same
/// thing, and neither should be stored as if something had been written.
fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

/// Requires a field that was actually filled in.
fn require_text(value: &str, complaint: &'static str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(Error::InvalidAuthoring(complaint.to_owned()));
    }
    Ok(value.to_owned())
}

/// Requires that a named person stands behind a mutation.
///
/// Every record a person adds carries their name for the same reason every
/// review decision does: a later reader has to be able to ask whoever wrote it.
fn require_named_person(actor: &str) -> Result<&str> {
    let actor = actor.trim();
    if actor.is_empty() {
        return Err(Error::InvalidAuthoring(
            "authoring must name the person accountable for it".to_owned(),
        ));
    }
    Ok(actor)
}

/// Returns SQL yielding the one original a reviewer must open to verify a record.
///
/// Relationships, propositions, and events are attorney judgments spanning
/// several sources. They have no single original to check against, so verifying
/// one requires a written basis instead of a locator.
const fn locator_sql(target: ReviewTarget) -> Option<&'static str> {
    match target {
        ReviewTarget::Content => Some(
            "SELECT src.logical_name || ' @ ' || seg.locator
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.id = ?1",
        ),
        ReviewTarget::Source => {
            Some("SELECT logical_name || ' @ sha256:' || sha256 FROM sources WHERE id = ?1")
        }
        ReviewTarget::Edge | ReviewTarget::Proposition | ReviewTarget::Event => None,
    }
}

/// Refuses a decision that claims more than the reviewer has shown.
///
/// Verification asserts that a person opened the original; it must name that
/// original, and the name must be the record's own. A record that spans several
/// sources has no original to name, so citing one there is refused rather than
/// recorded: the trail must not hold a locator nobody could have opened.
/// Rejection removes evidence from view, so it always carries a reason someone
/// else can weigh later.
fn require_justification(
    decision: &ReviewDecision,
    locator: Option<&str>,
    basis: Option<&str>,
    cited: Option<&str>,
) -> Result<()> {
    let target = decision.target.as_str();
    match decision.to_state {
        ReviewState::Verified => match (locator, cited) {
            (Some(actual), Some(cited)) if cited != actual => Err(Error::LocatorMismatch {
                id: decision.target_id.clone(),
                cited: cited.to_owned(),
                actual: actual.to_owned(),
            }),
            (Some(actual), None) => Err(Error::InvalidReview(format!(
                "verifying `{}` requires citing the original locator `{actual}`",
                decision.target_id
            ))),
            (None, Some(cited)) => Err(Error::InvalidReview(format!(
                "{target} `{}` spans several sources and has no original to cite; \
                 `{cited}` cannot be verified against it — give a written basis instead",
                decision.target_id
            ))),
            (None, None) if basis.is_none() => Err(Error::InvalidReview(format!(
                "verifying {target} `{}` has no single original and requires a written basis",
                decision.target_id
            ))),
            _ => Ok(()),
        },
        ReviewState::Rejected if basis.is_none() => Err(Error::InvalidReview(format!(
            "rejecting {target} `{}` requires a written reason",
            decision.target_id
        ))),
        _ => Ok(()),
    }
}

fn validate_batch(batch: &NormalizedBatch) -> Result<()> {
    if batch.sources.is_empty() {
        return Err(Error::InvalidFixture(
            "a normalized batch must contain at least one source".to_owned(),
        ));
    }
    for source in &batch.sources {
        if source.sha256.len() != 64 || !source.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Error::InvalidFixture(format!(
                "source `{}` does not have a hexadecimal SHA-256",
                source.id
            )));
        }
        for segment in &source.segments {
            if segment
                .start_ms
                .zip(segment.end_ms)
                .is_some_and(|(start, end)| end < start)
            {
                return Err(Error::InvalidFixture(format!(
                    "segment `{}` ends before it starts",
                    segment.id
                )));
            }
            for content in &segment.content {
                let provenance = &content.extraction;
                if provenance.machine_generated && provenance.review_state != ReviewState::Suggested
                {
                    return Err(Error::InvalidFixture(format!(
                        "machine content `{}` must enter as suggested",
                        content.id
                    )));
                }
                if provenance
                    .confidence
                    .is_some_and(|confidence| !(0.0..=1.0).contains(&confidence))
                {
                    return Err(Error::InvalidFixture(format!(
                        "content `{}` confidence is outside 0..=1",
                        content.id
                    )));
                }
            }
        }
    }
    Ok(())
}

fn to_sql_integer(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| Error::InvalidFixture(format!("{label} exceeds SQLite integer range")))
}

/// Schema-level guarantees that no public API can reach around.
///
/// These assert what the migrations enforce rather than what Rust enforces, so
/// they need the connection itself. Every rule about *behavior* belongs in the
/// integration tests, which build a store and seed a fixture.
#[cfg(test)]
mod schema {
    use super::Store;

    fn seeded() -> Store {
        let mut store = Store::in_memory().expect("in-memory store");
        crate::DemoFixture::HitAndRun
            .seed(&mut store)
            .expect("seed hit-and-run");
        store
    }

    fn both_cases() -> Store {
        let mut store = seeded();
        crate::DemoFixture::VehicleStop
            .seed(&mut store)
            .expect("seed vehicle-stop");
        store
    }

    #[test]
    fn an_element_mapping_cannot_be_written_without_an_author() {
        let store = seeded();
        for author in ["NULL", "'   '"] {
            let error = store
                .connection
                .execute(
                    &format!(
                        "INSERT INTO element_links
                           (id, element_id, proposition_id, assessment, created_by)
                         VALUES ('anonymous', 'hr-el-fi-drive', 'hr-prop-injury',
                                 'supports', {author})"
                    ),
                    [],
                )
                .expect_err("an unattributed mapping must be refused");
            assert!(
                error.to_string().contains("must name the person"),
                "{error}"
            );
        }
    }

    /// Work product is analysis somebody is accountable for, and the schema says
    /// so rather than trusting every future writer to remember.
    #[test]
    fn work_product_cannot_be_written_without_an_author() {
        let store = seeded();
        let cases = [
            ("advocacy_items",
             "INSERT INTO advocacy_items (id, case_id, kind, title, body, author)
              VALUES ('anon', 'case-hit-run-001', 'legal_issue', 't', 'b', '  ')"),
            ("annotations",
             "INSERT INTO annotations (id, case_id, target_kind, target_id, body, version, author)
              VALUES ('anon', 'case-hit-run-001', 'content', 'hr-content-911-injury', 'b', 1, '')"),
            ("decision_briefs",
             "INSERT INTO decision_briefs
                (id, case_id, posture, summary, strengths, risks, unresolved_questions,
                 client_topics, author)
              VALUES ('anon', 'case-hit-run-001', 'trial', 's', '', '', '', '', '   ')"),
        ];
        for (table, sql) in cases {
            let error = store
                .connection
                .execute(sql, [])
                .expect_err("unattributed work product must be refused");
            assert!(
                error
                    .to_string()
                    .contains("must name the person writing it"),
                "{table}: {error}"
            );
        }
    }

    /// The store refuses to write one, but the view must not depend on that: a
    /// row reaching another case's proposition is exactly the kind of thing a
    /// future writer, an import, or a hand-edited database could introduce, and
    /// it would put privileged material from one case into another's matrix.
    #[test]
    fn a_cross_case_element_mapping_never_surfaces_in_a_view() {
        let store = both_cases();
        store
            .connection
            .execute(
                "INSERT INTO element_links
                   (id, element_id, proposition_id, assessment, created_by)
                 VALUES ('smuggled', 'hr-el-fi-drive', 'prop-consent', 'supports', 'nobody')",
                [],
            )
            .expect("the schema alone does not stop this");

        let hit_run = crate::CaseId("case-hit-run-001".to_owned());
        let leaked = store
            .connection
            .query_row(
                "SELECT text FROM propositions WHERE id = 'prop-consent'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("the other case's proposition");

        assert!(
            store
                .element_matrix(&hit_run)
                .expect("matrix")
                .iter()
                .all(|row| row.proposition.as_deref() != Some(leaked.as_str())),
            "the element matrix must not surface another case's proposition"
        );
        assert!(
            store
                .offense_comparison(&hit_run)
                .expect("offenses")
                .iter()
                .flat_map(|charge| &charge.elements)
                .flat_map(|element| {
                    element
                        .supporting
                        .iter()
                        .chain(&element.opposing)
                        .chain(&element.uncertain)
                        .chain(&element.excluded)
                })
                .all(|proposition| proposition != &leaked),
            "the offense comparison must not surface another case's proposition"
        );
    }

    /// `propositions` and `events` are review targets, so an unrecognized state
    /// there is as unreadable as one on `content` — which the schema has always
    /// refused with a CHECK.
    #[test]
    fn a_review_target_cannot_hold_an_unrecognized_review_state() {
        let store = seeded();
        for (table, id) in [
            ("propositions", "hr-prop-collision"),
            ("events", "hr-event-video-impact"),
        ] {
            let sql = format!("UPDATE {table} SET review_state = 'looks_fine' WHERE id = ?1");
            let error = store
                .connection
                .execute(&sql, [id])
                .expect_err("an unrecognized review state must be refused");
            assert!(
                error.to_string().contains("unrecognized review state"),
                "{table}: {error}"
            );
        }
    }

    /// The same relationship asserted twice would stand in front of a reviewer
    /// as two separate claims and be counted twice in every view.
    #[test]
    fn the_same_relationship_cannot_be_asserted_twice() {
        let store = seeded();
        let error = store
            .connection
            .execute(
                "INSERT INTO edges
                   (id, case_id, source_kind, source_id, relation, target_kind,
                    target_id, rationale, created_by)
                 SELECT 'duplicate-claim', case_id, source_kind, source_id, relation,
                        target_kind, target_id, rationale, created_by
                 FROM edges WHERE id = 'hr-edge-report-collision'",
                [],
            )
            .expect_err("a duplicate relationship must be refused");
        assert!(error.to_string().contains("UNIQUE"), "{error}");
    }
}
