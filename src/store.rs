//! `SQLite` persistence and decision-oriented queries.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use evidence_adapter_protocol::{
    ADAPTER_PROTOCOL_VERSION, AdapterEvent, AdapterJobRequest, AdapterProfile,
    AdapterResultManifest, VideoTier,
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    AnalyzerReport, AuthoredCharge, AuthoredElement, AuthoredElementMapping, AuthoredEntity,
    AuthoredLink, AuthoredProposition, CaseExport, CaseId, CaseStanding, CaseSummary,
    ChargeStanding, CollationEntry, CollationGroup, CollationIndex, DecisionBrief, DiscoveryItem,
    EdgeKind, ElementAssessment, ElementCoverage, ElementRow, ElementStanding, Error,
    ExportAudience, ExportedProposition, ExportedWorkProduct, IndexedKeyframe, IntakeArtifact,
    IntakeJob, IntakeJobState, IssueWorkspace, KeyframeHit, KeyframeIndex, LiveDispute,
    LoadBearingSource, NewIntakeJob, NodeKind, NodeRef, NormalizedBatch, OffenseComparison,
    OpenGap, OpenedCase, OpenedProduction, Overview, PlacementGap, ProposedAdvocacyItem,
    ProposedAnnotation, ProposedBrief, ProposedCase, ProposedCharge, ProposedElementMapping,
    ProposedEntity, ProposedLink, ProposedProduction, ProposedProposition, PropositionEvidence,
    Result, ReviewDecision, ReviewEvent, ReviewQueueItem, ReviewState, ReviewTarget, SearchHit,
    SourceAnchorCoverage, SourceLocation, SuggestionKind, SuggestionRun, TimelineEntry,
    UnsupportedProposition, WitnessStatement, WorkProductVersion, review::transition_allowed,
    suggest::Finding,
};

/// Number of migrations applied by [`Store::migrate`].
///
/// Recorded in `PRAGMA user_version` so an already-current database can skip
/// re-executing several hundred lines of idempotent DDL on every open. The
/// migrations stay additive and re-runnable regardless: a database at any
/// earlier version — including one written before this stamp existed, which
/// reads as zero — runs all of them again.
const SCHEMA_VERSION: i64 = 10;

/// Distinct prepared statements kept compiled per connection.
///
/// Chosen to exceed the number of distinct queries in this module so the
/// working set never evicts itself mid-view.
const STATEMENT_CACHE_CAPACITY: usize = 64;

/// The factual material bearing on one issue.
///
/// Follow-up edges are excluded: they carry the issue's open tasks, which the
/// workspace reports separately, and listing them here would show the same work
/// twice under two headings.
const MATERIAL_FOR_ISSUE: &str =
    "SELECT relation || ': ' || COALESCE(rationale, source_kind || ' ' || source_id)
     FROM edges
     WHERE case_id = ?1 AND relation <> 'requires_follow_up'
       AND ((target_kind = 'advocacy' AND target_id = ?2)
         OR (source_kind = 'advocacy' AND source_id = ?2))
     ORDER BY relation, id";

const INTAKE_JOB_SELECT_COLUMNS: &str =
    "SELECT id, case_id, production_id, source_id, modality, profile,
            original_path, original_sha256, original_byte_length, logical_name,
            request_json, artifact_dir, state, attempt, stage,
            progress_completed, progress_total, message, error,
            created_at, started_at, finished_at
     FROM intake_jobs";

const INTAKE_JOB_SELECT_ONE: &str =
    "SELECT id, case_id, production_id, source_id, modality, profile,
            original_path, original_sha256, original_byte_length, logical_name,
            request_json, artifact_dir, state, attempt, stage,
            progress_completed, progress_total, message, error,
            created_at, started_at, finished_at
     FROM intake_jobs WHERE id = ?1";

/// A local `SQLite` case store.
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
             PRAGMA synchronous = FULL;
             PRAGMA temp_store = MEMORY;",
        )?;
        // Every read model here runs the same handful of queries repeatedly, so
        // the statement cache has to be large enough to hold all of them at
        // once; an LRU too small to fit the working set recompiles on every
        // call and costs more than no cache at all.
        connection.set_prepared_statement_cache_capacity(STATEMENT_CACHE_CAPACITY);

        let applied: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if applied < SCHEMA_VERSION {
            Self::migrate(&connection)?;
            connection.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
        }
        Ok(Self { connection })
    }

    /// Applies every migration in order.
    ///
    /// Each one is additive and re-runnable, so this is safe to execute against
    /// a database at any earlier version — including one predating the version
    /// stamp entirely, which reads as version zero and receives all of them.
    fn migrate(connection: &Connection) -> Result<()> {
        connection.execute_batch(include_str!("../migrations/0001_collation.sql"))?;
        connection.execute_batch(include_str!("../migrations/0002_review.sql"))?;
        connection.execute_batch(include_str!("../migrations/0003_authoring.sql"))?;
        Self::add_column_if_missing(connection, "element_links", "created_by", "TEXT")?;
        connection.execute_batch(include_str!("../migrations/0004_element_mapping.sql"))?;
        Self::add_column_if_missing(
            connection,
            "advocacy_items",
            "supersedes_advocacy_id",
            "TEXT REFERENCES advocacy_items(id)",
        )?;
        connection.execute_batch(include_str!("../migrations/0005_work_product.sql"))?;
        connection.execute_batch(include_str!("../migrations/0006_read_paths.sql"))?;
        connection.execute_batch(include_str!("../migrations/0007_search.sql"))?;
        connection.execute_batch(include_str!("../migrations/0008_case_isolation.sql"))?;
        connection.execute_batch(include_str!("../migrations/0009_keyframe_embeddings.sql"))?;
        connection.execute_batch(include_str!("../migrations/0010_intake_jobs.sql"))?;
        Ok(())
    }

    /// Adds a column only when it is absent, so migrations stay re-runnable.
    ///
    /// Every migration here executes on every open, which `CREATE ... IF NOT
    /// EXISTS` makes safe. `SQLite` has no such form of `ALTER TABLE ADD COLUMN`
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

    /// Returns every case on the docket, in stable name order.
    ///
    /// Each row is enough to pick a matter. It is not a reading of the case.
    pub fn cases(&self) -> Result<Vec<CaseSummary>> {
        let mut statement = self.connection.prepare(
            "SELECT c.id, c.name, c.reference, c.jurisdiction, c.created_at,
                    (SELECT count(*) FROM productions p WHERE p.case_id = c.id),
                    (SELECT count(*) FROM sources s WHERE s.case_id = c.id),
                    (SELECT count(*) FROM (
                       SELECT id FROM content
                        WHERE case_id = c.id AND review_state IN ('unreviewed','suggested')
                       UNION ALL
                       SELECT id FROM sources
                        WHERE case_id = c.id AND review_state IN ('unreviewed','suggested')
                       UNION ALL
                       SELECT id FROM edges
                        WHERE case_id = c.id AND review_state IN ('unreviewed','suggested')
                       UNION ALL
                       SELECT id FROM propositions
                        WHERE case_id = c.id AND review_state IN ('unreviewed','suggested')
                       UNION ALL
                       SELECT id FROM events
                        WHERE case_id = c.id AND review_state IN ('unreviewed','suggested')))
             FROM cases c
             ORDER BY c.name, c.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(CaseSummary {
                id: row.get(0)?,
                name: row.get(1)?,
                reference: row.get(2)?,
                jurisdiction: row.get(3)?,
                created_at: row.get(4)?,
                productions: row.get(5)?,
                sources: row.get(6)?,
                pending_review: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Opens a new case and its first production.
    ///
    /// A case starts empty of evidence. The production is the intake hook
    /// adapters already require; without it nothing can be imported. Opening a
    /// case confers no review and shares nothing with any other case.
    pub fn open_case(&mut self, proposal: &ProposedCase) -> Result<OpenedCase> {
        let name = require_text(&proposal.name, "a case must have a name")?;
        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("case", "cases", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };
        let reference = trimmed(proposal.reference.as_deref());
        let jurisdiction = trimmed(proposal.jurisdiction.as_deref());
        let production_label = proposal
            .production
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Initial production");

        let production_id = Uuid::now_v7().to_string();
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO cases (id, name, reference, jurisdiction)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, name, reference, jurisdiction],
        )?;
        transaction.execute(
            "INSERT INTO productions (id, case_id, label, received_at, producing_party, notes)
             VALUES (?1, ?2, ?3, NULL, NULL, NULL)",
            params![production_id, id, production_label],
        )?;
        transaction.commit()?;

        Ok(OpenedCase {
            production: OpenedProduction {
                id: production_id,
                case_id: id.clone(),
                label: production_label.to_owned(),
                received_at: None,
                producing_party: None,
                notes: None,
            },
            id,
            name,
            reference,
            jurisdiction,
        })
    }

    /// Opens a new production on an existing case.
    ///
    /// The label must be unique inside the case. Identifiers are globally
    /// unique, so two cases cannot share a production row.
    pub fn open_production(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedProduction,
    ) -> Result<OpenedProduction> {
        self.require_case(case_id)?;
        let label = require_text(&proposal.label, "a production must have a label")?;
        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_row("production", "productions", supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };
        let received_at = trimmed(proposal.received_at.as_deref());
        let producing_party = trimmed(proposal.producing_party.as_deref());
        let notes = trimmed(proposal.notes.as_deref());

        if self.exists(
            "SELECT 1 FROM productions WHERE case_id = ?1 AND label = ?2",
            params![case_id.0, label],
        )? {
            return Err(Error::AlreadyExists {
                kind: "production",
                id: label,
            });
        }

        self.connection.execute(
            "INSERT INTO productions
               (id, case_id, label, received_at, producing_party, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, case_id.0, label, received_at, producing_party, notes],
        )?;

        Ok(OpenedProduction {
            id,
            case_id: case_id.0.clone(),
            label,
            received_at,
            producing_party,
            notes,
        })
    }

    /// Returns the productions on one case, oldest first.
    pub fn productions(&self, case_id: &CaseId) -> Result<Vec<OpenedProduction>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT id, case_id, label, received_at, producing_party, notes
             FROM productions
             WHERE case_id = ?1
             ORDER BY COALESCE(received_at, ''), label, id",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(OpenedProduction {
                id: row.get(0)?,
                case_id: row.get(1)?,
                label: row.get(2)?,
                received_at: row.get(3)?,
                producing_party: row.get(4)?,
                notes: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Queue one validated adapter request without starting model work.
    pub fn enqueue_intake_job(&mut self, proposed: &NewIntakeJob) -> Result<IntakeJob> {
        let request: AdapterJobRequest = serde_json::from_str(&proposed.request_json)?;
        request
            .validate()
            .map_err(|error| Error::InvalidIntake(error.to_string()))?;
        verify_file_identity(
            &request.original_path,
            &request.original_sha256,
            request.original_byte_length,
        )?;
        self.require_case(&CaseId(request.case_id.clone()))?;
        if self.exists(
            "SELECT 1 FROM sources WHERE case_id = ?1 AND lower(sha256) = lower(?2)",
            params![request.case_id, request.original_sha256],
        )? {
            return Err(Error::AlreadyExists {
                kind: "source hash",
                id: request.original_sha256,
            });
        }
        if self.exists(
            "SELECT 1 FROM intake_jobs
             WHERE case_id = ?1 AND lower(json_extract(request_json, '$.original_sha256')) = lower(?2)
               AND state IN ('queued','running','importing','completed')",
            params![request.case_id, request.original_sha256],
        )? {
            return Err(Error::AlreadyExists {
                kind: "intake job for source hash",
                id: request.original_sha256,
            });
        }
        let (modality, profile) = intake_profile_labels(&request.profile);
        let original_path = request
            .original_path
            .to_str()
            .ok_or_else(|| Error::InvalidIntake("original path is not Unicode".to_owned()))?;
        let artifact_dir = request
            .artifacts_dir
            .to_str()
            .ok_or_else(|| Error::InvalidIntake("artifact path is not Unicode".to_owned()))?;
        self.connection.execute(
            "INSERT INTO intake_jobs
               (id, case_id, production_id, source_id, modality, profile,
                original_path, original_sha256, original_byte_length, logical_name,
                request_json, artifact_dir, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'queued')",
            params![
                request.job_id,
                request.case_id,
                request.production_id,
                request.source_id,
                modality,
                profile,
                original_path,
                request.original_sha256,
                to_sql_integer(request.original_byte_length, "original byte length")?,
                request.logical_name,
                proposed.request_json,
                artifact_dir,
            ],
        )?;
        self.intake_job(&request.job_id)
    }

    /// Return one intake job by identifier.
    pub fn intake_job(&self, job_id: &str) -> Result<IntakeJob> {
        self.connection
            .query_row(INTAKE_JOB_SELECT_ONE, [job_id], map_intake_job)
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "intake job",
                id: job_id.to_owned(),
            })
    }

    /// List a case's intake jobs, newest first.
    pub fn intake_jobs(&self, case_id: &CaseId) -> Result<Vec<IntakeJob>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(&format!(
            "{INTAKE_JOB_SELECT_COLUMNS} WHERE case_id = ?1 ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = statement.query_map([&case_id.0], map_intake_job)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Whether a process/import currently owns this database's GPU slot.
    pub fn has_active_intake(&self) -> Result<bool> {
        self.exists(
            "SELECT 1 FROM intake_jobs WHERE state IN ('running','importing')",
            [],
        )
    }

    /// Register a retained attempt artifact outside a successful result commit.
    ///
    /// This is primarily for failure logs, which must remain inspectable even
    /// when no valid adapter manifest was produced.
    pub fn register_intake_artifact(
        &mut self,
        job_id: &str,
        kind: &str,
        path: &Path,
        sha256: Option<&str>,
    ) -> Result<()> {
        let path = path
            .to_str()
            .ok_or_else(|| Error::InvalidIntake("artifact path is not Unicode".to_owned()))?;
        self.connection.execute(
            "INSERT OR IGNORE INTO intake_artifacts(job_id, kind, path, sha256)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                job_id,
                require_text(kind, "an intake artifact needs a kind")?,
                path,
                sha256
            ],
        )?;
        Ok(())
    }

    /// Mark unfinished processes interrupted after an application restart.
    pub fn recover_interrupted_intake_jobs(&mut self) -> Result<u64> {
        let changed = self.connection.execute(
            "UPDATE intake_jobs
             SET state = 'interrupted',
                 error = COALESCE(error, 'Application exited before the adapter reached a terminal state.'),
                 finished_at = CURRENT_TIMESTAMP
             WHERE state IN ('running','importing')",
            [],
        )?;
        Ok(changed as u64)
    }

    /// Atomically claim the oldest queued job across the open database.
    pub fn claim_next_intake_job(&mut self) -> Result<Option<IntakeJob>> {
        let transaction = self.connection.transaction()?;
        let id: Option<String> = transaction
            .query_row(
                "SELECT id FROM intake_jobs WHERE state = 'queued' ORDER BY created_at, id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            transaction.commit()?;
            return Ok(None);
        };
        let changed = transaction.execute(
            "UPDATE intake_jobs
             SET state = 'running', started_at = CURRENT_TIMESTAMP, finished_at = NULL,
                 error = NULL, stage = 'starting', progress_completed = NULL,
                 progress_total = NULL, message = 'Starting adapter'
             WHERE id = ?1 AND state = 'queued'",
            [&id],
        )?;
        transaction.commit()?;
        if changed == 0 {
            return Ok(None);
        }
        self.intake_job(&id).map(Some)
    }

    /// Persist one correlated adapter progress event.
    pub fn update_intake_progress(&mut self, event: &AdapterEvent) -> Result<()> {
        if event.schema_version != ADAPTER_PROTOCOL_VERSION {
            return Err(Error::InvalidIntake(format!(
                "progress protocol {} is incompatible",
                event.schema_version
            )));
        }
        let changed = self.connection.execute(
            "UPDATE intake_jobs
             SET stage = ?2, progress_completed = ?3, progress_total = ?4, message = ?5
             WHERE id = ?1 AND state = 'running'",
            params![
                event.job_id,
                event.stage,
                event
                    .completed
                    .map(|value| to_sql_integer(value, "intake progress"))
                    .transpose()?,
                event
                    .total
                    .map(|value| to_sql_integer(value, "intake total"))
                    .transpose()?,
                event.message,
            ],
        )?;
        if changed == 0 {
            return Err(Error::InvalidIntake(format!(
                "job `{}` is not running",
                event.job_id
            )));
        }
        Ok(())
    }

    /// Move a running job to the import boundary.
    pub fn mark_intake_importing(&mut self, job_id: &str) -> Result<()> {
        self.transition_intake_job(job_id, "running", "importing", None)
    }

    /// Validate and import one adapter result as a single database commit.
    ///
    /// Files are checked before the transaction begins. Sources, optional
    /// finder vectors, retained paths, artifacts, and the terminal job state
    /// are then written together; any failure rolls all of them back.
    pub fn commit_intake_result(
        &mut self,
        job_id: &str,
        manifest: &AdapterResultManifest,
    ) -> Result<()> {
        let job = self.intake_job(job_id)?;
        if job.state != IntakeJobState::Importing {
            return Err(Error::InvalidIntake(format!(
                "job `{job_id}` is not ready to import"
            )));
        }
        let request: AdapterJobRequest = serde_json::from_str(&job.request_json)?;
        request
            .validate()
            .map_err(|error| Error::InvalidIntake(error.to_string()))?;
        manifest
            .validate_for(&request)
            .map_err(|error| Error::InvalidIntake(error.to_string()))?;

        let batch: NormalizedBatch = serde_json::from_slice(&std::fs::read(&manifest.batch_path)?)?;
        if batch.case_id.0 != request.case_id {
            return Err(Error::InvalidIntake(format!(
                "result case `{}` does not match job case `{}`",
                batch.case_id, request.case_id
            )));
        }
        validate_batch(&batch)?;
        self.refuse_cross_case_batch(&batch)?;
        let original = batch
            .sources
            .iter()
            .find(|source| source.id == request.source_id)
            .ok_or_else(|| {
                Error::InvalidIntake(format!(
                    "result does not contain requested source `{}`",
                    request.source_id
                ))
            })?;
        if original.production_id != request.production_id
            || !original
                .sha256
                .eq_ignore_ascii_case(&request.original_sha256)
            || original.byte_length != request.original_byte_length
        {
            return Err(Error::InvalidIntake(
                "result changed the requested production or original identity".to_owned(),
            ));
        }

        let locations = manifest
            .source_locations
            .iter()
            .map(|location| (location.source_id.as_str(), location))
            .collect::<HashMap<_, _>>();
        for source in &batch.sources {
            let location = locations.get(source.id.as_str()).ok_or_else(|| {
                Error::InvalidIntake(format!(
                    "result does not retain a location for source `{}`",
                    source.id
                ))
            })?;
            if !source.sha256.eq_ignore_ascii_case(&location.sha256)
                || source.byte_length != location.byte_length
            {
                return Err(Error::InvalidIntake(format!(
                    "location identity does not match source `{}`",
                    source.id
                )));
            }
            verify_file_identity(&location.path, &location.sha256, location.byte_length)?;
        }
        let original_location = locations.get(request.source_id.as_str()).ok_or_else(|| {
            Error::InvalidIntake("requested source location is missing".to_owned())
        })?;
        if original_location.path.canonicalize()? != request.original_path.canonicalize()? {
            return Err(Error::InvalidIntake(
                "adapter changed the referenced original path".to_owned(),
            ));
        }

        for artifact in &manifest.artifacts {
            if let Some(hash) = &artifact.sha256 {
                verify_file_identity(&artifact.path, hash, artifact.path.metadata()?.len())?;
            }
        }

        let keyframes = manifest
            .keyframe_index_path
            .as_ref()
            .map(|path| -> Result<KeyframeIndex> {
                Ok(serde_json::from_slice(&std::fs::read(path)?)?)
            })
            .transpose()?;
        if let Some(index) = &keyframes {
            if index.case_id != batch.case_id {
                return Err(Error::InvalidIntake(
                    "keyframe index belongs to a different case".to_owned(),
                ));
            }
            Self::validate_keyframe_index(index)?;
        }

        let transaction = self.connection.transaction()?;
        Self::import_normalized_tx(&transaction, &batch)?;
        if let Some(index) = &keyframes {
            Self::index_keyframes_tx(&transaction, index)?;
        }
        for location in &manifest.source_locations {
            let path = location.path.to_str().ok_or_else(|| {
                Error::InvalidIntake("source location path is not Unicode".to_owned())
            })?;
            transaction.execute(
                "INSERT INTO source_locations(source_id, case_id, path, last_verified_at)
                 VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)",
                params![location.source_id, request.case_id, path],
            )?;
        }
        for artifact in &manifest.artifacts {
            let path = artifact
                .path
                .to_str()
                .ok_or_else(|| Error::InvalidIntake("artifact path is not Unicode".to_owned()))?;
            transaction.execute(
                "INSERT INTO intake_artifacts(job_id, kind, path, sha256)
                 VALUES (?1, ?2, ?3, ?4)",
                params![job_id, artifact.kind, path, artifact.sha256],
            )?;
        }
        for (kind, path) in [
            ("normalized_batch", Some(&manifest.batch_path)),
            ("keyframe_index", manifest.keyframe_index_path.as_ref()),
        ] {
            if let Some(path) = path {
                let path = path
                    .to_str()
                    .ok_or_else(|| Error::InvalidIntake("result path is not Unicode".to_owned()))?;
                transaction.execute(
                    "INSERT OR IGNORE INTO intake_artifacts(job_id, kind, path, sha256)
                     VALUES (?1, ?2, ?3, NULL)",
                    params![job_id, kind, path],
                )?;
            }
        }
        let changed = transaction.execute(
            "UPDATE intake_jobs
             SET state = 'completed', stage = 'completed', message = 'Import completed',
                 error = NULL, finished_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND state = 'importing'",
            [job_id],
        )?;
        if changed != 1 {
            return Err(Error::InvalidIntake(format!(
                "job `{job_id}` changed state while importing"
            )));
        }
        transaction.commit()?;
        Ok(())
    }

    /// Record a visible terminal adapter/import failure.
    pub fn fail_intake_job(&mut self, job_id: &str, error: &str) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE intake_jobs
             SET state = 'failed', error = ?2, message = 'Processing failed',
                 finished_at = CURRENT_TIMESTAMP
             WHERE id = ?1 AND state IN ('running','importing')",
            params![
                job_id,
                require_text(error, "an intake failure needs a diagnostic")?
            ],
        )?;
        if changed == 0 {
            return Err(Error::InvalidIntake(format!(
                "job `{job_id}` cannot fail from its current state"
            )));
        }
        Ok(())
    }

    /// Queue a new immutable attempt after failure or interruption.
    pub fn retry_intake_job(&mut self, job_id: &str, request_json: &str) -> Result<IntakeJob> {
        let request: AdapterJobRequest = serde_json::from_str(request_json)?;
        request
            .validate()
            .map_err(|error| Error::InvalidIntake(error.to_string()))?;
        if request.job_id != job_id {
            return Err(Error::InvalidIntake(
                "retry request changed the job identifier".to_owned(),
            ));
        }
        let artifact_dir = request
            .artifacts_dir
            .to_str()
            .ok_or_else(|| Error::InvalidIntake("artifact path is not Unicode".to_owned()))?;
        let changed = self.connection.execute(
            "UPDATE intake_jobs
             SET state = 'queued', attempt = attempt + 1, request_json = ?2,
                 artifact_dir = ?3, stage = NULL, progress_completed = NULL,
                 progress_total = NULL, message = 'Queued for retry', error = NULL,
                 started_at = NULL, finished_at = NULL
             WHERE id = ?1 AND state IN ('failed','interrupted')",
            params![job_id, request_json, artifact_dir],
        )?;
        if changed == 0 {
            return Err(Error::InvalidIntake(format!(
                "job `{job_id}` is not failed or interrupted"
            )));
        }
        self.intake_job(job_id)
    }

    /// Return retained artifacts for one job.
    pub fn intake_artifacts(&self, job_id: &str) -> Result<Vec<IntakeArtifact>> {
        let mut statement = self.connection.prepare(
            "SELECT job_id, kind, path, sha256 FROM intake_artifacts
             WHERE job_id = ?1 ORDER BY kind, path",
        )?;
        let rows = statement.query_map([job_id], |row| {
            Ok(IntakeArtifact {
                job_id: row.get(0)?,
                kind: row.get(1)?,
                path: std::path::PathBuf::from(row.get::<_, String>(2)?),
                sha256: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Return the current path for an imported source.
    pub fn source_location(&self, source_id: &str) -> Result<SourceLocation> {
        self.connection
            .query_row(
                "SELECT source_id, case_id, path, last_verified_at
                 FROM source_locations WHERE source_id = ?1",
                [source_id],
                |row| {
                    Ok(SourceLocation {
                        source_id: row.get(0)?,
                        case_id: CaseId(row.get(1)?),
                        path: std::path::PathBuf::from(row.get::<_, String>(2)?),
                        last_verified_at: row.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "source location",
                id: source_id.to_owned(),
            })
    }

    /// Relink a moved original only when its bytes still match the source row.
    pub fn relink_source(&mut self, source_id: &str, path: &Path) -> Result<SourceLocation> {
        let expected: Option<(String, i64, String)> = self
            .connection
            .query_row(
                "SELECT sha256, byte_length, case_id FROM sources WHERE id = ?1",
                [source_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((expected_hash, expected_length, case_id)) = expected else {
            return Err(Error::NotFound {
                kind: "source",
                id: source_id.to_owned(),
            });
        };
        let expected_length = u64::try_from(expected_length).map_err(|_| {
            Error::InvalidIntake(format!("source `{source_id}` has an invalid stored length"))
        })?;
        verify_file_identity(path, &expected_hash, expected_length)?;
        let path = path
            .to_str()
            .ok_or_else(|| Error::InvalidIntake("source path is not Unicode".to_owned()))?;
        self.connection.execute(
            "INSERT INTO source_locations(source_id, case_id, path, last_verified_at)
             VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)
             ON CONFLICT(source_id) DO UPDATE SET
                 case_id = excluded.case_id,
                 path = excluded.path,
                 last_verified_at = CURRENT_TIMESTAMP",
            params![source_id, case_id, path],
        )?;
        self.source_location(source_id)
    }

    fn transition_intake_job(
        &mut self,
        job_id: &str,
        from: &str,
        to: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE intake_jobs SET state = ?3, error = ?4 WHERE id = ?1 AND state = ?2",
            params![job_id, from, to, error],
        )?;
        if changed == 0 {
            return Err(Error::InvalidIntake(format!(
                "job `{job_id}` cannot move from `{from}` to `{to}`"
            )));
        }
        Ok(())
    }

    /// Atomically imports adapter-normalized sources and extracted content.
    ///
    /// Machine-generated records must enter as `suggested`; an adapter cannot
    /// confer human verification. Existing identifiers or source hashes are
    /// rejected rather than silently replacing evidence.
    pub fn import_normalized(&mut self, batch: &NormalizedBatch) -> Result<()> {
        self.require_case(&batch.case_id)?;
        validate_batch(batch)?;
        self.refuse_cross_case_batch(batch)?;
        let transaction = self.connection.transaction()?;

        Self::import_normalized_tx(&transaction, batch)?;
        transaction.commit()?;
        Ok(())
    }

    fn import_normalized_tx(
        transaction: &rusqlite::Transaction<'_>,
        batch: &NormalizedBatch,
    ) -> Result<()> {
        // A batch is many rows of three shapes, so each statement is compiled
        // once and reused for every row rather than once per row.
        let mut owning_production = transaction
            .prepare_cached("SELECT 1 FROM productions WHERE id = ?1 AND case_id = ?2")?;
        let mut insert_source = transaction.prepare_cached(
            "INSERT INTO sources
               (id, case_id, production_id, logical_name, media_type, source_kind,
                temporal_relation, sha256, byte_length)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        let mut insert_segment = transaction.prepare_cached(
            "INSERT INTO source_segments
               (id, source_id, locator, page, start_ms, end_ms, bbox_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        let mut insert_content = transaction.prepare_cached(
            "INSERT INTO content
               (id, case_id, segment_id, kind, text, speaker_entity_id,
                attributed_to_entity_id, parent_content_id, raw_time,
                content_created_at, asserted_time, normalized_start, normalized_end, time_basis,
                location_text, extractor, extractor_version, machine_generated,
                extractor_confidence, review_state)
             VALUES
               (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)",
        )?;

        for source in &batch.sources {
            let owned = owning_production
                .query_row(params![source.production_id, batch.case_id.0], |_| Ok(()))
                .optional()?
                .is_some();
            if !owned {
                return Err(Error::InvalidFixture(format!(
                    "production `{}` does not belong to case `{}`",
                    source.production_id, batch.case_id
                )));
            }

            insert_source.execute(params![
                source.id,
                batch.case_id.0,
                source.production_id,
                source.logical_name,
                source.media_type,
                source.source_kind.as_str(),
                source.temporal_relation.as_str(),
                source.sha256,
                to_sql_integer(source.byte_length, "source byte length")?
            ])?;

            for segment in &source.segments {
                let bounding_box = segment
                    .bounding_box
                    .map(|value| serde_json::to_string(&value))
                    .transpose()?;
                insert_segment.execute(params![
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
                ])?;

                for content in &segment.content {
                    insert_content.execute(params![
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
                    ])?;
                }
            }
        }
        drop(insert_content);
        drop(insert_segment);
        drop(insert_source);
        drop(owning_production);

        if !batch.edges.is_empty() {
            Self::import_edges(transaction, batch)?;
        }

        Ok(())
    }

    /// Writes an adapter's proposed relationships inside the import transaction.
    ///
    /// This runs after the batch's own sources and content are inserted, which
    /// is what lets a batch relate the stills it brings while an endpoint the
    /// case does not hold is still refused. `require_node` answers the same
    /// question, but it takes `&self` and the open transaction has borrowed the
    /// connection, so the lookup is issued through the transaction instead.
    fn import_edges(
        transaction: &rusqlite::Transaction<'_>,
        batch: &NormalizedBatch,
    ) -> Result<()> {
        let mut existing_edge = transaction.prepare_cached("SELECT 1 FROM edges WHERE id = ?1")?;
        // An adapter points at a *pair*. If the case already relates that pair
        // this way the claim is held however it happens to be oriented, and the
        // mirror image is not a second thing to review. The unique index sees
        // only one orientation, so the check is made here.
        let mut already_held = transaction.prepare_cached(
            "SELECT 1 FROM edges
             WHERE case_id = ?1 AND relation = ?2
               AND ((source_kind = ?3 AND source_id = ?4
                     AND target_kind = ?5 AND target_id = ?6)
                 OR (source_kind = ?5 AND source_id = ?6
                     AND target_kind = ?3 AND target_id = ?4))",
        )?;
        let mut insert_edge = transaction.prepare_cached(
            "INSERT INTO edges
               (id, case_id, source_kind, source_id, relation, target_kind, target_id,
                rationale, review_state, created_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'suggested', ?9)",
        )?;

        for edge in &batch.edges {
            if existing_edge
                .query_row([&edge.id], |_| Ok(()))
                .optional()?
                .is_some()
            {
                return Err(Error::AlreadyExists {
                    kind: NodeKind::Edge.as_str(),
                    id: edge.id.clone(),
                });
            }

            for (kind, id) in [(edge.from_kind, &edge.from_id), (edge.to_kind, &edge.to_id)] {
                let owner: Option<String> = transaction
                    .prepare_cached(&format!(
                        "SELECT case_id FROM {} WHERE id = ?1",
                        kind.table()
                    ))?
                    .query_row([id], |row| row.get(0))
                    .optional()?;
                match owner {
                    Some(owner) if owner == batch.case_id.0 => {}
                    Some(_) => {
                        return Err(Error::WrongCase {
                            kind: kind.as_str(),
                            id: id.clone(),
                        });
                    }
                    None => {
                        return Err(Error::NotFound {
                            kind: kind.as_str(),
                            id: id.clone(),
                        });
                    }
                }
            }

            let held = already_held
                .query_row(
                    params![
                        batch.case_id.0,
                        edge.relation.as_str(),
                        edge.from_kind.as_str(),
                        edge.from_id,
                        edge.to_kind.as_str(),
                        edge.to_id
                    ],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if held {
                return Err(Error::AlreadyExists {
                    kind: "relationship",
                    id: format!("{} {} {}", edge.from_id, edge.relation.as_str(), edge.to_id),
                });
            }

            // The same attribution prefix the analyzers write: that prefix is
            // how `review_queue` recognises a machine-proposed edge, and an
            // adapter's proposal is exactly that.
            insert_edge.execute(params![
                edge.id,
                batch.case_id.0,
                edge.from_kind.as_str(),
                edge.from_id,
                edge.relation.as_str(),
                edge.to_kind.as_str(),
                edge.to_id,
                edge.rationale.trim(),
                format!(
                    "suggest:{}@{}",
                    edge.extraction.extractor, edge.extraction.version
                )
            ])?;
        }
        Ok(())
    }

    /// Returns a non-evaluative case overview.
    pub fn overview(&self, case_id: &CaseId) -> Result<Overview> {
        let name = self.case_name(case_id)?;
        // One statement rather than eight. The counts are independent of each
        // other, so SQLite computes them in a single pass over the case and the
        // caller pays one round trip instead of eight.
        self.query_one(
            "SELECT
               (SELECT count(*) FROM productions WHERE case_id = ?1),
               (SELECT count(*) FROM sources WHERE case_id = ?1),
               (SELECT count(*) FROM sources
                 WHERE case_id = ?1 AND review_state = 'unreviewed'),
               (SELECT count(*) FROM content
                 WHERE case_id = ?1 AND kind = 'evidence_reference'
                   AND id IN (
                     SELECT source_id FROM edges
                     WHERE case_id = ?1 AND source_kind = 'content'
                       AND relation = 'expected_but_missing')),
               (SELECT count(*) FROM propositions WHERE case_id = ?1),
               (SELECT count(*) FROM (
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
                    AND review_state IN ('unreviewed','suggested'))),
               (SELECT count(*) FROM advocacy_items
                 WHERE case_id = ?1 AND status NOT IN ('complete', 'closed'))",
            [&case_id.0],
            |row| {
                Ok(Overview {
                    case_id: case_id.0.clone(),
                    case_name: name,
                    productions: row.get(0)?,
                    sources: row.get(1)?,
                    unreviewed_sources: row.get(2)?,
                    missing_references: row.get(3)?,
                    propositions: row.get(4)?,
                    pending_review: row.get(5)?,
                    open_advocacy_items: row.get(6)?,
                })
            },
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "case",
            id: case_id.0.clone(),
        })
    }

    /// Builds the discovery ledger, including referenced-but-missing evidence.
    pub fn discovery_ledger(&self, case_id: &CaseId) -> Result<Vec<DiscoveryItem>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(
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
        let mut statement = self.connection.prepare_cached(
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
        let mut statement = self.connection.prepare_cached(
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
             ORDER BY COALESCE(c.content_created_at, c.raw_time, ''), src.logical_name,
                      CASE WHEN seg.page IS NULL THEN 1 ELSE 0 END, seg.page,
                      CASE WHEN seg.start_ms IS NULL THEN 1 ELSE 0 END, seg.start_ms,
                      seg.locator, c.id",
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

        let mut links = self.connection.prepare_cached(
            "SELECT relation || ': ' || COALESCE(rationale, target_kind || ' ' || target_id)
             FROM edges
             WHERE case_id = ?1 AND source_kind = 'content' AND source_id = ?2
               AND relation IN ('contradicts','corroborates','impeaches','qualifies','explains')
             ORDER BY relation, id",
        )?;
        base.into_iter()
            .map(|mut item| {
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
        let mut statement = self.connection.prepare_cached(
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

    /// Collates active source-grounded passages by transparent time and location keys.
    ///
    /// This is a read model, not an analyzer: it writes no edge and makes no
    /// common-event claim. Date keys come only from `normalized_start`.
    /// Location matching folds case and repeated whitespace but performs no
    /// abbreviation, address, geospatial, or semantic inference.
    pub fn collation_index(&self, case_id: &CaseId) -> Result<CollationIndex> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT c.id, src.id, src.logical_name, src.source_kind, seg.locator, c.text,
                    c.raw_time, c.content_created_at, c.asserted_time, c.normalized_start,
                    c.normalized_end, c.time_basis, c.location_text, c.machine_generated,
                    c.extractor, c.review_state
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.case_id = ?1 AND c.review_state <> 'rejected'
             ORDER BY COALESCE(c.normalized_start, c.asserted_time, c.raw_time, ''),
                      src.logical_name,
                      CASE WHEN seg.page IS NULL THEN 1 ELSE 0 END, seg.page,
                      CASE WHEN seg.start_ms IS NULL THEN 1 ELSE 0 END, seg.start_ms,
                      seg.locator, c.id",
        )?;
        let entries = statement
            .query_map([&case_id.0], |row| {
                Ok(CollationEntry {
                    content_id: row.get(0)?,
                    source_id: row.get(1)?,
                    source: row.get(2)?,
                    source_kind: row.get(3)?,
                    locator: row.get(4)?,
                    text: row.get(5)?,
                    raw_time: row.get(6)?,
                    content_created_at: row.get(7)?,
                    asserted_time: row.get(8)?,
                    normalized_start: row.get(9)?,
                    normalized_end: row.get(10)?,
                    time_basis: row.get(11)?,
                    location: row.get(12)?,
                    machine_generated: row.get::<_, i64>(13)? != 0,
                    extractor: row.get(14)?,
                    review_state: row.get(15)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut by_date: BTreeMap<String, Vec<CollationEntry>> = BTreeMap::new();
        let mut by_location: BTreeMap<String, (String, Vec<CollationEntry>)> = BTreeMap::new();
        let mut possible: BTreeMap<(String, String), (String, Vec<CollationEntry>)> =
            BTreeMap::new();
        let mut source_coverage: BTreeMap<String, SourceAnchorCoverage> = BTreeMap::new();
        let mut needs_placement = Vec::new();
        let mut without_normalized_date = 0_u32;
        let mut without_location = 0_u32;

        for entry in entries {
            let date = entry
                .normalized_start
                .as_deref()
                .and_then(normalized_date)
                .map(str::to_owned);
            let location = entry.location.as_deref().and_then(|value| {
                let display = value.split_whitespace().collect::<Vec<_>>().join(" ");
                (!display.is_empty()).then(|| (conservative_location_key(&display), display))
            });

            let coverage = source_coverage
                .entry(entry.source_id.clone())
                .or_insert_with(|| SourceAnchorCoverage {
                    source_id: entry.source_id.clone(),
                    source: entry.source.clone(),
                    source_kind: entry.source_kind.clone(),
                    passages: 0,
                    with_raw_time: 0,
                    with_content_created_at: 0,
                    with_asserted_time: 0,
                    with_normalized_date: 0,
                    with_location: 0,
                });
            coverage.passages = coverage.passages.saturating_add(1);
            if has_text(entry.raw_time.as_deref()) {
                coverage.with_raw_time = coverage.with_raw_time.saturating_add(1);
            }
            if has_text(entry.content_created_at.as_deref()) {
                coverage.with_content_created_at =
                    coverage.with_content_created_at.saturating_add(1);
            }
            if has_text(entry.asserted_time.as_deref()) {
                coverage.with_asserted_time = coverage.with_asserted_time.saturating_add(1);
            }
            if date.is_some() {
                coverage.with_normalized_date = coverage.with_normalized_date.saturating_add(1);
            }
            if location.is_some() {
                coverage.with_location = coverage.with_location.saturating_add(1);
            }

            let mut missing_anchors = Vec::new();
            if date.is_none() {
                missing_anchors.push("normalized_date".to_owned());
            }
            if location.is_none() {
                missing_anchors.push("location".to_owned());
            }
            if !missing_anchors.is_empty() {
                needs_placement.push(PlacementGap {
                    missing_anchors,
                    entry: entry.clone(),
                });
            }

            if let Some(date) = &date {
                by_date.entry(date.clone()).or_default().push(entry.clone());
            } else {
                without_normalized_date = without_normalized_date.saturating_add(1);
            }
            if let Some((key, display)) = &location {
                by_location
                    .entry(key.clone())
                    .or_insert_with(|| (display.clone(), Vec::new()))
                    .1
                    .push(entry.clone());
            } else {
                without_location = without_location.saturating_add(1);
            }
            if let (Some(date), Some((key, display))) = (date, location) {
                possible
                    .entry((date, key))
                    .or_insert_with(|| (display, Vec::new()))
                    .1
                    .push(entry);
            }
        }

        let by_date = by_date
            .into_iter()
            .map(|(date, entries)| CollationGroup {
                distinct_sources: distinct_sources(&entries),
                rationale: format!(
                    "Grouped by normalized date `{date}` only; entries remain separate records."
                ),
                normalized_date: Some(date),
                location: None,
                entries,
            })
            .collect();
        let by_location = by_location
            .into_values()
            .map(|(location, entries)| CollationGroup {
                distinct_sources: distinct_sources(&entries),
                rationale: format!(
                    "Grouped by exact case-insensitive location text `{location}` only; no address or geospatial inference was performed."
                ),
                normalized_date: None,
                location: Some(location),
                entries,
            })
            .collect();
        let possibly_related = possible
            .into_iter()
            .filter_map(|((date, _), (location, entries))| {
                let distinct_sources = distinct_sources(&entries);
                (distinct_sources >= 2).then(|| CollationGroup {
                    normalized_date: Some(date.clone()),
                    location: Some(location.clone()),
                    distinct_sources,
                    rationale: format!(
                        "Shared collation keys only: normalized date `{date}` and exact case-insensitive location text `{location}` across {distinct_sources} immutable sources. The records remain separate; this does not assert a common event."
                    ),
                    entries,
                })
            })
            .collect();
        let mut source_coverage: Vec<_> = source_coverage.into_values().collect();
        source_coverage.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.source_id.cmp(&right.source_id))
        });

        Ok(CollationIndex {
            case_id: case_id.0.clone(),
            by_date,
            by_location,
            possibly_related,
            source_coverage,
            needs_placement,
            without_normalized_date,
            without_location,
        })
    }

    /// Returns all issue workspaces and their linked factual material.
    ///
    /// Only the current version of each issue appears. A superseded reading is
    /// still readable through `advocacy_history`, but it is not a second issue.
    pub fn issue_workspaces(&self, case_id: &CaseId) -> Result<Vec<IssueWorkspace>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(
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
        let mut tasks = self.connection.prepare_cached(
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

        let mut material = self.connection.prepare_cached(MATERIAL_FOR_ISSUE)?;

        issues
            .into_iter()
            .map(|mut issue| {
                issue.linked_material = material
                    .query_map(params![case_id.0, issue.id], |row| row.get(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
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
        if !self.exists(
            "SELECT 1 FROM propositions WHERE id = ?1 AND case_id = ?2",
            params![proposition_id, case_id.0],
        )? {
            return Err(Error::NotFound {
                kind: "proposition",
                id: proposition_id.to_owned(),
            });
        }

        let mut statement = self.connection.prepare_cached(
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
                      source.logical_name,
                      CASE WHEN segment.page IS NULL THEN 1 ELSE 0 END, segment.page,
                      CASE WHEN segment.start_ms IS NULL THEN 1 ELSE 0 END, segment.start_ms,
                      segment.locator, content.id",
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
        let mut charges = self.connection.prepare_cached(
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

        // The whole comparison is three statements: charges, then every element
        // of those charges, then every assessment on those elements. Reading it
        // charge-by-charge and element-by-element asked the same two questions
        // once per row for no additional information.
        let mut element_statement = self.connection.prepare_cached(
            "SELECT el.charge_id, el.id, el.ordinal, el.text
             FROM elements el
             JOIN charges ch ON ch.id = el.charge_id
             WHERE ch.case_id = ?1
             ORDER BY el.ordinal",
        )?;
        let mut elements_by_charge: HashMap<String, Vec<(String, ElementCoverage)>> =
            HashMap::new();
        for row in element_statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                ElementCoverage {
                    ordinal: row.get(2)?,
                    element: row.get(3)?,
                    supporting: Vec::new(),
                    opposing: Vec::new(),
                    uncertain: Vec::new(),
                    excluded: Vec::new(),
                },
            ))
        })? {
            let (charge_id, element_id, coverage) = row?;
            elements_by_charge
                .entry(charge_id)
                .or_default()
                .push((element_id, coverage));
        }

        // `prop.case_id` as well as `ch.case_id`: element mappings reach
        // propositions through a table with no case column of its own.
        let mut assessment_statement = self.connection.prepare_cached(
            "SELECT link.element_id, link.assessment, prop.text
             FROM element_links link
             JOIN elements el ON el.id = link.element_id
             JOIN charges ch ON ch.id = el.charge_id
             JOIN propositions prop ON prop.id = link.proposition_id
             WHERE ch.case_id = ?1 AND prop.case_id = ?1
             ORDER BY link.assessment, prop.text",
        )?;
        let mut assessments: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for row in assessment_statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (element_id, assessment, proposition) = row?;
            assessments
                .entry(element_id)
                .or_default()
                .push((assessment, proposition));
        }

        base.into_iter()
            .map(|mut charge| {
                let elements = elements_by_charge.remove(&charge.id).unwrap_or_default();
                charge.elements = elements
                    .into_iter()
                    .map(|(element_id, mut coverage)| {
                        for (assessment, proposition) in
                            assessments.remove(&element_id).unwrap_or_default()
                        {
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

    /// Finds excerpts matching a full-text query, best match first.
    ///
    /// Results are ordered by BM25 relevance, which ranks how well a passage
    /// matches the words asked for — not how much it is worth. That distinction
    /// is the whole reason a rank is acceptable here when a score is not
    /// acceptable anywhere: nothing about the ordering claims a passage is true,
    /// admissible, or important, only that it contains more of what was typed.
    /// The number itself is not reported, because a number invites being read as
    /// a measurement of the evidence.
    ///
    /// Only extracted content is searched. Advocacy items, annotations, and
    /// decision briefs are privileged, and a search that reached them would be a
    /// way for attorney analysis to surface somewhere that does not know it.
    pub fn search(&self, case_id: &CaseId, query: &str, limit: u32) -> Result<Vec<SearchHit>> {
        self.require_case(case_id)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(Error::InvalidSearch(
                "a search needs something to look for".to_owned(),
            ));
        }

        let mut statement = self.connection.prepare_cached(
            "SELECT c.id, c.kind,
                    snippet(content_search, 0, '[', ']', '…', 16),
                    c.text, src.logical_name, seg.locator, c.review_state, c.machine_generated
             FROM content_search
             JOIN content c ON c.rowid = content_search.rowid
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE content_search MATCH ?1 AND c.case_id = ?2
             ORDER BY bm25(content_search), c.id
             LIMIT ?3",
        )?;
        let hits = statement
            .query_map(params![query, case_id.0, limit], |row| {
                Ok(SearchHit {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    excerpt: row.get(2)?,
                    text: row.get(3)?,
                    source: row.get(4)?,
                    locator: row.get(5)?,
                    review_state: row.get(6)?,
                    machine_generated: row.get(7)?,
                    bears_on: Vec::new(),
                })
            })
            .map_err(|error| unreadable_query(query, error))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| unreadable_query(query, error))?;

        // What each hit is already tied to, read in one pass rather than once
        // per hit. A passage nobody has connected to anything is worth seeing.
        let mut links = self.connection.prepare_cached(
            "SELECT e.source_id, e.relation || ': ' || p.text
             FROM edges e
             JOIN propositions p ON p.id = e.target_id AND p.case_id = e.case_id
             WHERE e.case_id = ?1 AND e.source_kind = 'content'
               AND e.target_kind = 'proposition' AND e.review_state <> 'rejected'
             ORDER BY e.source_id, e.relation, p.text",
        )?;
        let mut by_content: HashMap<String, Vec<String>> = HashMap::new();
        for row in links.query_map([&case_id.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (content_id, description) = row?;
            by_content.entry(content_id).or_default().push(description);
        }

        Ok(hits
            .into_iter()
            .map(|mut hit| {
                hit.bears_on = by_content.remove(&hit.id).unwrap_or_default();
                hit
            })
            .collect())
    }

    /// Stores embeddings against derived stills the case already holds.
    ///
    /// This is a finder index, not evidence: the write inserts no content and
    /// no edge. A still that is not `derived_from` another source in this case
    /// is refused. Re-running the same `(source_id, model)` replaces the vector.
    pub fn index_keyframes(&mut self, index: &KeyframeIndex) -> Result<()> {
        self.require_case(&index.case_id)?;
        Self::validate_keyframe_index(index)?;
        let transaction = self.connection.transaction()?;
        Self::index_keyframes_tx(&transaction, index)?;
        transaction.commit()?;
        Ok(())
    }

    fn validate_keyframe_index(index: &KeyframeIndex) -> Result<()> {
        if index.embeddings.is_empty() {
            return Err(Error::InvalidIndex(
                "an index needs at least one keyframe".to_owned(),
            ));
        }

        let mut seen = HashSet::new();
        let mut model_dim: HashMap<&str, usize> = HashMap::new();
        for item in &index.embeddings {
            validate_embedding_vector(item)?;
            if !seen.insert((item.source_id.as_str(), item.model.as_str())) {
                return Err(Error::InvalidIndex(format!(
                    "still `{}` is embedded twice for model `{}` in this index",
                    item.source_id, item.model
                )));
            }
            match model_dim.entry(item.model.as_str()) {
                std::collections::hash_map::Entry::Occupied(existing)
                    if *existing.get() != item.vector.len() =>
                {
                    return Err(Error::InvalidIndex(format!(
                        "model `{}` mixes {}- and {}-dimensional vectors",
                        item.model,
                        existing.get(),
                        item.vector.len()
                    )));
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(item.vector.len());
                }
                std::collections::hash_map::Entry::Occupied(_) => {}
            }
        }
        Ok(())
    }

    fn index_keyframes_tx(
        transaction: &rusqlite::Transaction<'_>,
        index: &KeyframeIndex,
    ) -> Result<()> {
        let mut source_case =
            transaction.prepare_cached("SELECT case_id FROM sources WHERE id = ?1")?;
        let mut is_derived = transaction.prepare_cached(
            "SELECT 1 FROM edges
             WHERE case_id = ?1 AND relation = 'derived_from'
               AND source_kind = 'source' AND source_id = ?2
               AND target_kind = 'source'",
        )?;
        let mut existing_dim = transaction.prepare_cached(
            "SELECT dim FROM keyframe_embeddings WHERE case_id = ?1 AND model = ?2 LIMIT 1",
        )?;
        let mut upsert = transaction.prepare_cached(
            "INSERT INTO keyframe_embeddings
               (source_id, case_id, model, dim, vector, extractor, extractor_version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source_id, model) DO UPDATE SET
               case_id = excluded.case_id,
               dim = excluded.dim,
               vector = excluded.vector,
               extractor = excluded.extractor,
               extractor_version = excluded.extractor_version",
        )?;

        let mut dim_written: HashMap<String, i64> = HashMap::new();
        for item in &index.embeddings {
            let owner: Option<String> = source_case
                .query_row([&item.source_id], |row| row.get(0))
                .optional()?;
            match owner {
                Some(owner) if owner == index.case_id.0 => {}
                Some(_) => {
                    return Err(Error::WrongCase {
                        kind: "source",
                        id: item.source_id.clone(),
                    });
                }
                None => {
                    return Err(Error::NotFound {
                        kind: "source",
                        id: item.source_id.clone(),
                    });
                }
            }

            let derived = is_derived
                .query_row(params![index.case_id.0, item.source_id], |_| Ok(()))
                .optional()?
                .is_some();
            if !derived {
                return Err(Error::InvalidIndex(format!(
                    "source `{}` is not a derived still",
                    item.source_id
                )));
            }

            let dim = i64::try_from(item.vector.len()).map_err(|_| {
                Error::InvalidIndex(format!(
                    "still `{}` vector is too long to store",
                    item.source_id
                ))
            })?;
            let stored: Option<i64> = match dim_written.get(&item.model) {
                Some(value) => Some(*value),
                None => existing_dim
                    .query_row(params![index.case_id.0, item.model], |row| row.get(0))
                    .optional()?,
            };
            if let Some(stored) = stored
                && stored != dim
            {
                return Err(Error::InvalidIndex(format!(
                    "model `{}` is {stored}-dimensional in this case, not {dim}",
                    item.model
                )));
            }
            dim_written.insert(item.model.clone(), dim);

            upsert.execute(params![
                item.source_id,
                index.case_id.0,
                item.model,
                dim,
                encode_vector(&item.vector),
                item.extractor,
                item.version
            ])?;
        }

        drop(upsert);
        drop(existing_dim);
        drop(is_derived);
        drop(source_case);
        Ok(())
    }

    /// Selects the closest stills for `query`, then presents them by identifier.
    ///
    /// Similarity selects a bounded candidate pool but is then discarded; hits
    /// are presented by still identifier and contain no rank or score. An
    /// unknown model is an error so the operator can tell an unindexed case
    /// from an empty index.
    pub fn search_keyframes(
        &self,
        case_id: &CaseId,
        model: &str,
        query: &[f32],
        limit: u32,
    ) -> Result<Vec<KeyframeHit>> {
        self.require_case(case_id)?;
        let model = model.trim();
        if model.is_empty() {
            return Err(Error::InvalidSearch(
                "a visual search needs a model".to_owned(),
            ));
        }
        validate_query_vector(query)?;
        if limit == 0 {
            return Ok(Vec::new());
        }

        let mut statement = self.connection.prepare_cached(
            "SELECT e.source_id, e.vector, e.dim, still.sha256,
                    video.logical_name, video.sha256,
                    seg.locator, c.review_state, c.machine_generated, c.id
             FROM keyframe_embeddings e
             JOIN sources still ON still.id = e.source_id AND still.case_id = e.case_id
             JOIN edges der ON der.case_id = e.case_id
               AND der.source_kind = 'source' AND der.source_id = e.source_id
               AND der.relation = 'derived_from' AND der.target_kind = 'source'
               AND der.review_state <> 'rejected'
             JOIN sources video ON video.id = der.target_id AND video.case_id = e.case_id
             JOIN source_segments seg ON seg.source_id = still.id
             JOIN content c ON c.segment_id = seg.id AND c.case_id = e.case_id
             WHERE e.case_id = ?1 AND e.model = ?2
             ORDER BY e.source_id, c.id",
        )?;
        let rows = statement
            .query_map(params![case_id.0, model], |row| {
                Ok(IndexedRow {
                    source_id: row.get(0)?,
                    vector: row.get(1)?,
                    dim: row.get(2)?,
                    still_sha256: row.get(3)?,
                    source: row.get(4)?,
                    sha256: row.get(5)?,
                    locator: row.get(6)?,
                    review_state: row.get(7)?,
                    machine_generated: row.get(8)?,
                    content_id: row.get(9)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        if rows.is_empty() {
            let any: bool = self.exists(
                "SELECT 1 FROM keyframe_embeddings WHERE case_id = ?1 AND model = ?2 LIMIT 1",
                params![case_id.0, model],
            )?;
            if any {
                return Ok(Vec::new());
            }
            return Err(Error::InvalidSearch(format!(
                "case has no keyframe embeddings for model `{model}`"
            )));
        }

        let mut first_by_still: HashMap<String, IndexedRow> = HashMap::new();
        for row in rows {
            first_by_still.entry(row.source_id.clone()).or_insert(row);
        }

        let query_dim = i64::try_from(query.len()).unwrap_or(i64::MAX);
        let mut scored = Vec::new();
        for (source_id, row) in first_by_still {
            if row.dim != query_dim {
                return Err(Error::InvalidSearch(format!(
                    "model `{model}` is {}-dimensional in this case, not {}",
                    row.dim,
                    query.len()
                )));
            }
            let coordinates = decode_vector(&row.vector, row.dim)?;
            let Some(similarity) = cosine(query, &coordinates) else {
                continue;
            };
            scored.push((source_id, row, similarity));
        }
        scored.sort_by(|left, right| {
            right
                .2
                .total_cmp(&left.2)
                .then_with(|| left.0.cmp(&right.0))
        });
        scored.truncate(limit as usize);
        scored.sort_by(|left, right| left.0.cmp(&right.0));

        let mut links = self.connection.prepare_cached(
            "SELECT e.source_id, e.relation || ': ' || p.text
             FROM edges e
             JOIN propositions p ON p.id = e.target_id AND p.case_id = e.case_id
             WHERE e.case_id = ?1 AND e.source_kind = 'content'
               AND e.target_kind = 'proposition' AND e.review_state <> 'rejected'
             ORDER BY e.source_id, e.relation, p.text",
        )?;
        let mut by_content: HashMap<String, Vec<String>> = HashMap::new();
        for row in links.query_map([&case_id.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (content_id, description) = row?;
            by_content.entry(content_id).or_default().push(description);
        }

        Ok(scored
            .into_iter()
            .map(|(_, row, _)| KeyframeHit {
                source_id: row.source_id,
                still_sha256: row.still_sha256,
                source: row.source,
                sha256: row.sha256,
                locator: row.locator,
                review_state: row.review_state,
                machine_generated: row.machine_generated,
                bears_on: by_content.remove(&row.content_id).unwrap_or_default(),
            })
            .collect())
    }

    /// Embedding spaces currently present in one case's finder index.
    pub fn keyframe_models(&self, case_id: &CaseId) -> Result<Vec<String>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT model FROM keyframe_embeddings
             WHERE case_id = ?1 ORDER BY model",
        )?;
        let rows = statement.query_map([&case_id.0], |row| row.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Reports where the case stands, element by element.
    ///
    /// This is the view a defender opens first: not what the case contains, but
    /// what its charges rest on and where they are thin. Every number here
    /// counts something the case holds — propositions mapped in a direction,
    /// distinct sources under them, material nobody has checked. Nothing is
    /// weighted and nothing is ranked by strength, because the moment a tool
    /// says which element is *weak* it has made the argument for the person
    /// whose job that is.
    ///
    /// What it will say is structural, and a defender can act on structure:
    /// that an element's support all traces to one report, that a proposition
    /// carries evidence both ways, that a gap in the record touches a charged
    /// element rather than an idle corner of the file.
    pub fn case_standing(&self, case_id: &CaseId) -> Result<CaseStanding> {
        let case_name = self.case_name(case_id)?;

        let mut charge_statement = self.connection.prepare_cached(
            "SELECT ch.id, ch.label, ch.citation, ch.posture, ch.grade,
                    el.id, el.ordinal, el.text
             FROM charges ch
             JOIN elements el ON el.charge_id = ch.id
             WHERE ch.case_id = ?1
             ORDER BY CASE ch.posture
               WHEN 'charged' THEN 1 WHEN 'lesser_candidate' THEN 2
               WHEN 'alternative' THEN 3 ELSE 4 END, ch.label, el.ordinal",
        )?;

        // Element identifier -> where it sits, so every later pass can find it
        // by identifier without re-querying.
        let mut charges: Vec<ChargeStanding> = Vec::new();
        let mut placement: HashMap<String, (usize, usize)> = HashMap::new();
        let mut label_of: HashMap<String, String> = HashMap::new();
        for row in charge_statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, u32>(6)?,
                row.get::<_, String>(7)?,
            ))
        })? {
            let (id, label, citation, posture, grade, element_id, ordinal, text) = row?;
            if charges.last().map(|charge| charge.id.as_str()) != Some(id.as_str()) {
                charges.push(ChargeStanding {
                    id,
                    charge: label.clone(),
                    citation,
                    posture,
                    grade,
                    elements: Vec::new(),
                });
            }
            let charge_index = charges.len() - 1;
            let elements = &mut charges[charge_index].elements;
            label_of.insert(element_id.clone(), format!("{label}, element {ordinal}"));
            placement.insert(element_id, (charge_index, elements.len()));
            elements.push(ElementStanding {
                ordinal,
                element: text,
                supporting: 0,
                opposing: 0,
                uncertain: 0,
                excluded: 0,
                sources_behind_support: 0,
                sole_source: None,
                unchecked_support: 0,
                unbacked: 0,
            });
        }

        // One row per mapping: which direction a person filed the proposition
        // under, whether anything source-grounded reaches it at all, and
        // whether a person has checked any of what supports it.
        let mut mapping_statement = self.connection.prepare_cached(
            "SELECT link.element_id, link.assessment, prop.id,
                    (SELECT count(*) FROM edges e
                      WHERE e.case_id = prop.case_id AND e.target_kind = 'proposition'
                        AND e.target_id = prop.id AND e.source_kind = 'content'
                        AND e.review_state <> 'rejected'),
                    (SELECT count(*) FROM edges e
                      JOIN content c ON c.id = e.source_id
                      WHERE e.case_id = prop.case_id AND e.target_kind = 'proposition'
                        AND e.target_id = prop.id AND e.source_kind = 'content'
                        AND e.review_state <> 'rejected'
                        AND e.relation IN ('supports','corroborates')
                        AND c.review_state IN ('reviewed','verified'))
             FROM element_links link
             JOIN elements el ON el.id = link.element_id
             JOIN charges ch ON ch.id = el.charge_id
             JOIN propositions prop ON prop.id = link.proposition_id
             WHERE ch.case_id = ?1 AND prop.case_id = ?1
             ORDER BY link.element_id, prop.id",
        )?;
        let mut elements_of_proposition: HashMap<String, Vec<String>> = HashMap::new();
        for row in mapping_statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, u32>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })? {
            let (element_id, assessment, proposition_id, backing, checked) = row?;
            if let Some(label) = label_of.get(&element_id) {
                elements_of_proposition
                    .entry(proposition_id)
                    .or_default()
                    .push(label.clone());
            }
            let Some(standing) = placement
                .get(&element_id)
                .and_then(|&(charge, element)| charges[charge].elements.get_mut(element))
            else {
                continue;
            };
            let direction = ElementAssessment::from_db(&assessment).ok_or_else(|| {
                Error::InvalidAuthoring(format!(
                    "element `{element_id}` holds unrecognized assessment `{assessment}`"
                ))
            })?;
            match direction {
                ElementAssessment::Supports => {
                    standing.supporting += 1;
                    if checked == 0 {
                        standing.unchecked_support += 1;
                    }
                }
                ElementAssessment::Opposes => standing.opposing += 1,
                ElementAssessment::Uncertain => standing.uncertain += 1,
                ElementAssessment::Excluded => standing.excluded += 1,
            }
            if backing == 0 {
                standing.unbacked += 1;
            }
        }

        // The distinct originals under each element's support. An element whose
        // support all arrives through one source fails entirely if that source
        // does, which is a fact about the record rather than a judgment on it.
        let mut source_statement = self.connection.prepare_cached(
            "SELECT link.element_id, src.id, src.logical_name, src.review_state
             FROM element_links link
             JOIN elements el ON el.id = link.element_id
             JOIN charges ch ON ch.id = el.charge_id
             JOIN propositions prop ON prop.id = link.proposition_id
             JOIN edges e ON e.case_id = prop.case_id AND e.target_kind = 'proposition'
                         AND e.target_id = prop.id AND e.source_kind = 'content'
                         AND e.review_state <> 'rejected'
                         AND e.relation IN ('supports','corroborates')
             JOIN content c ON c.id = e.source_id
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE ch.case_id = ?1 AND prop.case_id = ?1 AND link.assessment = 'supports'
             GROUP BY link.element_id, src.id
             ORDER BY link.element_id, src.logical_name, src.id",
        )?;
        let mut sources_of_element: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for row in source_statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })? {
            let (element_id, source_id, name, review_state) = row?;
            sources_of_element
                .entry(element_id)
                .or_default()
                .push((source_id, name, review_state));
        }

        let mut load_bearing: HashMap<String, LoadBearingSource> = HashMap::new();
        for (element_id, sources) in &sources_of_element {
            let Some(standing) = placement
                .get(element_id)
                .and_then(|&(charge, element)| charges[charge].elements.get_mut(element))
            else {
                continue;
            };
            standing.sources_behind_support = u32::try_from(sources.len()).unwrap_or(u32::MAX);
            if let [(source_id, name, review_state)] = sources.as_slice() {
                standing.sole_source = Some(name.clone());
                let entry =
                    load_bearing
                        .entry(source_id.clone())
                        .or_insert_with(|| LoadBearingSource {
                            id: source_id.clone(),
                            source: name.clone(),
                            review_state: review_state.clone(),
                            sole_support_for: Vec::new(),
                        });
                if let Some(label) = label_of.get(element_id) {
                    entry.sole_support_for.push(label.clone());
                }
            }
        }
        let mut load_bearing_sources: Vec<LoadBearingSource> = load_bearing.into_values().collect();
        for source in &mut load_bearing_sources {
            source.sole_support_for.sort();
        }
        load_bearing_sources.sort_by(|left, right| {
            right
                .sole_support_for
                .len()
                .cmp(&left.sole_support_for.len())
                .then_with(|| left.source.cmp(&right.source))
        });

        Ok(CaseStanding {
            case_id: case_id.0.clone(),
            case_name,
            charges,
            load_bearing_sources,
            live_disputes: self.live_disputes(case_id, &elements_of_proposition)?,
            open_gaps: self.open_gaps(case_id, &elements_of_proposition)?,
        })
    }

    /// Returns propositions carrying source-grounded evidence in both directions.
    ///
    /// Not a problem to resolve. A proposition with evidence pulling both ways
    /// is the contested ground the case is actually fought on, and the kernel's
    /// whole posture is that it stays contested until a person says otherwise.
    fn live_disputes(
        &self,
        case_id: &CaseId,
        elements_of_proposition: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<LiveDispute>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT p.id, p.text,
                    sum(CASE WHEN e.relation IN ('supports','corroborates') THEN 1 ELSE 0 END),
                    sum(CASE WHEN e.relation IN ('contradicts','impeaches') THEN 1 ELSE 0 END)
             FROM propositions p
             JOIN edges e ON e.case_id = p.case_id AND e.target_kind = 'proposition'
                         AND e.target_id = p.id AND e.source_kind = 'content'
                         AND e.review_state <> 'rejected'
             WHERE p.case_id = ?1
             GROUP BY p.id
             HAVING sum(CASE WHEN e.relation IN ('supports','corroborates') THEN 1 ELSE 0 END) > 0
                AND sum(CASE WHEN e.relation IN ('contradicts','impeaches') THEN 1 ELSE 0 END) > 0
             ORDER BY p.text, p.id",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            let id: String = row.get(0)?;
            Ok(LiveDispute {
                proposition: row.get(1)?,
                supporting_evidence: row.get(2)?,
                contradicting_evidence: row.get(3)?,
                bears_on: elements_of_proposition
                    .get(&id)
                    .cloned()
                    .unwrap_or_default(),
                id,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Collects every analyzer finding, those bearing on a charge first.
    ///
    /// The analyzers already report these; what this adds is where each one
    /// lands. A reference nobody resolved matters differently depending on
    /// whether it touches a charged element or a corner of the file, and a
    /// defender with an hour should not have to work that out by hand.
    fn open_gaps(
        &self,
        case_id: &CaseId,
        elements_of_proposition: &HashMap<String, Vec<String>>,
    ) -> Result<Vec<OpenGap>> {
        let mut gaps = Vec::new();
        for kind in SuggestionKind::ALL {
            if kind.proposes_relationships() {
                continue;
            }
            for finding in self.findings(case_id, kind)? {
                let bears_on = elements_of_proposition
                    .get(&finding.subject_id)
                    .cloned()
                    .unwrap_or_default();
                gaps.push(OpenGap {
                    analyzer: kind.as_str().to_owned(),
                    subject_kind: finding.subject_kind,
                    subject_id: finding.subject_id,
                    subject: finding.subject,
                    summary: finding.summary,
                    bears_on,
                });
            }
        }
        gaps.sort_by(|left, right| {
            // A gap that touches a charged element comes first: `false` sorts
            // before `true`, so comparing `is_empty()` left-to-right puts the
            // ones that bear on something at the top.
            left.bears_on
                .is_empty()
                .cmp(&right.bears_on.is_empty())
                .then_with(|| right.bears_on.len().cmp(&left.bears_on.len()))
                .then_with(|| left.analyzer.cmp(&right.analyzer))
                .then_with(|| left.subject.cmp(&right.subject))
        });
        Ok(gaps)
    }

    /// Runs deterministic analyzers and proposes what they find.
    ///
    /// Every proposal enters as `suggested` and joins the review queue. Nothing
    /// here reviews, verifies, merges, scores, or touches a record a person
    /// wrote; an analyzer's whole authority is to point at a pair and say why.
    ///
    /// Running twice proposes nothing new. A claim the case already holds is
    /// counted and skipped, whether a person asserted it, an earlier run
    /// proposed it, or a reviewer rejected it — a reviewer who has said no does
    /// not need to be asked again next time the analyzer runs.
    pub fn suggest(&mut self, case_id: &CaseId, kinds: &[SuggestionKind]) -> Result<SuggestionRun> {
        self.require_case(case_id)?;
        let mut analyzers = Vec::with_capacity(kinds.len());
        let mut proposed = 0_u32;
        let mut already_recorded = 0_u32;

        let mut findings = 0_u32;

        for kind in kinds {
            let attribution = kind.attribution();
            let mut written = Vec::new();
            let mut skipped = 0_u32;
            let mut reported = Vec::new();

            if kind.proposes_relationships() {
                for candidate in self.candidates(case_id, *kind)? {
                    match self.propose(case_id, &candidate, &attribution)? {
                        Some(link) => written.push(link),
                        None => skipped += 1,
                    }
                }
            } else {
                reported = self.findings(case_id, *kind)?;
            }

            proposed += u32::try_from(written.len()).unwrap_or(u32::MAX);
            already_recorded += skipped;
            findings += u32::try_from(reported.len()).unwrap_or(u32::MAX);
            analyzers.push(AnalyzerReport {
                analyzer: attribution,
                proposed: written,
                already_recorded: skipped,
                findings: reported,
            });
        }

        Ok(SuggestionRun {
            case_id: case_id.0.clone(),
            analyzers,
            proposed,
            already_recorded,
            findings,
        })
    }

    /// Reports the gaps one analyzer found. Nothing is written.
    fn findings(&self, case_id: &CaseId, kind: SuggestionKind) -> Result<Vec<Finding>> {
        // A proposition rests on evidence when some content bears on it and a
        // reviewer has not rejected the link.
        const SUPPORTED: &str = "EXISTS (SELECT 1 FROM edges e
                                  WHERE e.case_id = p.case_id AND e.target_kind = 'proposition'
                                    AND e.target_id = p.id AND e.source_kind = 'content'
                                    AND e.review_state <> 'rejected')";

        let (subject_kind, sql) = match kind {
            SuggestionKind::UnsupportedProposition => (
                NodeKind::Proposition,
                format!(
                    "SELECT p.id, p.text,
                            'Nothing source-grounded bears on this proposition, so no reader \
                             can check it. Link evidence to it or withdraw it.'
                     FROM propositions p
                     WHERE p.case_id = ?1 AND NOT {SUPPORTED}
                     ORDER BY p.text, p.id"
                ),
            ),
            SuggestionKind::UnmappedProposition => (
                NodeKind::Proposition,
                format!(
                    "SELECT p.id, p.text,
                            'Evidence bears on this proposition but it is mapped to no element \
                             of any charge, so it does not reach the element matrix.'
                     FROM propositions p
                     WHERE p.case_id = ?1 AND {SUPPORTED}
                       AND NOT EXISTS (SELECT 1 FROM element_links link
                                       JOIN elements el ON el.id = link.element_id
                                       JOIN charges ch ON ch.id = el.charge_id
                                       WHERE link.proposition_id = p.id
                                         AND ch.case_id = p.case_id)
                     ORDER BY p.text, p.id"
                ),
            ),
            SuggestionKind::UnresolvedReference => (
                NodeKind::Content,
                "SELECT c.id, c.text,
                        'This passage refers to evidence, but nothing in the case says whether \
                         that evidence was produced, is missing, or was never sought.'
                 FROM content c
                 WHERE c.case_id = ?1 AND c.kind = 'evidence_reference'
                   AND NOT EXISTS (SELECT 1 FROM edges e
                                   WHERE e.case_id = c.case_id AND e.source_kind = 'content'
                                     AND e.source_id = c.id AND e.target_kind = 'source')
                 ORDER BY c.text, c.id"
                    .to_owned(),
            ),
            // Two sources placing the same proposition at different times. No
            // tolerance window: how much disagreement matters is a judgment,
            // and stating one here would make it the tool's rather than the
            // defender's.
            SuggestionKind::ClockDisagreement => (
                NodeKind::Proposition,
                "SELECT p.id, p.text,
                        'Sources place this at different times: ' || sa.logical_name || ' gives '
                          || COALESCE(a.asserted_time, a.normalized_start) || ', '
                          || sb.logical_name || ' gives '
                          || COALESCE(b.asserted_time, b.normalized_start)
                          || '. Raw times are never overwritten; reconciling them is a \
                              reviewable hypothesis.'
                 FROM propositions p
                 JOIN edges ea ON ea.case_id = p.case_id AND ea.target_kind = 'proposition'
                              AND ea.target_id = p.id AND ea.source_kind = 'content'
                              AND ea.review_state <> 'rejected'
                 JOIN edges eb ON eb.case_id = p.case_id AND eb.target_kind = 'proposition'
                              AND eb.target_id = p.id AND eb.source_kind = 'content'
                              AND eb.review_state <> 'rejected'
                 JOIN content a ON a.id = ea.source_id
                 JOIN content b ON b.id = eb.source_id
                 JOIN source_segments ga ON ga.id = a.segment_id
                 JOIN source_segments gb ON gb.id = b.segment_id
                 JOIN sources sa ON sa.id = ga.source_id
                 JOIN sources sb ON sb.id = gb.source_id
                 WHERE p.case_id = ?1 AND sa.id < sb.id
                   AND COALESCE(a.asserted_time, a.normalized_start) IS NOT NULL
                   AND COALESCE(b.asserted_time, b.normalized_start) IS NOT NULL
                   AND COALESCE(a.asserted_time, a.normalized_start)
                       <> COALESCE(b.asserted_time, b.normalized_start)
                 GROUP BY p.id, sa.id, sb.id
                 ORDER BY p.text, sa.logical_name, sb.logical_name"
                    .to_owned(),
            ),
            _ => {
                return Err(Error::InvalidAuthoring(format!(
                    "`{}` proposes relationships and reports no findings",
                    kind.as_str()
                )));
            }
        };

        let mut statement = self.connection.prepare_cached(&sql)?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(Finding {
                subject_kind: subject_kind.as_str().to_owned(),
                subject_id: row.get(0)?,
                subject: row.get(1)?,
                summary: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Writes one proposal, or reports that the case already held the claim.
    fn propose(
        &self,
        case_id: &CaseId,
        candidate: &Candidate,
        attribution: &str,
    ) -> Result<Option<AuthoredLink>> {
        // An analyzer points at a *pair*. If the case already relates that pair
        // this way, the claim is held however it happens to be oriented — a
        // person who wrote `b impeaches a` has answered the question, and the
        // mirror image is not a second thing to review. The unique index only
        // sees one orientation, so the check is made here.
        let held = self.exists(
            "SELECT 1 FROM edges
             WHERE case_id = ?1 AND relation = ?2
               AND source_kind = ?3 AND target_kind = ?3
               AND ((source_id = ?4 AND target_id = ?5)
                 OR (source_id = ?5 AND target_id = ?4))",
            params![
                case_id.0,
                candidate.relation.as_str(),
                candidate.kind.as_str(),
                candidate.from,
                candidate.to
            ],
        )?;
        if held {
            return Ok(None);
        }

        let id = Uuid::now_v7().to_string();
        let written = self
            .connection
            .prepare_cached(
                "INSERT INTO edges
                   (id, case_id, source_kind, source_id, relation, target_kind, target_id,
                    rationale, review_state, created_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'suggested', ?9)
                 ON CONFLICT (case_id, source_kind, source_id, relation, target_kind, target_id)
                   DO NOTHING",
            )?
            .execute(params![
                id,
                case_id.0,
                candidate.kind.as_str(),
                candidate.from,
                candidate.relation.as_str(),
                candidate.kind.as_str(),
                candidate.to,
                candidate.rationale,
                attribution
            ])?;
        if written == 0 {
            return Ok(None);
        }
        Ok(Some(AuthoredLink {
            id,
            from_kind: candidate.kind.as_str().to_owned(),
            from_id: candidate.from.clone(),
            relation: candidate.relation.as_str().to_owned(),
            to_kind: candidate.kind.as_str().to_owned(),
            to_id: candidate.to.clone(),
            rationale: candidate.rationale.clone(),
            review_state: ReviewState::Suggested.as_str().to_owned(),
            created_by: attribution.to_owned(),
        }))
    }

    /// Runs one analyzer's query and returns what it found.
    ///
    /// Each pair is ordered by identifier so that a second run proposes the
    /// same direction as the first. Without that, the mirror image of an
    /// existing suggestion would look like a new claim.
    fn candidates(&self, case_id: &CaseId, kind: SuggestionKind) -> Result<Vec<Candidate>> {
        if matches!(kind, SuggestionKind::DuplicateEntity) {
            return self.duplicate_people(case_id);
        }
        let (node_kind, relation, sql) = match kind {
            // Two lanes describing overlapping time. Half-open intervals: an
            // event starting exactly when another ends does not overlap it, and
            // an event with no recorded end is a point in time, not an infinity.
            SuggestionKind::TemporalOverlap => (
                NodeKind::Event,
                EdgeKind::TemporallyOverlaps,
                "SELECT a.id, b.id,
                        'Normalized intervals overlap across lanes: ' || a.lane || ' ('
                          || a.normalized_start || ') and ' || b.lane || ' ('
                          || b.normalized_start || '). Proposed as overlap only; \
                             the lanes remain separate accounts.'
                 FROM events a
                 JOIN events b
                   ON b.case_id = a.case_id AND b.id > a.id AND b.lane <> a.lane
                 WHERE a.case_id = ?1
                   AND a.normalized_start IS NOT NULL AND b.normalized_start IS NOT NULL
                   AND a.normalized_start < COALESCE(b.normalized_end, b.normalized_start)
                   AND b.normalized_start < COALESCE(a.normalized_end, a.normalized_start)
                 ORDER BY a.id, b.id",
            ),
            // Two excerpts pulling opposite ways on one proposition. Pairs that
            // share an attributed witness belong to the analyzer below.
            SuggestionKind::ContradictionCandidate => (
                NodeKind::Content,
                EdgeKind::Contradicts,
                // Which excerpt supports and which contradicts is independent of
                // which identifier sorts first, so the pair is matched on the
                // relations and only then ordered for a stable edge direction.
                "SELECT DISTINCT
                        MIN(pro.source_id, con.source_id),
                        MAX(pro.source_id, con.source_id),
                        'Two excerpts bear on the same proposition in opposite directions ('
                          || pro.relation || ' and ' || con.relation
                          || '). Proposed as a tension to examine, not as a finding \
                              that either is wrong.'
                 FROM edges pro
                 JOIN edges con
                   ON con.case_id = pro.case_id
                  AND con.target_kind = 'proposition' AND pro.target_kind = 'proposition'
                  AND con.target_id = pro.target_id
                  AND con.source_kind = 'content' AND pro.source_kind = 'content'
                  AND con.source_id <> pro.source_id
                 JOIN content a ON a.id = pro.source_id
                 JOIN content b ON b.id = con.source_id
                 WHERE pro.case_id = ?1
                   AND pro.relation IN ('supports','corroborates')
                   AND con.relation IN ('contradicts','impeaches')
                   AND pro.review_state <> 'rejected' AND con.review_state <> 'rejected'
                   AND (a.attributed_to_entity_id IS NULL
                        OR b.attributed_to_entity_id IS NULL
                        OR a.attributed_to_entity_id <> b.attributed_to_entity_id)
                 ORDER BY 1, 2",
            ),
            // One witness, two accounts, opposite directions.
            SuggestionKind::ConflictingAttribution => (
                NodeKind::Content,
                EdgeKind::Impeaches,
                "SELECT DISTINCT
                        MIN(pro.source_id, con.source_id),
                        MAX(pro.source_id, con.source_id),
                        'Two accounts attributed to ' || entity.display_name
                          || ' bear opposite ways on the same proposition ('
                          || pro.relation || ' and ' || con.relation
                          || '). The accounts are not merged and neither is preferred.'
                 FROM edges pro
                 JOIN edges con
                   ON con.case_id = pro.case_id
                  AND con.target_kind = 'proposition' AND pro.target_kind = 'proposition'
                  AND con.target_id = pro.target_id
                  AND con.source_kind = 'content' AND pro.source_kind = 'content'
                  AND con.source_id <> pro.source_id
                 JOIN content a ON a.id = pro.source_id
                 JOIN content b ON b.id = con.source_id
                 JOIN entities entity ON entity.id = a.attributed_to_entity_id
                 WHERE pro.case_id = ?1
                   AND pro.relation IN ('supports','corroborates')
                   AND con.relation IN ('contradicts','impeaches')
                   AND pro.review_state <> 'rejected' AND con.review_state <> 'rejected'
                   AND a.attributed_to_entity_id = b.attributed_to_entity_id
                 ORDER BY 1, 2",
            ),
            _ => {
                return Err(Error::InvalidAuthoring(format!(
                    "`{}` reports findings and proposes no relationships",
                    kind.as_str()
                )));
            }
        };

        let mut statement = self.connection.prepare_cached(sql)?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(Candidate {
                kind: node_kind,
                relation,
                from: row.get(0)?,
                to: row.get(1)?,
                rationale: row.get(2)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Assembles a source-linked export of the case.
    ///
    /// Two rules govern this and neither is left to the caller. Every factual
    /// line resolves to an exact original locator; a proposition that resolves
    /// to nothing openable is listed as unsupported rather than exported as a
    /// bare assertion. And a disclosable export never reads the advocacy,
    /// annotation, or brief tables at all — privilege is excluded structurally,
    /// not by filtering a flag a future writer could set wrongly.
    ///
    /// Evidence a reviewer rejected is left out, as everywhere else, but it is
    /// counted: nothing leaves this tool silently reduced.
    pub fn export_case(&self, case_id: &CaseId, audience: ExportAudience) -> Result<CaseExport> {
        let case_name = self.case_name(case_id)?;

        let mut statement = self.connection.prepare_cached(
            "SELECT id, text, status, review_state FROM propositions
             WHERE case_id = ?1 ORDER BY text, id",
        )?;
        let rows = statement
            .query_map([&case_id.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Every proposition's evidence is read in one pass rather than a query
        // per proposition: an export walks the whole case, and asking the same
        // question once per line is what makes a large one slow.
        let mut by_proposition = self.evidence_by_proposition(case_id)?;

        let mut propositions = Vec::with_capacity(rows.len());
        let mut unsupported = Vec::new();
        let mut unreviewed_evidence_included = 0_u32;
        for (id, text, status, review_state) in rows {
            let evidence = by_proposition.remove(&id).unwrap_or_default();
            if evidence.is_empty() {
                unsupported.push(UnsupportedProposition {
                    id,
                    text,
                    reason: "no source-grounded evidence resolves to this proposition".to_owned(),
                });
                continue;
            }
            // Rule twelve, checked rather than trusted: a line a reader cannot
            // open is not a fact, and this is the last place to catch one.
            if let Some(unlocated) = evidence.iter().find(|item| item.locator.trim().is_empty()) {
                return Err(Error::UnlocatedExport {
                    proposition: id,
                    source_name: unlocated.source.clone(),
                });
            }
            unreviewed_evidence_included += evidence
                .iter()
                .filter(|item| {
                    is_intake_state(&item.review_state)
                        || is_intake_state(&item.relation_review_state)
                })
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            propositions.push(ExportedProposition {
                id,
                text,
                status,
                review_state,
                evidence,
            });
        }

        let rejected_evidence_omitted = self
            .query_one(
                "SELECT count(*) FROM edges
                 WHERE case_id = ?1 AND target_kind = 'proposition'
                   AND source_kind = 'content' AND review_state = 'rejected'",
                [&case_id.0],
                |row| row.get(0),
            )?
            .unwrap_or(0);

        let privileged = if audience.includes_privileged() {
            self.privileged_work_product(case_id)?
        } else {
            Vec::new()
        };

        Ok(CaseExport {
            case_id: case_id.0.clone(),
            case_name,
            audience: audience.as_str().to_owned(),
            includes_privileged: audience.includes_privileged(),
            productions: self.discovery_ledger(case_id)?,
            propositions,
            unsupported,
            privileged,
            rejected_evidence_omitted,
            unreviewed_evidence_included,
        })
    }

    /// Reads the evidence bearing on every proposition in the case at once.
    ///
    /// The same query as [`Store::proposition_evidence`] without the target
    /// filter, grouped in memory. Each proposition's evidence keeps the order
    /// that function gives it, so an export reads identically either way.
    fn evidence_by_proposition(
        &self,
        case_id: &CaseId,
    ) -> Result<HashMap<String, Vec<PropositionEvidence>>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT edge.target_id, edge.relation, content.text, source.logical_name,
                    segment.locator,
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
               AND edge.review_state != 'rejected'
             ORDER BY edge.target_id,
                      COALESCE(content.normalized_start, content.asserted_time,
                               content.raw_time, ''),
                      source.logical_name,
                      CASE WHEN segment.page IS NULL THEN 1 ELSE 0 END, segment.page,
                      CASE WHEN segment.start_ms IS NULL THEN 1 ELSE 0 END, segment.start_ms,
                      segment.locator, content.id",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                PropositionEvidence {
                    relation: row.get(1)?,
                    text: row.get(2)?,
                    source: row.get(3)?,
                    locator: row.get(4)?,
                    source_time: row.get(5)?,
                    asserted_time: row.get(6)?,
                    normalized_start: row.get(7)?,
                    extractor: row.get(8)?,
                    extractor_version: row.get(9)?,
                    machine_generated: row.get(10)?,
                    extractor_confidence: row.get(11)?,
                    review_state: row.get(12)?,
                    rationale: row.get(13)?,
                    relation_review_state: row.get(14)?,
                },
            ))
        })?;

        let mut grouped: HashMap<String, Vec<PropositionEvidence>> = HashMap::new();
        for row in rows {
            let (proposition_id, evidence) = row?;
            grouped.entry(proposition_id).or_default().push(evidence);
        }
        Ok(grouped)
    }

    /// Reads the current version of every privileged work-product record.
    ///
    /// Only ever called for a work-file export. A disclosable export does not
    /// reach this function, which is what keeps privilege out of it.
    fn privileged_work_product(&self, case_id: &CaseId) -> Result<Vec<ExportedWorkProduct>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT id, kind, title, body, version, author
             FROM advocacy_items item
             WHERE case_id = ?1
               AND NOT EXISTS (SELECT 1 FROM advocacy_items later
                               WHERE later.supersedes_advocacy_id = item.id)
             UNION ALL
             SELECT id, 'annotation', 'annotation on ' || target_kind || ' ' || target_id,
                    body, version, author
             FROM annotations note
             WHERE case_id = ?1
               AND NOT EXISTS (SELECT 1 FROM annotations later
                               WHERE later.supersedes_annotation_id = note.id)
             UNION ALL
             SELECT id, 'decision_brief', posture, summary, version, author
             FROM decision_briefs brief
             WHERE case_id = ?1
               AND version = (SELECT MAX(version) FROM decision_briefs latest
                              WHERE latest.case_id = brief.case_id
                                AND latest.posture = brief.posture)
             ORDER BY 2, 3",
        )?;
        let rows = statement.query_map([&case_id.0], |row| {
            Ok(ExportedWorkProduct {
                id: row.get(0)?,
                kind: row.get(1)?,
                title: row.get(2)?,
                body: row.get(3)?,
                version: row.get(4)?,
                author: row.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Lists records still in an intake state, machine suggestions first.
    ///
    /// This is the work queue implied by the rule that only a person can move
    /// an item out of `unreviewed` or `suggested`. Items carry the exact
    /// original locator so the reviewer can open the source in one action.
    pub fn review_queue(&self, case_id: &CaseId) -> Result<Vec<ReviewQueueItem>> {
        self.require_case(case_id)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT 'content', c.id, c.review_state, c.machine_generated, c.text,
                    src.logical_name || ' @ ' || seg.locator, c.extractor,
                    src.logical_name,
                    CASE WHEN seg.page IS NOT NULL THEN 0
                         WHEN seg.start_ms IS NOT NULL THEN 1 ELSE 2 END,
                    COALESCE(seg.page, seg.start_ms, 0), seg.locator
             FROM content c
             JOIN source_segments seg ON seg.id = c.segment_id
             JOIN sources src ON src.id = seg.source_id
             WHERE c.case_id = ?1 AND c.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'source', s.id, s.review_state, 0, s.logical_name,
                    s.logical_name || ' @ sha256:' || s.sha256, NULL,
                    s.logical_name, 2, 0, ''
             FROM sources s
             WHERE s.case_id = ?1 AND s.review_state IN ('unreviewed','suggested')
             UNION ALL
             -- An analyzer can propose a relationship, so an edge is machine-
             -- generated exactly when a suggester wrote it, and names that
             -- suggester the way extracted content names its extractor.
             SELECT 'edge', e.id, e.review_state,
                    e.created_by LIKE 'suggest:%',
                    e.source_kind || ' ' || e.source_id || ' ' || e.relation || ' '
                      || e.target_kind || ' ' || e.target_id,
                    NULL,
                    CASE WHEN e.created_by LIKE 'suggest:%' THEN e.created_by END,
                    e.id, 2, 0, ''
             FROM edges e
             WHERE e.case_id = ?1 AND e.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'proposition', p.id, p.review_state, 0, p.text, NULL, NULL,
                    p.id, 2, 0, ''
             FROM propositions p
             WHERE p.case_id = ?1 AND p.review_state IN ('unreviewed','suggested')
             UNION ALL
             SELECT 'event', ev.id, ev.review_state, 0, ev.label, NULL, NULL,
                    ev.id, 2, 0, ''
             FROM events ev
             WHERE ev.case_id = ?1 AND ev.review_state IN ('unreviewed','suggested')
             ORDER BY 4 DESC, 1, 8, 9, 10, 11, 2",
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
        let mut statement = self.connection.prepare_cached(
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
        self.query_one(
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
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "advocacy item",
            id: id.to_owned(),
        })
    }

    /// Reads one version of an annotation and whether it is the current one.
    fn annotation_version(&self, case_id: &CaseId, id: &str) -> Result<WorkProductVersion> {
        self.query_one(
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
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "annotation",
            id: id.to_owned(),
        })
    }

    /// Returns the identifier of the version replacing this one, if any.
    fn superseding_id(&self, table: &str, column: &str, id: &str) -> Result<Option<String>> {
        let sql = format!("SELECT id FROM {table} WHERE {column} = ?1");
        self.query_one(&sql, [id], |row| row.get(0))
    }

    /// Records a person, organization, object, or place in the case.
    ///
    /// A name already in use is not refused. Whether two records name one thing
    /// is a question for a person — the `duplicate-entity` analyzer raises it
    /// and `possibly_same_person` holds the answer — and refusing the second
    /// write would answer it by merging, which is the one thing this kernel
    /// will not do on its own.
    pub fn record_entity(
        &mut self,
        case_id: &CaseId,
        proposal: &ProposedEntity,
    ) -> Result<AuthoredEntity> {
        self.require_case(case_id)?;
        let display_name = require_text(&proposal.display_name, "an entity must have a name")?;

        let id = match proposal.id.as_deref().map(str::trim) {
            Some(supplied) if !supplied.is_empty() => {
                self.refuse_existing_id(NodeKind::Entity, supplied)?;
                supplied.to_owned()
            }
            _ => Uuid::now_v7().to_string(),
        };
        let notes = trimmed(proposal.notes.as_deref());

        self.connection.execute(
            "INSERT INTO entities (id, case_id, kind, display_name, is_client, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                case_id.0,
                proposal.kind.as_str(),
                display_name,
                proposal.is_client,
                notes
            ],
        )?;

        Ok(AuthoredEntity {
            id,
            kind: proposal.kind.as_str().to_owned(),
            display_name,
            is_client: proposal.is_client,
            notes,
        })
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
        let element_case: Option<String> = self.query_one(
            "SELECT ch.case_id FROM elements el
             JOIN charges ch ON ch.id = el.charge_id
             WHERE el.id = ?1",
            [&proposal.element_id],
            |row| row.get(0),
        )?;
        match element_case {
            Some(found) if found == case_id.0 => {}
            Some(_) => {
                return Err(Error::WrongCase {
                    kind: "element",
                    id: proposal.element_id.clone(),
                });
            }
            None => {
                return Err(Error::NotFound {
                    kind: "element",
                    id: proposal.element_id.clone(),
                });
            }
        }
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
        self.query_one(
            "SELECT assessment FROM element_links
             WHERE element_id = ?1 AND proposition_id = ?2",
            params![element_id, proposition_id],
            |row| row.get(0),
        )
    }

    /// Refuses an identifier already in use in a table that is not a graph node.
    fn refuse_existing_row(&self, kind: &'static str, table: &str, id: &str) -> Result<()> {
        let sql = format!("SELECT 1 FROM {table} WHERE id = ?1");
        if self.exists(&sql, [id])? {
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
    ///
    /// A node that exists only in another case is a different error from a
    /// missing node: attaching it would share evidence across the docket.
    fn require_node(&self, case_id: &CaseId, node: &NodeRef) -> Result<()> {
        let found_case: Option<String> = self.query_one(
            &format!("SELECT case_id FROM {} WHERE id = ?1", node.kind.table()),
            [&node.id],
            |row| row.get(0),
        )?;
        match found_case {
            Some(found) if found == case_id.0 => Ok(()),
            Some(_) => Err(Error::WrongCase {
                kind: node.kind.as_str(),
                id: node.id.clone(),
            }),
            None => Err(Error::NotFound {
                kind: node.kind.as_str(),
                id: node.id.clone(),
            }),
        }
    }

    /// Refuses ingest pointers that name a record from another case.
    ///
    /// Production ownership is checked at insert. Speaker, attributed person,
    /// and parent content are not: they are ordinary foreign keys on id, so
    /// without this check a batch could attach another case's person or nest
    /// under another case's statement.
    fn refuse_cross_case_batch(&self, batch: &NormalizedBatch) -> Result<()> {
        let mut same_batch = HashSet::new();
        for source in &batch.sources {
            for segment in &source.segments {
                for content in &segment.content {
                    same_batch.insert(content.id.as_str());
                }
            }
        }

        for source in &batch.sources {
            for segment in &source.segments {
                for content in &segment.content {
                    if let Some(speaker) = content.speaker_entity_id.as_deref() {
                        self.require_node(
                            &batch.case_id,
                            &NodeRef::new(NodeKind::Entity, speaker),
                        )?;
                    }
                    if let Some(attributed) = content.attributed_to_entity_id.as_deref() {
                        self.require_node(
                            &batch.case_id,
                            &NodeRef::new(NodeKind::Entity, attributed),
                        )?;
                    }
                    if let Some(parent) = content.parent_content_id.as_deref()
                        && !same_batch.contains(parent)
                    {
                        self.require_node(
                            &batch.case_id,
                            &NodeRef::new(NodeKind::Content, parent),
                        )?;
                    }
                }
            }
        }
        Ok(())
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
        let mut statement = self.connection.prepare_cached(
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
        let raw: Option<String> =
            self.query_one(&sql, params![target_id, case_id.0], |row| row.get(0))?;
        let Some(raw) = raw else {
            let elsewhere = self.exists(
                &format!("SELECT 1 FROM {} WHERE id = ?1", target.table()),
                [target_id],
            )?;
            return Err(if elsewhere {
                Error::WrongCase {
                    kind: "review target",
                    id: target_id.to_owned(),
                }
            } else {
                Error::NotFound {
                    kind: "review target",
                    id: target_id.to_owned(),
                }
            });
        };
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
        self.query_one(sql, [target_id], |row| row.get(0))
    }

    fn require_case(&self, case_id: &CaseId) -> Result<()> {
        if self.exists("SELECT 1 FROM cases WHERE id = ?1", [&case_id.0])? {
            return Ok(());
        }
        Err(Error::NotFound {
            kind: "case",
            id: case_id.0.clone(),
        })
    }

    fn case_name(&self, case_id: &CaseId) -> Result<String> {
        self.query_one(
            "SELECT name FROM cases WHERE id = ?1",
            [&case_id.0],
            |row| row.get(0),
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "case",
            id: case_id.0.clone(),
        })
    }

    /// Runs a query expected to match at most one row, through the cache.
    ///
    /// Nearly every mutation checks a case, an endpoint, or an identifier
    /// before it writes. Compiling those checks afresh each time cost more than
    /// running them.
    fn query_one<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<Option<T>>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.connection
            .prepare_cached(sql)?
            .query_row(params, map)
            .optional()
            .map_err(Into::into)
    }

    /// Answers whether a row matching the query exists.
    fn exists<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<bool> {
        Ok(self.query_one(sql, params, |_| Ok(()))?.is_some())
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

/// Returns pairs of people in a case who may be one person.
///
/// The rule is deliberately narrow, and comparison happens here rather than in
/// SQL so a defender can read it. Two people are a candidate when every part of
/// one name appears in the other after folding case and punctuation — `Patel`
/// and `Jordan Patel`, `Jordan Patel` and `Patel, Jordan`. `J. Patel` does not
/// match `Jordan Patel`: expanding an initial is a guess, and the same guess
/// would tie `J. Patel` to `Jane Patel` just as confidently. Anything looser
/// invents relationships between strangers who share a common surname, and a
/// tool that cries duplicate gets ignored precisely when it is right.
///
/// Only people are compared. `possibly_same_person` says what it means, and
/// whether two vehicles are one vehicle is a different question with different
/// evidence.
impl Store {
    fn duplicate_people(&self, case_id: &CaseId) -> Result<Vec<Candidate>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT id, display_name FROM entities
             WHERE case_id = ?1 AND kind = 'person' ORDER BY id",
        )?;
        let named = statement
            .query_map([&case_id.0], |row| {
                let id: String = row.get(0)?;
                let name: String = row.get(1)?;
                let tokens = name_tokens(&name);
                Ok((id, name, tokens))
            })?
            .collect::<std::result::Result<Vec<(String, String, Vec<String>)>, _>>()?;

        let mut candidates = Vec::new();
        for (index, (id, name, tokens)) in named.iter().enumerate() {
            for (other_id, other_name, other_tokens) in named.iter().skip(index + 1) {
                if tokens.is_empty() || other_tokens.is_empty() {
                    continue;
                }
                let contained = tokens.iter().all(|token| other_tokens.contains(token))
                    || other_tokens.iter().all(|token| tokens.contains(token));
                if !contained {
                    continue;
                }
                candidates.push(Candidate {
                    kind: NodeKind::Entity,
                    relation: EdgeKind::PossiblySamePerson,
                    from: id.clone(),
                    to: other_id.clone(),
                    rationale: format!(
                        "`{name}` and `{other_name}` may name one person: every part of one \
                         name appears in the other. Proposed as a question only — the records \
                         are not merged, and mentions stay attached to the record they were \
                         written against."
                    ),
                });
            }
        }
        Ok(candidates)
    }
}

/// Splits a display name into comparable parts.
///
/// Case and punctuation are folded so `J. Patel` and `j patel` agree. Initials
/// are kept as-is rather than expanded: guessing that `J.` is `Jordan` is the
/// kind of inference that belongs to the person reviewing, not to the rule.
fn name_tokens(name: &str) -> Vec<String> {
    name.split(|character: char| !character.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// One pair an analyzer found, before anything is written.
struct Candidate {
    /// Node type of both endpoints; analyzers relate like to like.
    kind: NodeKind,
    /// The relationship being proposed.
    relation: EdgeKind,
    /// Lower identifier of the pair, so a rerun proposes the same direction.
    from: String,
    /// Higher identifier of the pair.
    to: String,
    /// The deterministic reason, stated in full so a defender can argue with it.
    rationale: String,
}

/// Reports a query `SQLite` could not read as a search rather than as a failure.
///
/// FTS5 has its own syntax, and a stray quote or a bare `AND` is a typo, not a
/// broken database. Saying so — and saying what was typed — is the difference
/// between a person fixing their query and a person believing the tool broke.
fn unreadable_query(query: &str, error: rusqlite::Error) -> Error {
    match &error {
        rusqlite::Error::SqliteFailure(_, Some(message)) if message.contains("fts5") => {
            Error::InvalidSearch(format!("`{query}` is not a readable search: {message}"))
        }
        _ => Error::Database(error),
    }
}

/// Returns whether a stored state is one import produced rather than a person.
///
/// An unrecognized value counts as unreviewed: for a caller deciding what to
/// warn about, treating something unreadable as unchecked is the safe reading.
fn is_intake_state(value: &str) -> bool {
    ReviewState::from_db(value).is_none_or(ReviewState::is_intake_state)
}

/// Reads a calendar key only from an ISO-like normalized timestamp.
fn normalized_date(value: &str) -> Option<&str> {
    let date = value.get(..10)?;
    let bytes = date.as_bytes();
    (bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit()))
    .then_some(date)
}

/// Conservative location equality: case and repeated whitespace only.
fn conservative_location_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn has_text(value: Option<&str>) -> bool {
    value.is_some_and(|text| !text.trim().is_empty())
}

fn distinct_sources(entries: &[CollationEntry]) -> u32 {
    entries
        .iter()
        .map(|entry| entry.source_id.as_str())
        .collect::<HashSet<_>>()
        .len()
        .try_into()
        .unwrap_or(u32::MAX)
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

/// One stored keyframe vector plus the still and original it points at.
struct IndexedRow {
    source_id: String,
    vector: Vec<u8>,
    dim: i64,
    still_sha256: String,
    source: String,
    sha256: String,
    locator: String,
    review_state: String,
    machine_generated: bool,
    content_id: String,
}

fn validate_embedding_vector(item: &IndexedKeyframe) -> Result<()> {
    if item.source_id.trim().is_empty() {
        return Err(Error::InvalidIndex(
            "an embedding needs a still source id".to_owned(),
        ));
    }
    if item.model.trim().is_empty() {
        return Err(Error::InvalidIndex(
            "an embedding needs a model name".to_owned(),
        ));
    }
    if item.extractor.trim().is_empty() || item.version.trim().is_empty() {
        return Err(Error::InvalidIndex(format!(
            "still `{}` is missing extractor provenance",
            item.source_id
        )));
    }
    vector_error(&item.vector).map_err(Error::InvalidIndex)
}

fn validate_query_vector(query: &[f32]) -> Result<()> {
    vector_error(query).map_err(Error::InvalidSearch)
}

fn vector_error(values: &[f32]) -> std::result::Result<(), String> {
    if values.is_empty() {
        return Err("a vector needs at least one dimension".to_owned());
    }
    let mut norm = 0.0_f64;
    for value in values {
        if !value.is_finite() {
            return Err("a vector cannot contain non-finite coordinates".to_owned());
        }
        norm += f64::from(*value) * f64::from(*value);
    }
    if norm == 0.0 {
        return Err("a zero vector cannot be compared".to_owned());
    }
    Ok(())
}

fn encode_vector(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len().saturating_mul(4));
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_vector(bytes: &[u8], dim: i64) -> Result<Vec<f32>> {
    let expected = usize::try_from(dim)
        .ok()
        .and_then(|count| count.checked_mul(4))
        .ok_or_else(|| Error::InvalidSearch("stored vector dimension is unreadable".to_owned()))?;
    if bytes.len() != expected {
        return Err(Error::InvalidSearch(
            "stored vector length does not match its dimension".to_owned(),
        ));
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect())
}

fn cosine(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.len() != right.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut left_norm = 0.0_f64;
    let mut right_norm = 0.0_f64;
    for (a, b) in left.iter().zip(right) {
        let a = f64::from(*a);
        let b = f64::from(*b);
        if !a.is_finite() || !b.is_finite() {
            return None;
        }
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    let denom = left_norm.sqrt() * right_norm.sqrt();
    if denom == 0.0 {
        return None;
    }
    Some(dot / denom)
}

fn validate_batch(batch: &NormalizedBatch) -> Result<()> {
    // A later pass may carry nothing but relationships between originals
    // imported on different days, so a batch without sources is empty only
    // when it proposes nothing at all.
    if batch.sources.is_empty() && batch.edges.is_empty() {
        return Err(Error::InvalidFixture(
            "a normalized batch must contain at least one source or edge".to_owned(),
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
    validate_edges(batch)
}

/// Checks what an adapter's proposed relationships must satisfy on their face.
///
/// An adapter may say how records sit relative to each other and never what
/// they establish, so only the structural relations are admitted, and only in
/// the state a machine is allowed to write. Everything decidable from the batch
/// document alone is decided here; whether the endpoints exist, belong to this
/// case, and are not already related is settled inside the import transaction,
/// once the batch's own rows are in.
fn validate_edges(batch: &NormalizedBatch) -> Result<()> {
    let mut seen = HashSet::new();
    for edge in &batch.edges {
        if !seen.insert(edge.id.as_str()) {
            return Err(Error::AlreadyExists {
                kind: NodeKind::Edge.as_str(),
                id: edge.id.clone(),
            });
        }
        if !matches!(
            edge.relation,
            EdgeKind::TemporallyOverlaps | EdgeKind::DerivedFrom | EdgeKind::RefersTo
        ) {
            return Err(Error::InvalidFixture(format!(
                "edge `{}` proposes `{}`; import admits only the structural relations \
                 temporally_overlaps, derived_from and refers_to. An adapter may say how \
                 records sit relative to each other, never what they establish",
                edge.id,
                edge.relation.as_str()
            )));
        }
        for (endpoint, kind) in [("from", edge.from_kind), ("to", edge.to_kind)] {
            if !matches!(kind, NodeKind::Source | NodeKind::Content) {
                return Err(Error::InvalidFixture(format!(
                    "edge `{}` names a `{}` as its {endpoint} endpoint; import relates only \
                     sources and content",
                    edge.id,
                    kind.as_str()
                )));
            }
        }
        if edge.from_kind == edge.to_kind && edge.from_id == edge.to_id {
            return Err(Error::InvalidFixture(format!(
                "edge `{}` cannot stand in a relationship to itself",
                edge.id
            )));
        }
        if edge.rationale.trim().is_empty() {
            return Err(Error::InvalidFixture(format!(
                "edge `{}` requires a written rationale; a relationship has no original of \
                 its own to check it against",
                edge.id
            )));
        }
        let provenance = &edge.extraction;
        if !provenance.machine_generated {
            return Err(Error::InvalidFixture(format!(
                "edge `{}` is attributed to a person; import proposes a relationship, \
                 a named person authors one",
                edge.id
            )));
        }
        if provenance.review_state != ReviewState::Suggested {
            return Err(Error::InvalidFixture(format!(
                "machine edge `{}` must enter as suggested",
                edge.id
            )));
        }
    }
    Ok(())
}

fn to_sql_integer(value: u64, label: &str) -> Result<i64> {
    i64::try_from(value)
        .map_err(|_| Error::InvalidFixture(format!("{label} exceeds SQLite integer range")))
}

fn verify_file_identity(path: &Path, expected_hash: &str, expected_length: u64) -> Result<()> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let actual_length = file.metadata()?.len();
    if actual_length != expected_length {
        return Err(Error::InvalidIntake(format!(
            "{} is {actual_length} bytes, expected {expected_length}",
            path.display()
        )));
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual_hash = format!("{:x}", digest.finalize());
    if !actual_hash.eq_ignore_ascii_case(expected_hash) {
        return Err(Error::InvalidIntake(format!(
            "{} does not match its stored SHA-256",
            path.display()
        )));
    }
    Ok(())
}

fn intake_profile_labels(profile: &AdapterProfile) -> (&'static str, &'static str) {
    match profile {
        AdapterProfile::Document { .. } => ("document", "search"),
        AdapterProfile::Audio { .. } => ("audio", "transcript"),
        AdapterProfile::Video {
            tier: VideoTier::Tier1,
            ..
        } => ("video", "tier1"),
        AdapterProfile::Video {
            tier: VideoTier::Overnight,
            ..
        } => ("video", "overnight"),
    }
}

fn map_intake_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<IntakeJob> {
    let state_text: String = row.get(12)?;
    let state = IntakeJobState::parse(&state_text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            12,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("unknown intake job state `{state_text}`"),
            )),
        )
    })?;
    Ok(IntakeJob {
        id: row.get(0)?,
        case_id: CaseId(row.get(1)?),
        production_id: row.get(2)?,
        source_id: row.get(3)?,
        modality: row.get(4)?,
        profile: row.get(5)?,
        original_path: std::path::PathBuf::from(row.get::<_, String>(6)?),
        original_sha256: row.get(7)?,
        original_byte_length: row_u64(row, 8)?,
        logical_name: row.get(9)?,
        request_json: row.get(10)?,
        artifact_dir: std::path::PathBuf::from(row.get::<_, String>(11)?),
        state,
        attempt: row.get(13)?,
        stage: row.get(14)?,
        progress_completed: row_optional_u64(row, 15)?,
        progress_total: row_optional_u64(row, 16)?,
        message: row.get(17)?,
        error: row.get(18)?,
        created_at: row.get(19)?,
        started_at: row.get(20)?,
        finished_at: row.get(21)?,
    })
}

fn row_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn row_optional_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
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

    /// A keyframe vector that names another case's still is refused by the
    /// schema, not only by the store API.
    #[test]
    fn the_schema_refuses_a_cross_case_keyframe_embedding() {
        let store = both_cases();
        let error = store
            .connection
            .execute(
                "INSERT INTO keyframe_embeddings
                   (source_id, case_id, model, dim, vector, extractor, extractor_version)
                 VALUES ('hr-src-camera', 'case-vehicle-stop-001', 'clip', 1, X'0000803f',
                         'keyframe_embed', 'from-json')",
                [],
            )
            .expect_err("a still from another case must be refused");
        assert!(error.to_string().contains("same case"), "{error}");
    }

    /// A speaker, parent, or element mapping that names another case is
    /// refused by the schema, not only by the store API.
    #[test]
    fn the_schema_refuses_a_cross_case_attachment() {
        let store = both_cases();
        let speaker = store
            .connection
            .execute(
                "UPDATE content SET speaker_entity_id = 'person-chen'
                 WHERE id = 'hr-content-911-injury'",
                [],
            )
            .expect_err("a speaker from another case must be refused");
        assert!(speaker.to_string().contains("same case"), "{speaker}");

        let parent = store
            .connection
            .execute(
                "UPDATE content SET parent_content_id = 'content-report-consent'
                 WHERE id = 'hr-content-911-injury'",
                [],
            )
            .expect_err("parent content from another case must be refused");
        assert!(parent.to_string().contains("same case"), "{parent}");
    }

    /// A mapping that names another case's proposition is refused at write
    /// time. The views still constrain `propositions.case_id` themselves, so a
    /// future writer cannot make one case's matrix display the other's claim
    /// even if the trigger were ever removed.
    #[test]
    fn a_cross_case_element_mapping_never_surfaces_in_a_view() {
        let store = both_cases();
        let error = store
            .connection
            .execute(
                "INSERT INTO element_links
                   (id, element_id, proposition_id, assessment, created_by)
                 VALUES ('smuggled', 'hr-el-fi-drive', 'prop-consent', 'supports', 'nobody')",
                [],
            )
            .expect_err("the schema refuses a cross-case element mapping");
        assert!(error.to_string().contains("one case"), "{error}");

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

    /// The version stamp lets an up-to-date database skip re-running the
    /// migrations, so the migrations must still reach a database that predates
    /// the stamp — every database written before this existed reads as version
    /// zero, and one of them arriving unmigrated would be silent corruption.
    #[test]
    fn a_database_predating_the_version_stamp_is_still_migrated() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("case.sqlite");

        let store = Store::open(&path).expect("create");
        store
            .connection
            .execute_batch("PRAGMA user_version = 0; DROP INDEX idx_entities_case_kind;")
            .expect("rewind to an unstamped database");
        drop(store);

        let store = Store::open(&path).expect("reopen");
        let restored: bool = store
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_entities_case_kind'",
                [],
                |row| Ok(row.get::<_, i64>(0)? == 1),
            )
            .expect("index lookup");
        assert!(
            restored,
            "an unstamped database must receive every migration"
        );

        let stamped: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(stamped, super::SCHEMA_VERSION);
    }

    /// The search index is added to databases that already hold evidence, so the
    /// migration has to backfill rather than only catch what arrives next. An
    /// excerpt that existed before the index did must still be findable.
    #[test]
    fn content_written_before_the_search_index_is_still_findable() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("case.sqlite");

        let mut store = Store::open(&path).expect("create");
        let case = crate::DemoFixture::HitAndRun
            .seed(&mut store)
            .expect("seed");
        store
            .connection
            .execute_batch("DROP TABLE content_search; PRAGMA user_version = 0;")
            .expect("rewind to a database with no search index");
        drop(store);

        let store = Store::open(&path).expect("reopen");
        assert!(
            !store
                .search(&case, "hatchback", 25)
                .expect("search")
                .is_empty(),
            "evidence predating the index must be backfilled into it"
        );
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
