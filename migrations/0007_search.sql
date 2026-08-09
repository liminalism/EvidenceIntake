-- Full-text search over extracted content.
--
-- An external-content FTS5 table: the index stores no copy of the text, only
-- the terms, and reads the words back out of `content` itself. That matters
-- more here than the disk it saves. The originals stay the one place the words
-- live, and a search index that held its own copy would be a second version of
-- an evidentiary record that could drift from the first.
--
-- The triggers keep the index inside the same transaction as the write, so a
-- committed excerpt is always findable and a rolled-back one never is. This is
-- why search belongs in SQLite rather than beside it: a separate engine would
-- be a second store that can disagree with the append-only trail.
--
-- Only `content` is indexed. Advocacy items, annotations, and decision briefs
-- are privileged, and a search path that reached them would be a way for
-- attorney analysis to surface in a place that does not know it is privileged.

CREATE VIRTUAL TABLE IF NOT EXISTS content_search USING fts5(
    text,
    content = 'content',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS content_search_after_insert
AFTER INSERT ON content BEGIN
    INSERT INTO content_search(rowid, text) VALUES (new.rowid, new.text);
END;

CREATE TRIGGER IF NOT EXISTS content_search_after_delete
AFTER DELETE ON content BEGIN
    INSERT INTO content_search(content_search, rowid, text)
      VALUES ('delete', old.rowid, old.text);
END;

CREATE TRIGGER IF NOT EXISTS content_search_after_update
AFTER UPDATE ON content BEGIN
    INSERT INTO content_search(content_search, rowid, text)
      VALUES ('delete', old.rowid, old.text);
    INSERT INTO content_search(rowid, text) VALUES (new.rowid, new.text);
END;

-- Backfill anything written before the index existed. Rebuilding is idempotent
-- and runs only when the schema version advances, not on every open.
INSERT INTO content_search(content_search) VALUES ('rebuild');
