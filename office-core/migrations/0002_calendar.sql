PRAGMA foreign_keys = ON;

-- The court calendar. These are operational schedule rows and nothing else:
-- they say where a person has to be and when. They are not the kernel's
-- `events`, which express competing accounts of what happened and never
-- collapse into one authoritative sequence. The two never merge, and no query
-- here reaches into the evidence database.

CREATE TABLE IF NOT EXISTS courts (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL CHECK(length(trim(name)) > 0),
    division TEXT,
    address TEXT,
    room TEXT,
    UNIQUE(name, division)
) STRICT;

-- Judges are plain reference rows. Nothing in this milestone reasons about a
-- judge; the column exists so a docket row can say whose courtroom it is.
CREATE TABLE IF NOT EXISTS judges (
    id TEXT PRIMARY KEY,
    court_id TEXT REFERENCES courts(id),
    display_name TEXT NOT NULL CHECK(length(trim(display_name)) > 0),
    UNIQUE(court_id, display_name)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_judges_court ON judges(court_id);

-- One setting. It belongs to a client, not to a matter, which is the whole
-- point: a person with three open cases called for the same 9:00 docket has one
-- place to be, and a calendar that renders three rows for it is the
-- duplicate-setting error this table exists to make impossible.
CREATE TABLE IF NOT EXISTS appearances (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id),
    court_id TEXT REFERENCES courts(id),
    judge_id TEXT REFERENCES judges(id),
    appearance_date TEXT NOT NULL
      CHECK(appearance_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    appearance_time TEXT
      CHECK(appearance_time IS NULL OR appearance_time GLOB '[0-2][0-9]:[0-5][0-9]'),
    appearance_type TEXT NOT NULL DEFAULT 'status'
      CHECK(appearance_type IN ('arraignment','status','pretrial','motion','plea',
                                'trial','sentencing','review','violation','other')),
    outcome TEXT,
    notes TEXT,
    -- A cancelled setting is struck, not deleted. Notes may already hang off
    -- it and notes are append-only, so a hard delete would either fail on the
    -- foreign key or take an author's words with it. Every docket read filters
    -- on this column.
    cancelled INTEGER NOT NULL DEFAULT 0 CHECK(cancelled IN (0, 1)),
    cancelled_on TEXT
      CHECK(cancelled_on IS NULL OR cancelled_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    author_user_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK((cancelled = 0 AND cancelled_on IS NULL) OR (cancelled = 1 AND cancelled_on IS NOT NULL))
) STRICT;

CREATE TRIGGER IF NOT EXISTS appearances_require_author
BEFORE INSERT ON appearances
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a court setting must name the person who scheduled it');
END;

CREATE TRIGGER IF NOT EXISTS appearances_author_is_immutable
BEFORE UPDATE ON appearances
WHEN NEW.author_user_id <> OLD.author_user_id OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'who opened a record, and when, cannot be rewritten');
END;

CREATE INDEX IF NOT EXISTS idx_appearances_date
  ON appearances(appearance_date, appearance_time) WHERE cancelled = 0;
CREATE INDEX IF NOT EXISTS idx_appearances_client ON appearances(client_id, appearance_date);

-- Which of the client's matters this setting covers. `override_note` is how a
-- matter is handled differently within a setting the rest of them share --
-- "continued to the 14th", "passed for plea" -- without splitting the setting
-- into two rows and reintroducing the duplication.
CREATE TABLE IF NOT EXISTS appearance_matters (
    id TEXT PRIMARY KEY,
    appearance_id TEXT NOT NULL REFERENCES appearances(id) ON DELETE CASCADE,
    matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    override_note TEXT,
    UNIQUE(appearance_id, matter_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_appearance_matters_matter ON appearance_matters(matter_id);
CREATE INDEX IF NOT EXISTS idx_appearance_matters_appearance ON appearance_matters(appearance_id);

-- A setting spans the matters of one client and no one else's. Without this a
-- mis-typed identifier silently puts another person's case on someone's docket
-- row, which is the kind of error that is only ever found in the courtroom.
CREATE TRIGGER IF NOT EXISTS appearance_matters_same_client
BEFORE INSERT ON appearance_matters
WHEN (SELECT client_id FROM matters WHERE id = NEW.matter_id)
     IS NOT (SELECT client_id FROM appearances WHERE id = NEW.appearance_id)
BEGIN
    SELECT RAISE(ABORT, 'a setting spans the matters of one client');
END;

-- A setting with no matter left on it is a calendar entry that means nothing,
-- so unlinking the last one is refused. Cancelling a setting is a different act
-- and goes through `appearances.cancelled`, which is why nothing here has to
-- reason about a delete of the parent: an appearance is struck, never removed.
CREATE TRIGGER IF NOT EXISTS appearance_matters_keep_at_least_one
BEFORE DELETE ON appearance_matters
WHEN EXISTS (SELECT 1 FROM appearances WHERE id = OLD.appearance_id)
 AND (SELECT count(*) FROM appearance_matters WHERE appearance_id = OLD.appearance_id) <= 1
BEGIN
    SELECT RAISE(ABORT, 'a setting cannot be emptied of every matter; cancel the setting instead');
END;

-- What is owed and when. `origin` is the distinction that decides whether a
-- date can move: a statutory deadline cannot be renegotiated, a self-imposed
-- one is the defender's own working target.
CREATE TABLE IF NOT EXISTS deadlines (
    id TEXT PRIMARY KEY,
    matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    description TEXT NOT NULL CHECK(length(trim(description)) > 0),
    due_date TEXT NOT NULL
      CHECK(due_date GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    origin TEXT NOT NULL
      CHECK(origin IN ('statutory','court_ordered','self_imposed')),
    satisfied INTEGER NOT NULL DEFAULT 0 CHECK(satisfied IN (0, 1)),
    satisfied_on TEXT
      CHECK(satisfied_on IS NULL OR satisfied_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]'),
    author_user_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK((satisfied = 0 AND satisfied_on IS NULL) OR (satisfied = 1 AND satisfied_on IS NOT NULL))
) STRICT;

CREATE TRIGGER IF NOT EXISTS deadlines_require_author
BEFORE INSERT ON deadlines
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a deadline must name the person who recorded it');
END;

CREATE TRIGGER IF NOT EXISTS deadlines_author_is_immutable
BEFORE UPDATE ON deadlines
WHEN NEW.author_user_id <> OLD.author_user_id OR NEW.created_at <> OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'who opened a record, and when, cannot be rewritten');
END;

CREATE INDEX IF NOT EXISTS idx_deadlines_matter ON deadlines(matter_id, due_date);
CREATE INDEX IF NOT EXISTS idx_deadlines_open ON deadlines(due_date) WHERE satisfied = 0;
