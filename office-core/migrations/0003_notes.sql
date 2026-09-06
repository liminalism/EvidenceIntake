PRAGMA foreign_keys = ON;

-- Office notes, scoped to exactly one of a client, a matter, or an appearance.
-- A client note follows the person across every matter they ever have; a matter
-- note stays with the case; an appearance note belongs to one setting.
--
-- The kernel versions work product by superseding rather than overwriting
-- (migrations/0005_work_product.sql). Notes go one step further and make the
-- row append-only at the schema level, because the rule here is stronger than
-- "a later reader can see it changed": no user may silently alter another
-- author's note, and the cheapest way to guarantee that is to make altering one
-- in place impossible for everybody, including the author.
CREATE TABLE IF NOT EXISTS notes (
    id TEXT PRIMARY KEY,
    client_id TEXT REFERENCES clients(id),
    matter_id TEXT REFERENCES matters(id),
    appearance_id TEXT REFERENCES appearances(id),
    body TEXT NOT NULL CHECK(length(trim(body)) > 0),
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    supersedes_note_id TEXT REFERENCES notes(id),
    author_user_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK(
        (client_id IS NOT NULL AND matter_id IS NULL AND appearance_id IS NULL) OR
        (client_id IS NULL AND matter_id IS NOT NULL AND appearance_id IS NULL) OR
        (client_id IS NULL AND matter_id IS NULL AND appearance_id IS NOT NULL)
    )
) STRICT;

CREATE TRIGGER IF NOT EXISTS notes_require_author
BEFORE INSERT ON notes
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a note must name the person writing it');
END;

CREATE TRIGGER IF NOT EXISTS notes_are_never_updated
BEFORE UPDATE ON notes
BEGIN
    SELECT RAISE(ABORT, 'notes are append-only; write a superseding revision instead');
END;

CREATE TRIGGER IF NOT EXISTS notes_are_never_deleted
BEFORE DELETE ON notes
BEGIN
    SELECT RAISE(ABORT, 'notes are append-only; write a superseding revision instead');
END;

-- A revision supersedes exactly one predecessor. Two rows claiming to replace
-- the same note would fork the history and leave no single current reading --
-- the same idiom as `idx_annotations_supersedes` in the kernel.
CREATE UNIQUE INDEX IF NOT EXISTS idx_notes_supersedes
  ON notes(supersedes_note_id) WHERE supersedes_note_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_notes_client
  ON notes(client_id, created_at) WHERE client_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_notes_matter
  ON notes(matter_id, created_at) WHERE matter_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_notes_appearance
  ON notes(appearance_id, created_at) WHERE appearance_id IS NOT NULL;

-- Mentions ride on notes, parsed once at write time into a queryable side table
-- rather than re-scanned out of `body` on every read. A revision re-derives its
-- own mentions from its own text; nothing edits a mention alone.
CREATE TABLE IF NOT EXISTS note_mentions (
    id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    tag TEXT NOT NULL
      CHECK(tag IN ('investigator','socialwork','immigration','supervisor')),
    UNIQUE(note_id, tag)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_note_mentions_tag ON note_mentions(tag, note_id);
