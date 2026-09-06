PRAGMA foreign_keys = ON;

-- Office layer schema v1. This database is a sibling of the evidence kernel's,
-- never a superset of it: no table here stores extracted content, a locator, a
-- review state, or any privileged attorney analysis. The only reference across
-- the boundary is `matters.evidence_case_id`, a bare identifier with no foreign
-- key, resolved by the application layer that holds both connections.
--
-- Portability: this schema is written for a later move to PostgreSQL behind a
-- server. `STRICT` is SQLite's way of getting the column typing PostgreSQL has
-- by default; `datetime('now')` becomes `now()`; partial indexes exist in both.
-- Dates are TEXT `YYYY-MM-DD` and times TEXT `HH:MM`, which map straight onto
-- `DATE` and `TIME`. The one construct with no direct equivalent is the FTS5
-- index in 0005, which becomes a `tsvector` column and a GIN index.
--
-- Two clocks, deliberately: business dates are local, because a court setting
-- happens on the day the courthouse says it does, and audit timestamps are UTC,
-- matching `cases.created_at` in the kernel. See `today` in `civil_date.rs`.

-- Everyone who can author something. Modeled as a real table with real rows
-- even on a single-seat installation, because the alternative -- a free-text
-- author name -- makes "no user can silently alter another author's note" a
-- convention rather than a constraint, and the server milestone would then have
-- to retrofit identity onto records already written.
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK(length(trim(display_name)) > 0),
    role TEXT NOT NULL DEFAULT 'attorney'
      CHECK(role IN ('attorney','investigator','paralegal','social_worker','supervisor','administrator')),
    email TEXT,
    active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(display_name)
) STRICT;

-- One person, however many matters. This is the office counterpart to the
-- kernel's `entities`, and the two are deliberately different: an entity is a
-- conservative per-case mention that never merges, while a client is the
-- office's record of a human being who walks in, has a phone number, and comes
-- back next year on a new charge.
CREATE TABLE IF NOT EXISTS clients (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL CHECK(length(trim(display_name)) > 0),
    date_of_birth TEXT
      CHECK(date_of_birth IS NULL OR date_of_birth GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    notes TEXT,
    author_user_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TRIGGER IF NOT EXISTS clients_require_author
BEFORE INSERT ON clients
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a client record must name the person who opened it');
END;

-- Authorship and creation time are what a later reader has to be able to trust.
-- Everything else about a client is editable; these two are not.
CREATE TRIGGER IF NOT EXISTS clients_author_is_immutable
BEFORE UPDATE ON clients
WHEN NEW.author_user_id <> OLD.author_user_id OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'who opened a record, and when, cannot be rewritten');
END;

-- Names a person also goes by. Kept as rows rather than a delimited field so
-- office search can match one without scanning.
CREATE TABLE IF NOT EXISTS client_aliases (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    alias TEXT NOT NULL CHECK(length(trim(alias)) > 0),
    UNIQUE(client_id, alias)
) STRICT;

-- `digits` holds the punctuation-free form of a phone number, because FTS5's
-- unicode61 tokenizer splits `(555) 123-4567` into three tokens and a person
-- searching `5551234567` would otherwise match nothing. It is NULL for kinds
-- where it means nothing.
CREATE TABLE IF NOT EXISTS client_contacts (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('phone','email','address','emergency','other')),
    value TEXT NOT NULL CHECK(length(trim(value)) > 0),
    digits TEXT,
    label TEXT,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK(is_primary IN (0, 1)),
    UNIQUE(client_id, kind, value)
) STRICT;

-- At most one primary per kind, so "call the client" resolves to one number.
CREATE UNIQUE INDEX IF NOT EXISTS idx_client_contacts_one_primary_per_kind
  ON client_contacts(client_id, kind) WHERE is_primary = 1;

CREATE INDEX IF NOT EXISTS idx_client_contacts_client ON client_contacts(client_id);
CREATE INDEX IF NOT EXISTS idx_client_contacts_digits ON client_contacts(digits) WHERE digits IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_client_aliases_client ON client_aliases(client_id);

-- The office's unit of work. `evidence_case_id` is the whole of the boundary:
-- a bare kernel `cases(id)` with no foreign key, nullable because most matters
-- never accumulate enough discovery to be worth collating.
CREATE TABLE IF NOT EXISTS matters (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id),
    caption TEXT NOT NULL CHECK(length(trim(caption)) > 0),
    court_number TEXT,
    court_id TEXT,
    status TEXT NOT NULL DEFAULT 'open'
      CHECK(status IN ('open','pending_appointment','closed','transferred','withdrawn')),
    custody_state TEXT NOT NULL DEFAULT 'unknown'
      CHECK(custody_state IN ('unknown','out','in_custody','released_on_bond','detained_hold')),
    offer_state TEXT NOT NULL DEFAULT 'none'
      CHECK(offer_state IN ('none','extended','under_advisement','rejected','accepted','expired')),
    offer_summary TEXT,
    charge_summary TEXT,
    opened_on TEXT
      CHECK(opened_on IS NULL OR opened_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    last_contact_on TEXT
      CHECK(last_contact_on IS NULL OR last_contact_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    evidence_case_id TEXT,
    author_user_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TRIGGER IF NOT EXISTS matters_require_author
BEFORE INSERT ON matters
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a matter must name the person who opened it');
END;

CREATE TRIGGER IF NOT EXISTS matters_author_is_immutable
BEFORE UPDATE ON matters
WHEN NEW.author_user_id <> OLD.author_user_id OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'who opened a record, and when, cannot be rewritten');
END;

CREATE INDEX IF NOT EXISTS idx_matters_client ON matters(client_id);
CREATE INDEX IF NOT EXISTS idx_matters_open ON matters(status) WHERE status <> 'closed';
CREATE INDEX IF NOT EXISTS idx_matters_evidence_case
  ON matters(evidence_case_id) WHERE evidence_case_id IS NOT NULL;

-- Related matters of the same person. The relation is descriptive; nothing in
-- the office layer derives consequences from it beyond offering the other
-- matters when a setting is scheduled.
CREATE TABLE IF NOT EXISTS matter_links (
    id TEXT PRIMARY KEY,
    matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    related_matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    relation TEXT NOT NULL DEFAULT 'related'
      CHECK(relation IN ('related','consolidated','probation_violation','companion','refiled')),
    CHECK(matter_id <> related_matter_id),
    UNIQUE(matter_id, related_matter_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_matter_links_matter ON matter_links(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_links_related ON matter_links(related_matter_id);

-- Who is on the matter. `assigned_by_user_id` costs nothing now and answers the
-- question an office eventually asks about every staffing decision.
CREATE TABLE IF NOT EXISTS matter_assignments (
    id TEXT PRIMARY KEY,
    matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id),
    role TEXT NOT NULL
      CHECK(role IN ('attorney','second_chair','investigator','paralegal','social_worker','supervisor')),
    assigned_by_user_id TEXT NOT NULL REFERENCES users(id),
    assigned_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(matter_id, user_id, role)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_matter_assignments_user ON matter_assignments(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_assignments_matter ON matter_assignments(matter_id);
