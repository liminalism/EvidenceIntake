//! The office layer's `SQLite` store.
//!
//! The only module in this crate that touches a database, following the
//! evidence kernel's store discipline exactly: strict tables, numbered
//! idempotent migrations behind a version stamp, `prepare_cached` on every
//! query, and append-only trails wherever authorship matters.
//!
//! It has no connection to the evidence database and no way to open one. That
//! is what makes the privileged boundary structural: advocacy items,
//! annotations and decision briefs are not filtered out of these reads, they
//! are unreachable from this crate.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::authoring::{
    ProposedAppearance, ProposedAssignment, ProposedClient, ProposedClientContact, ProposedCourt,
    ProposedDeadline, ProposedIdentityLinkDecision, ProposedMatter, ProposedNote, ProposedUser,
};
use crate::civil_date::{CivilDate, require_time};
use crate::error::{Error, Result};
use crate::model::{
    AssignmentRole, ContactKind, CustodyState, IdentityLinkState, MatterStatus, MentionTag,
    NoteScope, OfferState, Sex,
};
use crate::views::{
    AppearanceSummary, ClientContactRow, ClientProfile, DeadlineRow, DocketDay, DocketEntry,
    DocketMatterLine, MatterAssignmentRow, MatterLinkRow, MatterProfile, MatterSummary, NoteEntry,
    NoteHistory, OfficeSearchHit, PossiblePerson, UpcomingDeadlines,
};

/// Number of migrations applied by [`OfficeStore::migrate`].
///
/// Recorded in `PRAGMA user_version` so an already-current database can skip
/// re-executing the whole schema on every open. The migrations stay additive
/// and re-runnable regardless: a database at any earlier version — including
/// one written before this stamp existed, which reads as zero — runs all of
/// them again.
const SCHEMA_VERSION: i64 = 6;

/// Prepared statements held per connection.
///
/// The docket read models run the same handful of queries once per setting, so
/// the cache has to be large enough to hold the whole working set; an LRU too
/// small recompiles on every call and costs more than no cache at all.
const STATEMENT_CACHE_CAPACITY: usize = 64;

/// A local `SQLite` office store.
///
/// Foreign keys are enabled for every connection. Nothing in this schema holds
/// evidence: the single reference across the boundary is a matter's
/// `evidence_case_id`, a bare identifier with no foreign key.
pub struct OfficeStore {
    connection: Connection,
}

impl OfficeStore {
    /// Opens or creates an office store and applies all embedded migrations.
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
    /// a database at any earlier version.
    fn migrate(connection: &Connection) -> Result<()> {
        connection.execute_batch(include_str!("../migrations/0001_office_core.sql"))?;
        connection.execute_batch(include_str!("../migrations/0002_calendar.sql"))?;
        connection.execute_batch(include_str!("../migrations/0003_notes.sql"))?;
        connection.execute_batch(include_str!("../migrations/0004_identity_links.sql"))?;
        connection.execute_batch(include_str!("../migrations/0005_search.sql"))?;
        // Retrofitted onto an existing table, so nullable by necessity:
        // SQLite cannot add a NOT NULL or a CHECK to a table that already has
        // rows. 0006 enforces the vocabulary forward with triggers instead.
        Self::add_column_if_missing(connection, "clients", "sex", "TEXT")?;
        Self::add_column_if_missing(connection, "clients", "preferred_language", "TEXT")?;
        connection.execute_batch(include_str!("../migrations/0006_client_demographics.sql"))?;
        Ok(())
    }

    /// Adds a column only when it is absent, so migrations stay re-runnable.
    ///
    /// `SQLite` has no `IF NOT EXISTS` form of `ALTER TABLE ADD COLUMN` and
    /// cannot retrofit a `NOT NULL` constraint, so a column added this way is
    /// nullable and its guarantee is enforced forward by a trigger. Prefer a
    /// new table over this when the choice exists. First used at schema v6,
    /// which retrofits the two demographic columns onto `clients`.
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

    /// Today's local date, in the stored `YYYY-MM-DD` form.
    pub fn today(&self) -> Result<String> {
        Ok(crate::civil_date::today(&self.connection)?)
    }

    // ----- helpers ---------------------------------------------------------

    /// Runs a query expected to match at most one row, through the cache.
    fn query_one<T, P, F>(&self, sql: &str, parameters: P, map: F) -> Result<Option<T>>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.connection
            .prepare_cached(sql)?
            .query_row(parameters, map)
            .optional()
            .map_err(Into::into)
    }

    /// Answers whether a row matching the query exists.
    fn exists<P: rusqlite::Params>(&self, sql: &str, parameters: P) -> Result<bool> {
        Ok(self.query_one(sql, parameters, |_| Ok(()))?.is_some())
    }

    fn require_user(&self, user_id: &str) -> Result<String> {
        self.query_one(
            "SELECT display_name FROM users WHERE id = ?1",
            params![user_id],
            |row| row.get(0),
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "user",
            id: user_id.to_owned(),
        })
    }

    fn require_client(&self, client_id: &str) -> Result<String> {
        self.query_one(
            "SELECT display_name FROM clients WHERE id = ?1",
            params![client_id],
            |row| row.get(0),
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "client",
            id: client_id.to_owned(),
        })
    }

    /// Returns the matter's owning client, which is how every cross-record
    /// write checks that it is staying inside one person's file.
    fn matter_client(&self, matter_id: &str) -> Result<String> {
        self.query_one(
            "SELECT client_id FROM matters WHERE id = ?1",
            params![matter_id],
            |row| row.get(0),
        )?
        .ok_or_else(|| Error::NotFound {
            kind: "matter",
            id: matter_id.to_owned(),
        })
    }

    fn refuse_existing(&self, kind: &'static str, table: &str, id: &str) -> Result<()> {
        if self.exists(&format!("SELECT 1 FROM {table} WHERE id = ?1"), params![id])? {
            return Err(Error::AlreadyExists {
                kind,
                id: id.to_owned(),
            });
        }
        Ok(())
    }

    // ----- users -----------------------------------------------------------

    /// Records somebody who can author office records.
    ///
    /// Enforces that a user has a name; the role vocabulary is enforced by the
    /// schema.
    pub fn create_user(&self, proposal: &ProposedUser) -> Result<String> {
        let name = require_text(&proposal.display_name, "a user must have a name")?;
        let role = require_text(&proposal.role, "a user must have a role")?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("user", "users", &id)?;
        self.connection
            .prepare_cached(
                "INSERT INTO users (id, display_name, role, email) VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![id, name, role, optional(proposal.email.as_deref())])?;
        Ok(id)
    }

    /// Finds a user by the name they are known by, or records a new one.
    ///
    /// The native workspace already asks who is acting before it lets anybody
    /// write; this is how that name becomes the real identifier every office
    /// row carries, without making a person maintain a roster first.
    pub fn user_named(&self, display_name: &str, role: &str) -> Result<String> {
        let name = require_text(display_name, "a user must have a name")?;
        if let Some(id) = self.query_one(
            "SELECT id FROM users WHERE display_name = ?1",
            params![name],
            |row| row.get::<_, String>(0),
        )? {
            return Ok(id);
        }
        self.create_user(&ProposedUser {
            id: None,
            display_name: name,
            role: role.to_owned(),
            email: None,
        })
    }

    /// Lists users, active ones first, then by name.
    pub fn users(&self) -> Result<Vec<(String, String, String)>> {
        self.collect(
            "SELECT id, display_name, role FROM users ORDER BY active DESC, display_name, id",
            params![],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
    }

    // ----- clients ---------------------------------------------------------

    /// Opens a client record with its aliases and contacts.
    ///
    /// Enforces that the record names a real author, that a supplied identifier
    /// is not already taken, and that a date of birth is a real calendar date
    /// rather than merely the right shape. Nothing here merges: a client who
    /// resembles an existing one is still written, and the resemblance is a
    /// question for [`Self::possible_client_duplicates`] to raise.
    pub fn create_client(&mut self, proposal: &ProposedClient) -> Result<ClientProfile> {
        self.require_user(&proposal.author_user_id)?;
        let name = require_text(&proposal.display_name, "a client must have a name")?;
        let birth = match proposal.date_of_birth.as_deref() {
            Some(text) if !text.trim().is_empty() => {
                Some(CivilDate::require(text.trim())?.to_text())
            }
            _ => None,
        };
        let language = optional_language(proposal.preferred_language.as_deref())?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("client", "clients", &id)?;

        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO clients
                 (id, display_name, date_of_birth, sex, preferred_language, notes, author_user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                name,
                birth,
                proposal.sex.map(Sex::as_str),
                language,
                optional(proposal.notes.as_deref()),
                proposal.author_user_id
            ],
        )?;
        for alias in &proposal.aliases {
            let alias = alias.trim();
            if alias.is_empty() {
                continue;
            }
            transaction.execute(
                "INSERT OR IGNORE INTO client_aliases (id, client_id, alias) VALUES (?1, ?2, ?3)",
                params![Uuid::now_v7().to_string(), id, alias],
            )?;
        }
        for contact in &proposal.contacts {
            insert_contact(&transaction, &id, contact)?;
        }
        transaction.commit()?;
        self.client_profile(&id)
    }

    /// Adds another name a client goes by.
    pub fn add_client_alias(&self, client_id: &str, alias: &str) -> Result<String> {
        self.require_client(client_id)?;
        let alias = require_text(alias, "an alias must say something")?;
        let id = Uuid::now_v7().to_string();
        self.connection
            .prepare_cached(
                "INSERT INTO client_aliases (id, client_id, alias) VALUES (?1, ?2, ?3)",
            )?
            .execute(params![id, client_id, alias])?;
        Ok(id)
    }

    /// Adds a way of reaching a client.
    ///
    /// A dialable value is stored twice: once as a person wrote it and once
    /// reduced to bare digits, because the search tokenizer splits punctuation
    /// and would otherwise not match an unpunctuated query.
    pub fn add_client_contact(
        &self,
        client_id: &str,
        proposal: &ProposedClientContact,
    ) -> Result<String> {
        self.require_client(client_id)?;
        insert_contact(&self.connection, client_id, proposal)
    }

    /// Everything the office knows about one person, across every matter.
    pub fn client_profile(&self, client_id: &str) -> Result<ClientProfile> {
        let row = self
            .query_one(
                "SELECT c.display_name, c.date_of_birth, c.sex, c.preferred_language, c.notes,
                        u.display_name, c.created_at
                 FROM clients c JOIN users u ON u.id = c.author_user_id
                 WHERE c.id = ?1",
                params![client_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                },
            )?
            .ok_or_else(|| Error::NotFound {
                kind: "client",
                id: client_id.to_owned(),
            })?;

        let aliases = self.collect(
            "SELECT alias FROM client_aliases WHERE client_id = ?1 ORDER BY alias",
            params![client_id],
            |row| row.get(0),
        )?;
        let contacts = self.collect(
            "SELECT id, kind, value, label, is_primary FROM client_contacts
             WHERE client_id = ?1 ORDER BY is_primary DESC, kind, value",
            params![client_id],
            |row| {
                Ok(ClientContactRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    value: row.get(2)?,
                    label: row.get(3)?,
                    is_primary: row.get::<_, i64>(4)? == 1,
                })
            },
        )?;
        let matters = self.matter_summaries("m.client_id = ?1", params![client_id])?;
        let next_setting = self.next_setting_for_client(client_id)?;

        Ok(ClientProfile {
            id: client_id.to_owned(),
            display_name: row.0,
            date_of_birth: row.1,
            sex: row.2,
            preferred_language: row.3,
            notes: row.4,
            aliases,
            contacts,
            matters,
            next_setting,
            opened_by: row.5,
            created_at: row.6,
        })
    }

    /// Languages the office already records, most common first, for a chooser.
    pub fn client_languages(&self) -> Result<Vec<String>> {
        self.collect(
            "SELECT preferred_language FROM clients
             WHERE preferred_language IS NOT NULL
             GROUP BY preferred_language
             ORDER BY count(*) DESC, preferred_language",
            params![],
            |row| row.get(0),
        )
    }

    /// Lists clients by name, for a chooser.
    pub fn clients(&self) -> Result<Vec<(String, String)>> {
        self.collect(
            "SELECT id, display_name FROM clients ORDER BY display_name, id",
            params![],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    // ----- matters ---------------------------------------------------------

    /// Opens a matter on an existing client.
    ///
    /// Enforces a real author and a real client, and validates every date
    /// before it reaches SQL — the schema's `GLOB` guards check the shape of a
    /// date, not whether the day exists.
    pub fn open_matter(&self, proposal: &ProposedMatter) -> Result<String> {
        self.require_user(&proposal.author_user_id)?;
        self.require_client(&proposal.client_id)?;
        let caption = require_text(&proposal.caption, "a matter must have a caption")?;
        let opened_on = optional_date(proposal.opened_on.as_deref())?;
        let last_contact = optional_date(proposal.last_contact_on.as_deref())?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("matter", "matters", &id)?;

        self.connection
            .prepare_cached(
                "INSERT INTO matters
                   (id, client_id, caption, court_number, court_id, status, custody_state,
                    offer_state, offer_summary, charge_summary, opened_on, last_contact_on,
                    evidence_case_id, author_user_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            )?
            .execute(params![
                id,
                proposal.client_id,
                caption,
                optional(proposal.court_number.as_deref()),
                optional(proposal.court_id.as_deref()),
                proposal.status.unwrap_or(MatterStatus::Open).as_str(),
                proposal
                    .custody_state
                    .unwrap_or(CustodyState::Unknown)
                    .as_str(),
                proposal.offer_state.unwrap_or(OfferState::None).as_str(),
                optional(proposal.offer_summary.as_deref()),
                optional(proposal.charge_summary.as_deref()),
                opened_on,
                last_contact,
                optional(proposal.evidence_case_id.as_deref()),
                proposal.author_user_id,
            ])?;
        Ok(id)
    }

    /// Moves a matter to another office status.
    pub fn update_matter_status(&self, matter_id: &str, status: MatterStatus) -> Result<()> {
        self.update_matter_column(matter_id, "status", status.as_str())
    }

    /// Records where the client is.
    pub fn update_custody_state(&self, matter_id: &str, custody: CustodyState) -> Result<()> {
        self.update_matter_column(matter_id, "custody_state", custody.as_str())
    }

    /// Records where negotiation stands, and the offer in the defender's words.
    pub fn update_offer_state(
        &self,
        matter_id: &str,
        offer: OfferState,
        summary: Option<&str>,
    ) -> Result<()> {
        self.matter_client(matter_id)?;
        self.connection
            .prepare_cached(
                "UPDATE matters SET offer_state = ?2, offer_summary = COALESCE(?3, offer_summary)
                 WHERE id = ?1",
            )?
            .execute(params![matter_id, offer.as_str(), optional(summary)])?;
        Ok(())
    }

    /// Records that the client was spoken to on a date.
    pub fn record_client_contact(&self, matter_id: &str, on: &str) -> Result<()> {
        let on = CivilDate::require(on)?.to_text();
        self.update_matter_column(matter_id, "last_contact_on", &on)
    }

    /// Points a matter at the kernel case holding its discovery.
    ///
    /// The identifier is stored as given and never resolved here; this crate
    /// has no connection to the evidence database.
    pub fn link_evidence_case(&self, matter_id: &str, evidence_case_id: &str) -> Result<()> {
        let case = require_text(evidence_case_id, "an evidence case must be identified")?;
        self.update_matter_column(matter_id, "evidence_case_id", &case)
    }

    fn update_matter_column(&self, matter_id: &str, column: &str, value: &str) -> Result<()> {
        self.matter_client(matter_id)?;
        self.connection
            .prepare_cached(&format!("UPDATE matters SET {column} = ?2 WHERE id = ?1"))?
            .execute(params![matter_id, value])?;
        Ok(())
    }

    /// Ties two matters of the same client together.
    ///
    /// Enforces that both belong to one person — related matters are the office
    /// concept behind one setting spanning several cases, and a link across two
    /// clients would put somebody else's case on a docket row.
    pub fn link_matters(
        &self,
        matter_id: &str,
        related_matter_id: &str,
        relation: &str,
    ) -> Result<String> {
        let owner = self.matter_client(matter_id)?;
        let other = self.matter_client(related_matter_id)?;
        if owner != other {
            return Err(Error::WrongClient {
                kind: "matter",
                id: related_matter_id.to_owned(),
            });
        }
        let id = Uuid::now_v7().to_string();
        self.connection
            .prepare_cached(
                "INSERT OR IGNORE INTO matter_links (id, matter_id, related_matter_id, relation)
                 VALUES (?1, ?2, ?3, ?4)",
            )?
            .execute(params![id, matter_id, related_matter_id, relation])?;
        Ok(id)
    }

    /// Staffs somebody onto a matter.
    pub fn assign_to_matter(&self, proposal: &ProposedAssignment) -> Result<String> {
        self.matter_client(&proposal.matter_id)?;
        self.require_user(&proposal.user_id)?;
        self.require_user(&proposal.assigned_by_user_id)?;
        let id = Uuid::now_v7().to_string();
        self.connection
            .prepare_cached(
                "INSERT OR IGNORE INTO matter_assignments
                   (id, matter_id, user_id, role, assigned_by_user_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                id,
                proposal.matter_id,
                proposal.user_id,
                proposal.role.as_str(),
                proposal.assigned_by_user_id
            ])?;
        Ok(id)
    }

    /// One matter and everything hanging off it.
    pub fn matter_profile(&self, matter_id: &str) -> Result<MatterProfile> {
        let head = self
            .query_one(
                "SELECT m.client_id, c.display_name, m.caption, m.court_number, ct.name,
                        m.status, m.custody_state, m.offer_state, m.offer_summary,
                        m.charge_summary, m.opened_on, m.last_contact_on, m.evidence_case_id,
                        u.display_name
                 FROM matters m
                 JOIN clients c ON c.id = m.client_id
                 JOIN users u ON u.id = m.author_user_id
                 LEFT JOIN courts ct ON ct.id = m.court_id
                 WHERE m.id = ?1",
                params![matter_id],
                |row| {
                    Ok(MatterProfile {
                        id: String::new(),
                        client_id: row.get(0)?,
                        client: row.get(1)?,
                        caption: row.get(2)?,
                        court_number: row.get(3)?,
                        court: row.get(4)?,
                        status: row.get(5)?,
                        custody_state: row.get(6)?,
                        offer_state: row.get(7)?,
                        offer_summary: row.get(8)?,
                        charge_summary: row.get(9)?,
                        opened_on: row.get(10)?,
                        last_contact_on: row.get(11)?,
                        evidence_case_id: row.get(12)?,
                        opened_by: row.get(13)?,
                        assignments: Vec::new(),
                        related: Vec::new(),
                        settings: Vec::new(),
                        deadlines: Vec::new(),
                        notes: Vec::new(),
                    })
                },
            )?
            .ok_or_else(|| Error::NotFound {
                kind: "matter",
                id: matter_id.to_owned(),
            })?;

        let assignments = self.collect(
            "SELECT u.display_name, a.role, b.display_name, a.assigned_at
             FROM matter_assignments a
             JOIN users u ON u.id = a.user_id
             JOIN users b ON b.id = a.assigned_by_user_id
             WHERE a.matter_id = ?1
             ORDER BY a.role, u.display_name",
            params![matter_id],
            |row| {
                Ok(MatterAssignmentRow {
                    user: row.get(0)?,
                    role: row.get(1)?,
                    assigned_by: row.get(2)?,
                    assigned_at: row.get(3)?,
                })
            },
        )?;
        let related = self.collect(
            "SELECT m.id, m.caption, m.court_number, l.relation
             FROM matter_links l JOIN matters m ON m.id = l.related_matter_id
             WHERE l.matter_id = ?1
             ORDER BY m.caption, m.id",
            params![matter_id],
            |row| {
                Ok(MatterLinkRow {
                    matter_id: row.get(0)?,
                    caption: row.get(1)?,
                    court_number: row.get(2)?,
                    relation: row.get(3)?,
                })
            },
        )?;
        let settings = self.collect(
            "SELECT a.id, a.appearance_date, a.appearance_time, a.appearance_type, ct.name,
                    (SELECT count(*) FROM appearance_matters WHERE appearance_id = a.id),
                    a.outcome
             FROM appearances a
             JOIN appearance_matters am ON am.appearance_id = a.id
             LEFT JOIN courts ct ON ct.id = a.court_id
             WHERE am.matter_id = ?1 AND a.cancelled = 0
             ORDER BY a.appearance_date, COALESCE(a.appearance_time, '99:99'), a.id",
            params![matter_id],
            map_appearance_summary,
        )?;
        let today = self.today()?;
        let deadlines = self.deadline_rows("d.matter_id = ?1", params![matter_id], &today, None)?;
        let notes = self.notes_for(NoteScope::Matter, matter_id)?;

        Ok(MatterProfile {
            id: matter_id.to_owned(),
            assignments,
            related,
            settings,
            deadlines,
            notes,
            ..head
        })
    }

    /// Lists matters, open ones first, for a chooser.
    pub fn matters(&self) -> Result<Vec<MatterSummary>> {
        self.matter_summaries("1 = 1", params![])
    }

    fn matter_summaries<P: rusqlite::Params>(
        &self,
        predicate: &str,
        parameters: P,
    ) -> Result<Vec<MatterSummary>> {
        self.collect(
            &format!(
                "SELECT m.id, m.caption, m.court_number, ct.name, m.status, m.custody_state,
                        m.offer_state, m.charge_summary,
                        (SELECT min(a.appearance_date) FROM appearances a
                          JOIN appearance_matters am ON am.appearance_id = a.id
                          WHERE am.matter_id = m.id AND a.cancelled = 0),
                        (SELECT count(*) FROM deadlines d
                          WHERE d.matter_id = m.id AND d.satisfied = 0),
                        m.evidence_case_id
                 FROM matters m
                 LEFT JOIN courts ct ON ct.id = m.court_id
                 WHERE {predicate}
                 ORDER BY CASE m.status WHEN 'open' THEN 1 WHEN 'pending_appointment' THEN 2
                                        ELSE 3 END,
                          m.caption, m.id"
            ),
            parameters,
            |row| {
                Ok(MatterSummary {
                    id: row.get(0)?,
                    caption: row.get(1)?,
                    court_number: row.get(2)?,
                    court: row.get(3)?,
                    status: row.get(4)?,
                    custody_state: row.get(5)?,
                    offer_state: row.get(6)?,
                    charge_summary: row.get(7)?,
                    next_setting: row.get(8)?,
                    open_deadlines: row.get(9)?,
                    evidence_case_id: row.get(10)?,
                })
            },
        )
    }

    // ----- courts and settings ---------------------------------------------

    /// Records a court the office appears in.
    pub fn create_court(&self, proposal: &ProposedCourt) -> Result<String> {
        let name = require_text(&proposal.name, "a court must have a name")?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("court", "courts", &id)?;
        self.connection
            .prepare_cached(
                "INSERT INTO courts (id, name, division, address, room)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute(params![
                id,
                name,
                optional(proposal.division.as_deref()),
                optional(proposal.address.as_deref()),
                optional(proposal.room.as_deref())
            ])?;
        Ok(id)
    }

    /// Finds a judge by name at a court, or records one.
    ///
    /// The calendar's twin of [`Self::user_named`]: a docket names a judge,
    /// not an identifier, and the same judge scheduled twice is one row.
    pub fn judge_named(&self, court_id: Option<&str>, display_name: &str) -> Result<String> {
        let name = require_text(display_name, "a judge must have a name")?;
        if let Some(id) = self.query_one(
            "SELECT id FROM judges WHERE display_name = ?1 AND court_id IS ?2",
            params![name, optional(court_id)],
            |row| row.get::<_, String>(0),
        )? {
            return Ok(id);
        }
        self.create_judge(court_id, &name)
    }

    /// Lists judges with the court they sit in, for a chooser.
    pub fn judges(&self) -> Result<Vec<(String, String)>> {
        self.collect(
            "SELECT j.id,
                    j.display_name || COALESCE(' — ' || c.name, '')
             FROM judges j LEFT JOIN courts c ON c.id = j.court_id
             ORDER BY j.display_name, c.name, j.id",
            params![],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    /// Records a judge as a reference row.
    pub fn create_judge(&self, court_id: Option<&str>, display_name: &str) -> Result<String> {
        let name = require_text(display_name, "a judge must have a name")?;
        let id = Uuid::now_v7().to_string();
        self.connection
            .prepare_cached("INSERT INTO judges (id, court_id, display_name) VALUES (?1, ?2, ?3)")?
            .execute(params![id, optional(court_id), name])?;
        Ok(id)
    }

    /// Lists courts by name.
    pub fn courts(&self) -> Result<Vec<(String, String)>> {
        self.collect(
            "SELECT id, name FROM courts ORDER BY name, division, id",
            params![],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    /// Schedules one court setting spanning every named matter.
    ///
    /// Enforces the rule the whole calendar is built around: a setting belongs
    /// to a client and covers that client's matters, so a person called on
    /// three cases at nine o'clock has one place to be and the docket has one
    /// row for it. At least one matter is required, and a matter belonging to
    /// anybody else is refused.
    pub fn schedule_appearance(&mut self, proposal: &ProposedAppearance) -> Result<String> {
        self.require_user(&proposal.author_user_id)?;
        self.require_client(&proposal.client_id)?;
        if proposal.matter_ids.is_empty() {
            return Err(Error::InvalidRecord(
                "a setting must cover at least one matter".to_owned(),
            ));
        }
        for matter_id in &proposal.matter_ids {
            if self.matter_client(matter_id)? != proposal.client_id {
                return Err(Error::WrongClient {
                    kind: "matter",
                    id: matter_id.clone(),
                });
            }
        }
        let date = CivilDate::require(&proposal.appearance_date)?.to_text();
        let time = match proposal.appearance_time.as_deref() {
            Some(text) if !text.trim().is_empty() => Some(require_time(text.trim())?),
            _ => None,
        };
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("appearance", "appearances", &id)?;

        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO appearances
               (id, client_id, court_id, judge_id, appearance_date, appearance_time,
                appearance_type, notes, author_user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                proposal.client_id,
                optional(proposal.court_id.as_deref()),
                optional(proposal.judge_id.as_deref()),
                date,
                time,
                proposal.appearance_type.as_str(),
                optional(proposal.notes.as_deref()),
                proposal.author_user_id
            ],
        )?;
        for matter_id in &proposal.matter_ids {
            transaction.execute(
                "INSERT INTO appearance_matters (id, appearance_id, matter_id)
                 VALUES (?1, ?2, ?3)",
                params![Uuid::now_v7().to_string(), id, matter_id],
            )?;
        }
        transaction.commit()?;
        Ok(id)
    }

    /// Adds one more of the client's matters to an existing setting, or records
    /// how a matter already on it is being handled differently.
    ///
    /// An override is how one case is continued or passed inside a setting the
    /// rest share, without splitting the setting into two rows.
    pub fn link_matter_to_appearance(
        &self,
        appearance_id: &str,
        matter_id: &str,
        override_note: Option<&str>,
    ) -> Result<()> {
        let owner = self
            .query_one(
                "SELECT client_id FROM appearances WHERE id = ?1",
                params![appearance_id],
                |row| row.get::<_, String>(0),
            )?
            .ok_or_else(|| Error::NotFound {
                kind: "appearance",
                id: appearance_id.to_owned(),
            })?;
        if self.matter_client(matter_id)? != owner {
            return Err(Error::WrongClient {
                kind: "matter",
                id: matter_id.to_owned(),
            });
        }
        self.connection
            .prepare_cached(
                "INSERT INTO appearance_matters (id, appearance_id, matter_id, override_note)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(appearance_id, matter_id)
                 DO UPDATE SET override_note = excluded.override_note",
            )?
            .execute(params![
                Uuid::now_v7().to_string(),
                appearance_id,
                matter_id,
                optional(override_note)
            ])?;
        Ok(())
    }

    /// Removes one matter from a setting, leaving the setting and the rest of
    /// its matters alone.
    ///
    /// Emptying a setting of every matter is refused by the schema: cancelling
    /// a setting is a deletion of the setting, not an emptying of it.
    pub fn unlink_matter_from_appearance(
        &self,
        appearance_id: &str,
        matter_id: &str,
    ) -> Result<()> {
        let removed = self
            .connection
            .prepare_cached(
                "DELETE FROM appearance_matters WHERE appearance_id = ?1 AND matter_id = ?2",
            )?
            .execute(params![appearance_id, matter_id])?;
        if removed == 0 {
            return Err(Error::NotFound {
                kind: "matter on this setting",
                id: matter_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Strikes a setting from the calendar.
    ///
    /// A cancellation is recorded, not erased. Notes may already hang off the
    /// setting and notes are append-only, so deleting the row would either fail
    /// on the foreign key or take somebody's written words with it. Every
    /// docket read filters cancelled settings out; the record stays.
    pub fn cancel_appearance(&self, appearance_id: &str, on: &str) -> Result<()> {
        let on = CivilDate::require(on)?.to_text();
        let updated = self
            .connection
            .prepare_cached(
                "UPDATE appearances SET cancelled = 1, cancelled_on = ?2
                 WHERE id = ?1 AND cancelled = 0",
            )?
            .execute(params![appearance_id, on])?;
        if updated == 0 {
            return Err(Error::NotFound {
                kind: "open appearance",
                id: appearance_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Records what happened at a setting.
    pub fn record_appearance_outcome(&self, appearance_id: &str, outcome: &str) -> Result<()> {
        let outcome = require_text(outcome, "an outcome must say something")?;
        let updated = self
            .connection
            .prepare_cached("UPDATE appearances SET outcome = ?2 WHERE id = ?1")?
            .execute(params![appearance_id, outcome])?;
        if updated == 0 {
            return Err(Error::NotFound {
                kind: "appearance",
                id: appearance_id.to_owned(),
            });
        }
        Ok(())
    }

    // ----- deadlines -------------------------------------------------------

    /// Records something owed by a date.
    pub fn record_deadline(&self, proposal: &ProposedDeadline) -> Result<String> {
        self.require_user(&proposal.author_user_id)?;
        self.matter_client(&proposal.matter_id)?;
        let description = require_text(&proposal.description, "a deadline must say what is owed")?;
        let due = CivilDate::require(&proposal.due_date)?.to_text();
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("deadline", "deadlines", &id)?;
        self.connection
            .prepare_cached(
                "INSERT INTO deadlines
                   (id, matter_id, description, due_date, origin, author_user_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )?
            .execute(params![
                id,
                proposal.matter_id,
                description,
                due,
                proposal.origin.as_str(),
                proposal.author_user_id
            ])?;
        Ok(id)
    }

    /// Records a deadline as met on a date.
    pub fn satisfy_deadline(&self, deadline_id: &str, as_of: &str) -> Result<()> {
        let on = CivilDate::require(as_of)?.to_text();
        let updated = self
            .connection
            .prepare_cached("UPDATE deadlines SET satisfied = 1, satisfied_on = ?2 WHERE id = ?1")?
            .execute(params![deadline_id, on])?;
        if updated == 0 {
            return Err(Error::NotFound {
                kind: "deadline",
                id: deadline_id.to_owned(),
            });
        }
        Ok(())
    }

    /// Deadlines still owed, split into what is already past and what is coming.
    pub fn upcoming_deadlines(&self, as_of: &str, within_days: u32) -> Result<UpcomingDeadlines> {
        let start = CivilDate::require(as_of)?;
        let horizon = start.add_days(i64::from(within_days)).to_text();
        let as_of = start.to_text();
        let overdue = self.deadline_rows(
            "d.satisfied = 0 AND d.due_date < ?1",
            params![as_of],
            &as_of,
            Some("d.due_date ASC"),
        )?;
        let upcoming = self.deadline_rows(
            "d.satisfied = 0 AND d.due_date >= ?1 AND d.due_date <= ?2",
            params![as_of, horizon],
            &as_of,
            Some("d.due_date ASC"),
        )?;
        Ok(UpcomingDeadlines {
            as_of,
            within_days,
            overdue,
            upcoming,
        })
    }

    fn deadline_rows<P: rusqlite::Params>(
        &self,
        predicate: &str,
        parameters: P,
        as_of: &str,
        order: Option<&str>,
    ) -> Result<Vec<DeadlineRow>> {
        let measuring = CivilDate::require(as_of)?;
        let order = order.unwrap_or("d.due_date ASC");
        self.collect(
            &format!(
                "SELECT d.id, d.matter_id, m.caption, m.court_number, c.display_name,
                        d.description, d.due_date, d.origin, d.satisfied
                 FROM deadlines d
                 JOIN matters m ON m.id = d.matter_id
                 JOIN clients c ON c.id = m.client_id
                 WHERE {predicate}
                 ORDER BY {order}, m.caption, d.id"
            ),
            parameters,
            |row| {
                let due: String = row.get(6)?;
                Ok(DeadlineRow {
                    id: row.get(0)?,
                    matter_id: row.get(1)?,
                    matter: row.get(2)?,
                    court_number: row.get(3)?,
                    client: row.get(4)?,
                    description: row.get(5)?,
                    days_remaining: CivilDate::parse(&due)
                        .map_or(0, |date| measuring.days_until(date)),
                    due_date: due,
                    origin: row.get(7)?,
                    satisfied: row.get::<_, i64>(8)? == 1,
                })
            },
        )
    }

    // ----- the docket ------------------------------------------------------

    /// Every setting on one day, one row per setting.
    pub fn docket_day(&self, date: &str) -> Result<DocketDay> {
        let day = CivilDate::require(date)?;
        let date = day.to_text();
        let settings = self.docket_entries("a.appearance_date = ?1", params![date])?;
        let deadlines_due = self.deadline_rows(
            "d.satisfied = 0 AND d.due_date = ?1",
            params![date],
            &date,
            None,
        )?;
        Ok(DocketDay {
            weekday: day.weekday().as_str().to_owned(),
            date,
            settings,
            deadlines_due,
        })
    }

    /// Every day in a range, in order, including days with nothing on them.
    ///
    /// An empty day is still a day a defender has to know is empty, so it is
    /// returned rather than skipped.
    pub fn docket_range(&self, start_date: &str, end_date: &str) -> Result<Vec<DocketDay>> {
        let start = CivilDate::require(start_date)?;
        let end = CivilDate::require(end_date)?;
        if start.days_until(end) < 0 {
            return Err(Error::InvalidRecord(
                "a docket range ends on or after it starts".to_owned(),
            ));
        }
        let mut days = Vec::new();
        let mut cursor = start;
        while cursor <= end {
            days.push(self.docket_day(&cursor.to_text())?);
            cursor = cursor.add_days(1);
        }
        Ok(days)
    }

    /// The week containing a date, Monday through Sunday.
    pub fn docket_week(&self, containing: &str) -> Result<Vec<DocketDay>> {
        let day = CivilDate::require(containing)?;
        self.docket_range(&day.week_start().to_text(), &day.week_end().to_text())
    }

    fn docket_entries<P: rusqlite::Params>(
        &self,
        predicate: &str,
        parameters: P,
    ) -> Result<Vec<DocketEntry>> {
        let mut entries = self.collect(
            &format!(
                "SELECT a.id, a.appearance_date, a.appearance_time, a.appearance_type,
                        ct.name, ct.room, j.display_name, a.client_id, c.display_name, a.outcome
                 FROM appearances a
                 JOIN clients c ON c.id = a.client_id
                 LEFT JOIN courts ct ON ct.id = a.court_id
                 LEFT JOIN judges j ON j.id = a.judge_id
                 WHERE a.cancelled = 0 AND ({predicate})
                 ORDER BY a.appearance_date, COALESCE(a.appearance_time, '99:99'),
                          c.display_name, a.id"
            ),
            parameters,
            |row| {
                Ok(DocketEntry {
                    id: row.get(0)?,
                    date: row.get(1)?,
                    time: row.get(2)?,
                    appearance_type: row.get(3)?,
                    court: row.get(4)?,
                    room: row.get(5)?,
                    judge: row.get(6)?,
                    client_id: row.get(7)?,
                    client: row.get(8)?,
                    matters: Vec::new(),
                    last_contact: None,
                    open_deadlines: 0,
                    notes: 0,
                    outcome: row.get(9)?,
                })
            },
        )?;

        let mut lines = self.connection.prepare_cached(
            "SELECT m.id, m.caption, m.court_number, m.charge_summary, m.status,
                    m.custody_state, m.offer_state, m.offer_summary, am.override_note,
                    m.evidence_case_id, m.last_contact_on,
                    (SELECT count(*) FROM deadlines d
                      WHERE d.matter_id = m.id AND d.satisfied = 0)
             FROM appearance_matters am
             JOIN matters m ON m.id = am.matter_id
             WHERE am.appearance_id = ?1
             ORDER BY COALESCE(m.court_number, ''), m.caption, m.id",
        )?;
        let mut note_count = self.connection.prepare_cached(
            "SELECT count(*) FROM notes n
             WHERE n.appearance_id = ?1
               AND NOT EXISTS (SELECT 1 FROM notes later WHERE later.supersedes_note_id = n.id)",
        )?;

        for entry in &mut entries {
            let mut last_contact: Option<String> = None;
            let mut open_deadlines = 0_u32;
            entry.matters = lines
                .query_map(params![entry.id], |row| {
                    let contact: Option<String> = row.get(10)?;
                    if let Some(seen) = contact
                        && last_contact
                            .as_deref()
                            .is_none_or(|held| held < seen.as_str())
                    {
                        last_contact = Some(seen);
                    }
                    open_deadlines += row.get::<_, u32>(11)?;
                    Ok(DocketMatterLine {
                        id: row.get(0)?,
                        caption: row.get(1)?,
                        court_number: row.get(2)?,
                        charge_summary: row.get(3)?,
                        status: row.get(4)?,
                        custody_state: row.get(5)?,
                        offer_state: row.get(6)?,
                        offer_summary: row.get(7)?,
                        override_note: row.get(8)?,
                        evidence_case_id: row.get(9)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            entry.last_contact = last_contact;
            entry.open_deadlines = open_deadlines;
            entry.notes = note_count.query_row(params![entry.id], |row| row.get(0))?;
        }
        Ok(entries)
    }

    /// The client's next setting on or after today, across every matter.
    fn next_setting_for_client(&self, client_id: &str) -> Result<Option<AppearanceSummary>> {
        let today = self.today()?;
        self.query_one(
            "SELECT a.id, a.appearance_date, a.appearance_time, a.appearance_type, ct.name,
                    (SELECT count(*) FROM appearance_matters WHERE appearance_id = a.id),
                    a.outcome
             FROM appearances a
             LEFT JOIN courts ct ON ct.id = a.court_id
             WHERE a.cancelled = 0 AND a.client_id = ?1 AND a.appearance_date >= ?2
             ORDER BY a.appearance_date, COALESCE(a.appearance_time, '99:99'), a.id
             LIMIT 1",
            params![client_id, today],
            map_appearance_summary,
        )
    }

    // ----- notes -----------------------------------------------------------

    /// Writes a note on a client, a matter, or a setting.
    ///
    /// Enforces a real author, exactly one scope, and a body that says
    /// something. Mentions are read out of the text once, here, so a later read
    /// never rescans it.
    pub fn write_note(&mut self, proposal: &ProposedNote) -> Result<NoteEntry> {
        self.require_user(&proposal.author_user_id)?;
        let (scope, subject) = self.note_scope(proposal)?;
        let body = require_text(&proposal.body, "a note must say something")?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("note", "notes", &id)?;
        self.insert_note(&NoteVersion {
            id: &id,
            scope,
            subject_id: &subject,
            body: &body,
            version: 1,
            supersedes: None,
            author_user_id: &proposal.author_user_id,
        })?;
        self.note(&id)
    }

    /// Writes the next version of a note.
    ///
    /// The predecessor is not altered — the schema makes that impossible — so an
    /// edit is a new row that supersedes the old one, visible in the history and
    /// filtered out of every view. Revising an already-superseded version is
    /// refused, because two revisions of one note would fork its history.
    pub fn revise_note(&mut self, note_id: &str, proposal: &ProposedNote) -> Result<NoteEntry> {
        self.require_user(&proposal.author_user_id)?;
        let current = self.note(note_id)?;
        if let Some(successor) = self.query_one(
            "SELECT id FROM notes WHERE supersedes_note_id = ?1",
            params![note_id],
            |row| row.get::<_, String>(0),
        )? {
            return Err(Error::Superseded {
                id: note_id.to_owned(),
                by: Some(successor),
            });
        }
        let scope = NoteScope::from_db(&current.scope).ok_or_else(|| {
            Error::InvalidRecord(format!("note `{note_id}` has an unreadable scope"))
        })?;
        let body = require_text(&proposal.body, "a note must say something")?;
        let id = supplied_or_generated(proposal.id.as_deref());
        self.refuse_existing("note", "notes", &id)?;
        self.insert_note(&NoteVersion {
            id: &id,
            scope,
            subject_id: &current.subject_id,
            body: &body,
            version: current.version + 1,
            supersedes: Some(note_id),
            author_user_id: &proposal.author_user_id,
        })?;
        self.note(&id)
    }

    fn insert_note(&mut self, version: &NoteVersion<'_>) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let (client, matter, appearance) = match version.scope {
            NoteScope::Client => (Some(version.subject_id), None, None),
            NoteScope::Matter => (None, Some(version.subject_id), None),
            NoteScope::Appearance => (None, None, Some(version.subject_id)),
        };
        transaction.execute(
            "INSERT INTO notes
               (id, client_id, matter_id, appearance_id, body, version, supersedes_note_id,
                author_user_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                version.id,
                client,
                matter,
                appearance,
                version.body,
                version.version,
                version.supersedes,
                version.author_user_id
            ],
        )?;
        for tag in mentions_in(version.body) {
            transaction.execute(
                "INSERT OR IGNORE INTO note_mentions (id, note_id, tag) VALUES (?1, ?2, ?3)",
                params![Uuid::now_v7().to_string(), version.id, tag.as_str()],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    fn note_scope(&self, proposal: &ProposedNote) -> Result<(NoteScope, String)> {
        let named: Vec<(NoteScope, &str)> = [
            (NoteScope::Client, proposal.client_id.as_deref()),
            (NoteScope::Matter, proposal.matter_id.as_deref()),
            (NoteScope::Appearance, proposal.appearance_id.as_deref()),
        ]
        .into_iter()
        .filter_map(|(scope, id)| {
            id.map(str::trim)
                .filter(|id| !id.is_empty())
                .map(|id| (scope, id))
        })
        .collect();

        let [(scope, subject)] = named.as_slice() else {
            return Err(Error::InvalidRecord(
                "a note belongs to exactly one of a client, a matter, or a setting".to_owned(),
            ));
        };
        match scope {
            NoteScope::Client => {
                self.require_client(subject)?;
            }
            NoteScope::Matter => {
                self.matter_client(subject)?;
            }
            NoteScope::Appearance => {
                if !self.exists("SELECT 1 FROM appearances WHERE id = ?1", params![subject])? {
                    return Err(Error::NotFound {
                        kind: "appearance",
                        id: (*subject).to_owned(),
                    });
                }
            }
        }
        Ok((*scope, (*subject).to_owned()))
    }

    /// One note by identifier, whether or not it is still current.
    pub fn note(&self, note_id: &str) -> Result<NoteEntry> {
        let mut entry = self
            .query_one(
                "SELECT n.id, n.client_id, n.matter_id, n.appearance_id, n.body, n.version,
                        n.supersedes_note_id, u.display_name, n.created_at
                 FROM notes n JOIN users u ON u.id = n.author_user_id
                 WHERE n.id = ?1",
                params![note_id],
                map_note,
            )?
            .ok_or_else(|| Error::NotFound {
                kind: "note",
                id: note_id.to_owned(),
            })?;
        entry.mentions = self.mentions_of(note_id)?;
        Ok(entry)
    }

    /// Current notes on one subject, newest first.
    pub fn notes_for(&self, scope: NoteScope, subject_id: &str) -> Result<Vec<NoteEntry>> {
        let column = match scope {
            NoteScope::Client => "client_id",
            NoteScope::Matter => "matter_id",
            NoteScope::Appearance => "appearance_id",
        };
        let mut entries = self.collect(
            &format!(
                "SELECT n.id, n.client_id, n.matter_id, n.appearance_id, n.body, n.version,
                        n.supersedes_note_id, u.display_name, n.created_at
                 FROM notes n JOIN users u ON u.id = n.author_user_id
                 WHERE n.{column} = ?1
                   AND NOT EXISTS (SELECT 1 FROM notes later
                                    WHERE later.supersedes_note_id = n.id)
                 ORDER BY n.created_at DESC, n.id DESC"
            ),
            params![subject_id],
            map_note,
        )?;
        for entry in &mut entries {
            entry.mentions = self.mentions_of(&entry.id)?;
        }
        Ok(entries)
    }

    /// Every version of one note, oldest first.
    ///
    /// This is what makes an edit visible rather than silent: the superseded
    /// text is still here, with the name of whoever wrote it.
    pub fn note_history(&self, note_id: &str) -> Result<NoteHistory> {
        let mut chain = vec![self.note(note_id)?];
        while let Some(previous) = chain
            .last()
            .and_then(|entry| entry.supersedes_note_id.clone())
        {
            chain.push(self.note(&previous)?);
        }
        chain.reverse();

        let mut current = self.note(note_id)?;
        while let Some(next) = self.query_one(
            "SELECT id FROM notes WHERE supersedes_note_id = ?1",
            params![current.id],
            |row| row.get::<_, String>(0),
        )? {
            current = self.note(&next)?;
            chain.push(current.clone());
        }
        Ok(NoteHistory {
            current_id: current.id,
            versions: chain,
        })
    }

    fn mentions_of(&self, note_id: &str) -> Result<Vec<String>> {
        self.collect(
            "SELECT tag FROM note_mentions WHERE note_id = ?1 ORDER BY tag",
            params![note_id],
            |row| row.get(0),
        )
    }

    /// Current notes carrying one mention, across the whole office.
    pub fn notes_mentioning(&self, tag: MentionTag) -> Result<Vec<NoteEntry>> {
        let mut entries = self.collect(
            "SELECT n.id, n.client_id, n.matter_id, n.appearance_id, n.body, n.version,
                    n.supersedes_note_id, u.display_name, n.created_at
             FROM notes n
             JOIN users u ON u.id = n.author_user_id
             JOIN note_mentions t ON t.note_id = n.id
             WHERE t.tag = ?1
               AND NOT EXISTS (SELECT 1 FROM notes later WHERE later.supersedes_note_id = n.id)
             ORDER BY n.created_at DESC, n.id DESC",
            params![tag.as_str()],
            map_note,
        )?;
        for entry in &mut entries {
            entry.mentions = self.mentions_of(&entry.id)?;
        }
        Ok(entries)
    }

    // ----- search and conflicts --------------------------------------------

    /// Finds office records by their words, best match first.
    ///
    /// Spans clients, matters, notes, users and courts, and reaches nothing
    /// else.
    ///
    /// The query is plain text, not a query language. Somebody looking for
    /// `CR-2026-491` or `(555) 481-2290` is typing a case number and a phone
    /// number, not writing an expression, and full-text syntax would read the
    /// hyphens and parentheses as operators and return nothing. Every term is
    /// quoted into a literal by [`fts_query`]; only bare uppercase `AND`, `OR`
    /// and `NOT` keep their meaning. Evidence search is the one with a syntax,
    /// and it stays that way.
    ///
    /// A query that reduces to a run of digits is also matched against the
    /// digits-only form of every phone number, because the tokenizer splits on
    /// punctuation and an unpunctuated query would otherwise miss a punctuated
    /// record.
    pub fn search(&self, query: &str, limit: u32) -> Result<Vec<OfficeSearchHit>> {
        let query = query.trim();
        if query.is_empty() {
            return Err(Error::InvalidSearch("a search needs a term".to_owned()));
        }
        let mut expression = fts_query(query);
        let digits = digits_of(query);
        if digits.len() >= 7 && digits != query {
            expression = format!("({expression}) OR \"{digits}\"");
        }

        self.collect(
            "SELECT d.kind, d.subject_id, d.title,
                    snippet(office_search, 1, '[', ']', '…', 12),
                    CASE d.kind
                      WHEN 'matter' THEN (SELECT c.display_name FROM matters m
                                            JOIN clients c ON c.id = m.client_id
                                            WHERE m.id = d.subject_id)
                      WHEN 'note' THEN (SELECT c.display_name FROM notes n
                                          LEFT JOIN matters m ON m.id = n.matter_id
                                          JOIN clients c
                                            ON c.id = COALESCE(n.client_id, m.client_id)
                                          WHERE n.id = d.subject_id)
                      ELSE NULL END
             FROM office_search
             JOIN search_documents d ON d.rowid = office_search.rowid
             WHERE office_search MATCH ?1
             ORDER BY rank, d.kind, d.subject_id
             LIMIT ?2",
            params![expression, limit],
            |row| {
                Ok(OfficeSearchHit {
                    kind: row.get(0)?,
                    subject_id: row.get(1)?,
                    title: row.get(2)?,
                    excerpt: row.get(3)?,
                    client: row.get(4)?,
                })
            },
        )
        .map_err(|error| match error {
            Error::Database(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("fts5") || message.contains("syntax") =>
            {
                Error::InvalidSearch(message)
            }
            other => other,
        })
    }

    /// People the office may already know, offered for a person to judge.
    ///
    /// A question, never an answer. Nothing is merged, nothing is written, and
    /// an empty result is not a claim that the person is new — only that
    /// nothing obvious matched.
    pub fn possible_client_duplicates(
        &self,
        display_name: &str,
        contacts: &[String],
        exclude_client_id: Option<&str>,
    ) -> Result<Vec<PossiblePerson>> {
        let name = display_name.trim().to_lowercase();
        let mut reasons: BTreeMap<String, Vec<String>> = BTreeMap::new();

        if !name.is_empty() {
            let mut by_name = self.connection.prepare_cached(
                "SELECT DISTINCT c.id, c.display_name
                 FROM clients c
                 LEFT JOIN client_aliases a ON a.client_id = c.id
                 WHERE lower(c.display_name) = ?1 OR lower(a.alias) = ?1",
            )?;
            for row in by_name.query_map(params![name], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (id, existing) = row?;
                reasons
                    .entry(id)
                    .or_default()
                    .push(format!("name: {display_name} / {existing}"));
            }
        }

        for contact in contacts {
            let value = contact.trim();
            if value.is_empty() {
                continue;
            }
            let digits = digits_of(value);
            let mut by_contact = self.connection.prepare_cached(
                "SELECT c.id, ct.value FROM client_contacts ct
                 JOIN clients c ON c.id = ct.client_id
                 WHERE lower(ct.value) = lower(?1)
                    OR (?2 <> '' AND ct.digits = ?2)",
            )?;
            for row in by_contact.query_map(params![value, digits], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })? {
                let (id, existing) = row?;
                reasons
                    .entry(id)
                    .or_default()
                    .push(format!("contact: {value} / {existing}"));
            }
        }

        if let Some(excluded) = exclude_client_id {
            reasons.remove(excluded);
        }

        let mut candidates = Vec::with_capacity(reasons.len());
        for (client_id, mut matched_on) in reasons {
            matched_on.sort();
            matched_on.dedup();
            let Some((display_name, date_of_birth, matters)) = self.query_one(
                "SELECT c.display_name, c.date_of_birth,
                        (SELECT count(*) FROM matters m WHERE m.client_id = c.id)
                 FROM clients c WHERE c.id = ?1",
                params![client_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?
            else {
                continue;
            };
            candidates.push(PossiblePerson {
                client_id,
                display_name,
                date_of_birth,
                matched_on,
                matters,
            });
        }
        candidates.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.client_id.cmp(&right.client_id))
        });
        Ok(candidates)
    }

    /// Records a named person's decision about a possible identity match.
    ///
    /// The only way a client is ever tied to an evidence entity. Both `linked`
    /// and `dismissed` are decisions; neither alters the client or the entity,
    /// and there is no third value meaning "the software thinks so".
    pub fn decide_identity_link(&self, proposal: &ProposedIdentityLinkDecision) -> Result<()> {
        self.require_user(&proposal.author_user_id)?;
        self.require_client(&proposal.client_id)?;
        let case = require_text(
            &proposal.evidence_case_id,
            "an identity decision must name the evidence case",
        )?;
        let entity = require_text(
            &proposal.evidence_entity_id,
            "an identity decision must name the evidence entity",
        )?;
        self.connection
            .prepare_cached(
                "INSERT INTO client_evidence_links
                   (id, client_id, evidence_case_id, evidence_entity_id, state, matched_on,
                    author_user_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(client_id, evidence_case_id, evidence_entity_id) DO UPDATE
                   SET state = excluded.state,
                       matched_on = excluded.matched_on,
                       author_user_id = excluded.author_user_id,
                       decided_at = datetime('now')",
            )?
            .execute(params![
                Uuid::now_v7().to_string(),
                proposal.client_id,
                case,
                entity,
                proposal.state.as_str(),
                optional(proposal.matched_on.as_deref()),
                proposal.author_user_id
            ])?;
        Ok(())
    }

    /// What a person already decided about one candidate pair, if anything.
    ///
    /// `None` means nobody has been asked yet, which is the only state in which
    /// the prompt is offered again.
    pub fn identity_link_state(
        &self,
        client_id: &str,
        evidence_case_id: &str,
        evidence_entity_id: &str,
    ) -> Result<Option<IdentityLinkState>> {
        Ok(self
            .query_one(
                "SELECT state FROM client_evidence_links
                 WHERE client_id = ?1 AND evidence_case_id = ?2 AND evidence_entity_id = ?3",
                params![client_id, evidence_case_id, evidence_entity_id],
                |row| row.get::<_, String>(0),
            )?
            .and_then(|state| IdentityLinkState::from_db(&state)))
    }

    /// Kernel entities a person has confirmed are this client.
    pub fn linked_entities(&self, client_id: &str) -> Result<Vec<(String, String)>> {
        self.collect(
            "SELECT evidence_case_id, evidence_entity_id FROM client_evidence_links
             WHERE client_id = ?1 AND state = 'linked'
             ORDER BY evidence_case_id, evidence_entity_id",
            params![client_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    }

    // ----- shared read plumbing --------------------------------------------

    fn collect<T, P, F>(&self, sql: &str, parameters: P, map: F) -> Result<Vec<T>>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        self.connection
            .prepare_cached(sql)?
            .query_map(parameters, map)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

// ----- free helpers --------------------------------------------------------

/// One version of a note, as it is about to be written.
///
/// A note carries enough fields that passing them positionally invites the
/// silent mix-up this struct exists to prevent.
struct NoteVersion<'a> {
    id: &'a str,
    scope: NoteScope,
    subject_id: &'a str,
    body: &'a str,
    version: u32,
    supersedes: Option<&'a str>,
    author_user_id: &'a str,
}

fn require_text(value: &str, complaint: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidRecord(complaint.to_owned()));
    }
    Ok(trimmed.to_owned())
}

fn supplied_or_generated(supplied: Option<&str>) -> String {
    match supplied.map(str::trim) {
        Some(id) if !id.is_empty() => id.to_owned(),
        _ => Uuid::now_v7().to_string(),
    }
}

fn optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// A language as a person named it, or nothing.
///
/// Not a vocabulary: an office serves whoever walks in, and a `CHECK` over a
/// list of languages would refuse a real answer. Blank is nothing, and a value
/// too long to be a language name is refused rather than stored.
fn optional_language(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim) {
        Some(text) if !text.is_empty() => {
            if text.len() > 64 {
                return Err(Error::InvalidRecord(
                    "a preferred language is a name, not a description".to_owned(),
                ));
            }
            Ok(Some(text.to_owned()))
        }
        _ => Ok(None),
    }
}

fn optional_date(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim) {
        Some(text) if !text.is_empty() => Ok(Some(CivilDate::require(text)?.to_text())),
        _ => Ok(None),
    }
}

/// Turns what a person typed into a full-text expression.
///
/// Every term becomes a quoted literal, so a hyphen in a case number is a
/// hyphen and a parenthesis in a phone number is a parenthesis rather than a
/// grouping operator. Bare uppercase `AND`, `OR` and `NOT` are kept as
/// operators, and a trailing `*` still means a prefix, so the two things a
/// person might reasonably expect to work still do. An unbalanced quote is
/// searched for as a quote instead of failing.
fn fts_query(query: &str) -> String {
    let mut terms: Vec<String> = Vec::new();
    for token in query.split_whitespace() {
        if matches!(token, "AND" | "OR" | "NOT") {
            terms.push(token.to_owned());
            continue;
        }
        let (stem, prefix) = token
            .strip_suffix('*')
            .map_or((token, ""), |stem| (stem, "*"));
        let literal = stem.trim_matches('"').replace('"', "\"\"");
        if literal.is_empty() {
            continue;
        }
        terms.push(format!("\"{literal}\"{prefix}"));
    }
    if terms.is_empty() {
        // Everything the person typed was punctuation. Search for it whole
        // rather than for nothing at all.
        return format!("\"{}\"", query.replace('"', "\"\""));
    }
    terms.join(" ")
}

/// The digits of a contact value, which is how an unpunctuated query finds a
/// punctuated number.
fn digits_of(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}

/// The role mentions a note calls on, read out of its own text.
fn mentions_in(body: &str) -> Vec<MentionTag> {
    let mut found: Vec<MentionTag> = Vec::new();
    for (index, _) in body.match_indices('@') {
        let rest = &body[index + 1..];
        let word: String = rest
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        if let Some(tag) = MentionTag::from_db(&word)
            && !found.contains(&tag)
        {
            found.push(tag);
        }
    }
    found
}

fn insert_contact(
    connection: &Connection,
    client_id: &str,
    proposal: &ProposedClientContact,
) -> Result<String> {
    let value = require_text(&proposal.value, "a contact must have a value")?;
    let digits = proposal
        .kind
        .is_dialable()
        .then(|| digits_of(&value))
        .filter(|digits| !digits.is_empty());
    let id = Uuid::now_v7().to_string();
    connection
        .prepare_cached(
            "INSERT INTO client_contacts (id, client_id, kind, value, digits, label, is_primary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .execute(params![
            id,
            client_id,
            proposal.kind.as_str(),
            value,
            digits,
            optional(proposal.label.as_deref()),
            i64::from(proposal.is_primary)
        ])?;
    Ok(id)
}

fn map_appearance_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<AppearanceSummary> {
    Ok(AppearanceSummary {
        id: row.get(0)?,
        date: row.get(1)?,
        time: row.get(2)?,
        appearance_type: row.get(3)?,
        court: row.get(4)?,
        matters_covered: row.get(5)?,
        outcome: row.get(6)?,
    })
}

fn map_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<NoteEntry> {
    let client: Option<String> = row.get(1)?;
    let matter: Option<String> = row.get(2)?;
    let appearance: Option<String> = row.get(3)?;
    let (scope, subject_id) = match (client, matter, appearance) {
        (Some(id), _, _) => (NoteScope::Client, id),
        (_, Some(id), _) => (NoteScope::Matter, id),
        (_, _, Some(id)) => (NoteScope::Appearance, id),
        _ => (NoteScope::Client, String::new()),
    };
    Ok(NoteEntry {
        id: row.get(0)?,
        scope: scope.as_str().to_owned(),
        subject_id,
        body: row.get(4)?,
        version: row.get(5)?,
        supersedes_note_id: row.get(6)?,
        author: row.get(7)?,
        created_at: row.get(8)?,
        mentions: Vec::new(),
    })
}

/// Every assignment role, for a chooser.
pub const ASSIGNMENT_ROLES: [AssignmentRole; 6] = [
    AssignmentRole::Attorney,
    AssignmentRole::SecondChair,
    AssignmentRole::Investigator,
    AssignmentRole::Paralegal,
    AssignmentRole::SocialWorker,
    AssignmentRole::Supervisor,
];

/// Every contact kind, for a chooser.
pub const CONTACT_KINDS: [ContactKind; 5] = [
    ContactKind::Phone,
    ContactKind::Email,
    ContactKind::Address,
    ContactKind::Emergency,
    ContactKind::Other,
];

/// Every role a user may hold, mirroring the `CHECK` in migration 0001.
pub const USER_ROLES: [&str; 6] = [
    "attorney",
    "investigator",
    "paralegal",
    "social_worker",
    "supervisor",
    "administrator",
];

/// Languages a defender office meets before it meets any others.
///
/// A starting list for a chooser, never a limit: any typed language is
/// accepted, and [`OfficeStore::client_languages`] puts the office's own
/// answers ahead of this list once it has any.
pub const COMMON_LANGUAGES: [&str; 12] = [
    "English",
    "Spanish",
    "Vietnamese",
    "Mandarin",
    "Cantonese",
    "Arabic",
    "Somali",
    "Haitian Creole",
    "Russian",
    "Korean",
    "Tagalog",
    "American Sign Language",
];

#[cfg(test)]
mod schema {
    use super::*;
    use crate::fixture::OfficeFixture;

    /// The fixture anchors on a fixed Monday so nothing here changes overnight.
    const ANCHOR: &str = "2026-08-31";

    fn seeded() -> OfficeStore {
        let mut store = OfficeStore::in_memory().expect("in-memory store");
        OfficeFixture::MisdemeanorDocket
            .seed_from(&mut store, ANCHOR)
            .expect("seed the misdemeanor docket");
        store
    }

    /// These tests bypass the `OfficeStore` API on purpose. They assert what
    /// the *migrations* enforce, so the guarantee survives a future caller that
    /// writes its own SQL — the same reason the kernel keeps a `schema` module
    /// beside its store.
    fn refuses(store: &OfficeStore, statement: &str, expected: &str) {
        let error = store
            .connection
            .execute(statement, [])
            .expect_err("the schema must refuse this");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got: {error}"
        );
    }

    #[test]
    fn a_note_cannot_be_updated_or_deleted_in_place() {
        let store = seeded();
        refuses(
            &store,
            "UPDATE notes SET body = 'rewritten' WHERE id = 'note-rivera-client'",
            "append-only",
        );
        refuses(
            &store,
            "DELETE FROM notes WHERE id = 'note-rivera-client'",
            "append-only",
        );
        let body: String = store
            .connection
            .query_row(
                "SELECT body FROM notes WHERE id = 'note-rivera-client'",
                [],
                |row| row.get(0),
            )
            .expect("the note is still there");
        assert!(body.contains("Follows the person"), "{body}");
    }

    #[test]
    fn an_office_record_cannot_be_written_without_an_author() {
        let store = seeded();
        for (table, columns, values) in [
            (
                "clients",
                "(id, display_name, author_user_id)",
                "('anonymous', 'Nobody', '   ')",
            ),
            (
                "matters",
                "(id, client_id, caption, author_user_id)",
                "('anonymous', 'client-rivera', 'State v. Nobody', '   ')",
            ),
            (
                "appearances",
                "(id, client_id, appearance_date, author_user_id)",
                "('anonymous', 'client-rivera', '2026-09-04', '   ')",
            ),
            (
                "deadlines",
                "(id, matter_id, description, due_date, origin, author_user_id)",
                "('anonymous', 'matter-rivera-1', 'Something', '2026-09-04', 'statutory', '   ')",
            ),
            (
                "notes",
                "(id, client_id, body, author_user_id)",
                "('anonymous', 'client-rivera', 'Unattributed', '   ')",
            ),
        ] {
            refuses(
                &store,
                &format!("INSERT INTO {table} {columns} VALUES {values}"),
                "must name the person",
            );
        }
    }

    #[test]
    fn who_wrote_a_record_and_when_cannot_be_rewritten() {
        let store = seeded();
        for (table, id) in [
            ("clients", "client-rivera"),
            ("matters", "matter-rivera-1"),
            ("appearances", "appearance-rivera-consolidated"),
            ("deadlines", "deadline-rivera-speedy"),
        ] {
            // Somebody other than whoever actually wrote this row, so the
            // attempted rewrite is a real change rather than a no-op.
            let interloper: String = store
                .connection
                .query_row(
                    &format!(
                        "SELECT u.id FROM users u
                         WHERE u.id <> (SELECT author_user_id FROM {table} WHERE id = ?1)
                         LIMIT 1"
                    ),
                    params![id],
                    |row| row.get(0),
                )
                .expect("another user");
            refuses(
                &store,
                &format!("UPDATE {table} SET author_user_id = '{interloper}' WHERE id = '{id}'"),
                "cannot be rewritten",
            );
            refuses(
                &store,
                &format!("UPDATE {table} SET created_at = '1999-01-01 00:00:00' WHERE id = '{id}'"),
                "cannot be rewritten",
            );
        }
    }

    #[test]
    fn two_revisions_cannot_claim_the_same_predecessor() {
        let store = seeded();
        let error = store
            .connection
            .execute(
                "INSERT INTO notes (id, matter_id, body, version, supersedes_note_id,
                                    author_user_id)
                 SELECT 'forked', 'matter-rivera-1', 'A second replacement', 3,
                        'note-rivera-plan-v1', author_user_id
                 FROM notes WHERE id = 'note-rivera-plan-v1'",
                [],
            )
            .expect_err("a forked history must be refused");
        assert!(error.to_string().contains("UNIQUE"), "{error}");
    }

    #[test]
    fn a_setting_cannot_be_given_another_clients_matter() {
        let store = seeded();
        refuses(
            &store,
            "INSERT INTO appearance_matters (id, appearance_id, matter_id)
             VALUES ('crossed', 'appearance-rivera-consolidated', 'matter-okonkwo-1')",
            "one client",
        );
    }

    #[test]
    fn the_schema_refuses_to_empty_a_setting_of_every_matter() {
        let store = seeded();
        refuses(
            &store,
            "DELETE FROM appearance_matters
             WHERE appearance_id = 'appearance-okonkwo-bond' AND matter_id = 'matter-okonkwo-1'",
            "cannot be emptied",
        );
    }

    #[test]
    fn a_note_carries_exactly_one_scope() {
        let store = seeded();
        for (id, client, matter) in [
            ("two-scopes", "'client-rivera'", "'matter-rivera-1'"),
            ("no-scope", "NULL", "NULL"),
        ] {
            let error = store
                .connection
                .execute(
                    &format!(
                        "INSERT INTO notes (id, client_id, matter_id, body, author_user_id)
                         SELECT '{id}', {client}, {matter}, 'Filed under what?', id
                         FROM users LIMIT 1"
                    ),
                    [],
                )
                .expect_err("a note filed under two things, or none, must be refused");
            assert!(error.to_string().contains("CHECK"), "{error}");
        }
    }

    #[test]
    fn the_schema_accepts_only_the_vocabulary_the_enums_name() {
        let store = seeded();
        for statement in [
            "UPDATE matters SET status = 'archived' WHERE id = 'matter-rivera-1'",
            "UPDATE matters SET custody_state = 'maybe' WHERE id = 'matter-rivera-1'",
            "UPDATE matters SET offer_state = 'pending' WHERE id = 'matter-rivera-1'",
            "UPDATE deadlines SET origin = 'invented' WHERE id = 'deadline-rivera-speedy'",
            "UPDATE appearances SET appearance_type = 'brunch'
               WHERE id = 'appearance-rivera-consolidated'",
        ] {
            refuses(&store, statement, "CHECK");
        }
    }

    #[test]
    fn a_client_keeps_at_most_one_primary_contact_per_kind() {
        let store = seeded();
        let error = store
            .connection
            .execute(
                "INSERT INTO client_contacts (id, client_id, kind, value, is_primary)
                 VALUES ('second-primary', 'client-rivera', 'phone', '(555) 000-0000', 1)",
                [],
            )
            .expect_err("two primary numbers would make `call the client` ambiguous");
        assert!(error.to_string().contains("UNIQUE"), "{error}");
    }

    #[test]
    fn the_schema_refuses_a_sex_the_vocabulary_does_not_name() {
        let store = seeded();
        refuses(
            &store,
            "UPDATE clients SET sex = 'unspecified' WHERE id = 'client-rivera'",
            "female, male, or another",
        );
        refuses(
            &store,
            "UPDATE clients SET preferred_language = '   ' WHERE id = 'client-rivera'",
            "a name or nothing",
        );
    }

    #[test]
    fn a_client_written_before_the_demographic_columns_reads_as_not_recorded() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let path = file.path().to_path_buf();
        {
            let mut store = OfficeStore::open(&path).expect("open");
            OfficeFixture::MisdemeanorDocket
                .seed_from(&mut store, ANCHOR)
                .expect("seed");
            // Rebuild the v5 shape: every object 0006 added or recreated has
            // to go before the columns can, because SQLite refuses to drop a
            // column an index or trigger still names.
            store
                .connection
                .execute_batch(
                    "DROP TRIGGER clients_sex_vocabulary_on_insert;
                     DROP TRIGGER clients_sex_vocabulary_on_update;
                     DROP TRIGGER clients_language_is_never_blank_on_insert;
                     DROP TRIGGER clients_language_is_never_blank_on_update;
                     DROP INDEX idx_clients_preferred_language;
                     DROP TRIGGER search_documents_from_client_insert;
                     DROP TRIGGER search_documents_from_client_update;
                     DROP TRIGGER search_documents_from_alias_delete;
                     DROP TRIGGER search_documents_from_contact_delete;
                     ALTER TABLE clients DROP COLUMN sex;
                     ALTER TABLE clients DROP COLUMN preferred_language;
                     PRAGMA user_version = 5;",
                )
                .expect("rewind the schema to v5");
        }

        let store = OfficeStore::open(&path).expect("reopen migrates v5 to v6");
        let profile = store
            .client_profile("client-okonkwo")
            .expect("a client written before the columns existed");
        assert_eq!(profile.sex, None, "nobody asked, so nothing is recorded");
        assert_eq!(profile.preferred_language, None);
    }

    #[test]
    fn a_database_predating_the_version_stamp_is_still_migrated() {
        let file = tempfile::NamedTempFile::new().expect("temporary database");
        let path = file.path().to_path_buf();
        {
            let mut store = OfficeStore::open(&path).expect("open");
            OfficeFixture::MisdemeanorDocket
                .seed_from(&mut store, ANCHOR)
                .expect("seed");
            store
                .connection
                .execute_batch(
                    "DROP INDEX idx_notes_supersedes;
                     PRAGMA user_version = 0;",
                )
                .expect("rewind the stamp and drop an index");
        }

        let store = OfficeStore::open(&path).expect("reopen");
        let stamped: i64 = store
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("stamp");
        assert_eq!(stamped, SCHEMA_VERSION, "the stamp is restored");
        assert!(
            store
                .exists(
                    "SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = ?1",
                    params!["idx_notes_supersedes"],
                )
                .expect("index lookup"),
            "the dropped index came back"
        );
        assert!(
            store.note("note-rivera-plan-v2").is_ok(),
            "and the records written before the rewind are still there"
        );
    }

    #[test]
    fn records_written_before_the_search_index_existed_are_still_findable() {
        let store = seeded();
        store
            .connection
            .execute_batch("DROP TABLE office_search;")
            .expect("drop the index");
        OfficeStore::migrate(&store.connection).expect("re-run the migrations");
        let hits = store.search("Rivera", 10).expect("search after a rebuild");
        assert!(
            hits.iter().any(|hit| hit.title.contains("Rivera")),
            "a rebuilt index finds what was written before it: {hits:#?}"
        );
    }
}
