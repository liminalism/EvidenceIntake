//! SQLite persistence and decision-oriented queries.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::{
    AuthoredLink, AuthoredProposition, CaseId, DecisionBrief, DiscoveryItem, ElementCoverage,
    ElementRow, Error, IssueWorkspace, NodeKind, NodeRef, NormalizedBatch, OffenseComparison,
    Overview, ProposedLink, ProposedProposition, PropositionEvidence, Result, ReviewDecision,
    ReviewEvent, ReviewQueueItem, ReviewState, ReviewTarget, TimelineEntry, WitnessStatement,
    review::transition_allowed,
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
        Ok(Self { connection })
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
            "SELECT ch.label, ch.citation, el.ordinal, el.text,
                    link.assessment, prop.text, link.notes
             FROM charges ch
             JOIN elements el ON el.charge_id = ch.id
             LEFT JOIN element_links link ON link.element_id = el.id
             LEFT JOIN propositions prop ON prop.id = link.proposition_id
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
    pub fn issue_workspaces(&self, case_id: &CaseId) -> Result<Vec<IssueWorkspace>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, title, body, status FROM advocacy_items
             WHERE case_id = ?1 AND kind IN ('legal_issue','motion_issue')
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

        issues
            .into_iter()
            .map(|mut issue| {
                issue.linked_material = self.edge_descriptions(case_id, &issue.id)?;
                let mut tasks = self.connection.prepare(
                    "SELECT title || ': ' || body FROM advocacy_items
                     WHERE case_id = ?1 AND kind = 'investigation_task'
                       AND status NOT IN ('complete','closed')
                     ORDER BY title",
                )?;
                issue.follow_up = tasks
                    .query_map([&case_id.0], |row| row.get(0))?
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
                    content.extractor_confidence, content.review_state, edge.rationale
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
                             WHERE link.element_id = ?1
                             ORDER BY link.assessment, prop.text",
                        )?;
                        let assessments = links
                            .query_map([element_id], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })?
                            .collect::<std::result::Result<Vec<_>, _>>()?;
                        for (assessment, proposition) in assessments {
                            match assessment.as_str() {
                                "supports" => coverage.supporting.push(proposition),
                                "opposes" => coverage.opposing.push(proposition),
                                "uncertain" => coverage.uncertain.push(proposition),
                                "excluded" => coverage.excluded.push(proposition),
                                _ => unreachable!("assessment constrained by SQLite"),
                            }
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

    /// Refuses an identifier already in use, rather than replacing the record.
    fn refuse_existing_id(&self, kind: NodeKind, id: &str) -> Result<()> {
        let sql = format!("SELECT 1 FROM {} WHERE id = ?1", kind.table());
        let taken = self
            .connection
            .query_row(&sql, [id], |_| Ok(()))
            .optional()?
            .is_some();
        if taken {
            return Err(Error::AlreadyExists {
                kind: kind.as_str(),
                id: id.to_owned(),
            });
        }
        Ok(())
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

    fn edge_descriptions(&self, case_id: &CaseId, issue_id: &str) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT relation || ': ' || COALESCE(rationale, source_kind || ' ' || source_id)
             FROM edges
             WHERE case_id = ?1
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
