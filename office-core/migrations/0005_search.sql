PRAGMA foreign_keys = ON;

-- Office search, which answers the retrieval questions evidence search
-- deliberately cannot: a person's name across every matter they have ever had,
-- a phone number, a court case number, a colleague's client.
--
-- Evidence search is case-isolated by design and indexes `content` alone. This
-- index is its opposite in scope and its twin in discipline: it spans the whole
-- office, and it reaches nothing but office tables. There is no path from here
-- to an advocacy item, an annotation, or a decision brief -- not because a
-- filter excludes them, but because this crate has no connection to the
-- database that holds them.
--
-- Unlike the kernel's external-content index, this one is fed by a shadow table
-- rather than reading a single source table back. Five different office tables
-- contribute to one searchable surface, and an external-content FTS5 table can
-- only follow one. `search_documents` is that one, kept current by triggers on
-- each contributor, so the index cannot disagree with the records.
--
-- PostgreSQL equivalent: `search_documents` stays as it is, gains a `tsvector`
-- column, and the FTS5 table and its three sync triggers become one GIN index.
CREATE TABLE IF NOT EXISTS search_documents (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('client','matter','note','user','court')),
    subject_id TEXT NOT NULL,
    title TEXT NOT NULL,
    text TEXT NOT NULL,
    UNIQUE(kind, subject_id)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_search_documents_subject
  ON search_documents(kind, subject_id);

CREATE VIRTUAL TABLE IF NOT EXISTS office_search USING fts5(
    title,
    text,
    content = 'search_documents',
    content_rowid = 'rowid',
    tokenize = 'unicode61 remove_diacritics 2'
);

CREATE TRIGGER IF NOT EXISTS office_search_after_insert
AFTER INSERT ON search_documents BEGIN
    INSERT INTO office_search(rowid, title, text)
      VALUES (new.rowid, new.title, new.text);
END;

CREATE TRIGGER IF NOT EXISTS office_search_after_delete
AFTER DELETE ON search_documents BEGIN
    INSERT INTO office_search(office_search, rowid, title, text)
      VALUES ('delete', old.rowid, old.title, old.text);
END;

CREATE TRIGGER IF NOT EXISTS office_search_after_update
AFTER UPDATE ON search_documents BEGIN
    INSERT INTO office_search(office_search, rowid, title, text)
      VALUES ('delete', old.rowid, old.title, old.text);
    INSERT INTO office_search(rowid, title, text)
      VALUES (new.rowid, new.title, new.text);
END;

-- Contributors. Each keeps one `search_documents` row current for its subject.
-- A client's searchable text gathers the person's own aliases and contacts, so
-- finding someone by an old surname or a phone number is one MATCH rather than
-- a join across three tables. `digits` is folded in beside the formatted value
-- for the reason given in 0001: unicode61 splits punctuation, so an unpunctuated
-- query would otherwise miss a punctuated number.

CREATE TRIGGER IF NOT EXISTS search_documents_from_client_insert
AFTER INSERT ON clients BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'client', new.id, new.display_name,
              new.display_name || ' ' || COALESCE(new.date_of_birth, '') || ' '
                || COALESCE(new.notes, ''))
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_client_update
AFTER UPDATE ON clients BEGIN
    UPDATE search_documents
       SET title = new.display_name,
           text = new.display_name || ' ' || COALESCE(new.date_of_birth, '') || ' '
                    || COALESCE(new.notes, '') || ' '
                    || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                  WHERE client_id = new.id), '') || ' '
                    || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                   FROM client_contacts WHERE client_id = new.id), '')
     WHERE kind = 'client' AND subject_id = new.id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_alias_insert
AFTER INSERT ON client_aliases BEGIN
    UPDATE search_documents SET text = text || ' ' || new.alias
     WHERE kind = 'client' AND subject_id = new.client_id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_alias_delete
AFTER DELETE ON client_aliases BEGIN
    UPDATE search_documents
       SET text = (SELECT display_name || ' ' || COALESCE(date_of_birth, '') || ' '
                          || COALESCE(notes, '') FROM clients WHERE id = old.client_id)
                  || ' ' || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                       WHERE client_id = old.client_id), '')
                  || ' ' || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                        FROM client_contacts WHERE client_id = old.client_id), '')
     WHERE kind = 'client' AND subject_id = old.client_id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_contact_insert
AFTER INSERT ON client_contacts BEGIN
    UPDATE search_documents
       SET text = text || ' ' || new.value || ' ' || COALESCE(new.digits, '')
     WHERE kind = 'client' AND subject_id = new.client_id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_contact_delete
AFTER DELETE ON client_contacts BEGIN
    UPDATE search_documents
       SET text = (SELECT display_name || ' ' || COALESCE(date_of_birth, '') || ' '
                          || COALESCE(notes, '') FROM clients WHERE id = old.client_id)
                  || ' ' || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                       WHERE client_id = old.client_id), '')
                  || ' ' || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                        FROM client_contacts WHERE client_id = old.client_id), '')
     WHERE kind = 'client' AND subject_id = old.client_id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_matter_insert
AFTER INSERT ON matters BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'matter', new.id, new.caption,
              new.caption || ' ' || COALESCE(new.court_number, '') || ' '
                || COALESCE(new.charge_summary, '') || ' '
                || COALESCE((SELECT display_name FROM clients WHERE id = new.client_id), ''))
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_matter_update
AFTER UPDATE ON matters BEGIN
    UPDATE search_documents
       SET title = new.caption,
           text = new.caption || ' ' || COALESCE(new.court_number, '') || ' '
                    || COALESCE(new.charge_summary, '') || ' '
                    || COALESCE((SELECT display_name FROM clients WHERE id = new.client_id), '')
     WHERE kind = 'matter' AND subject_id = new.id;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_note_insert
AFTER INSERT ON notes BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'note', new.id,
              'Note ' || COALESCE(new.client_id, new.matter_id, new.appearance_id),
              new.body)
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_user_insert
AFTER INSERT ON users BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'user', new.id, new.display_name,
              new.display_name || ' ' || new.role || ' ' || COALESCE(new.email, ''))
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

CREATE TRIGGER IF NOT EXISTS search_documents_from_court_insert
AFTER INSERT ON courts BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'court', new.id, new.name,
              new.name || ' ' || COALESCE(new.division, '') || ' '
                || COALESCE(new.address, '') || ' ' || COALESCE(new.room, ''))
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

-- Backfill anything written before the index existed. Rebuilding is idempotent
-- and runs only when the schema version advances, not on every open.
INSERT INTO office_search(office_search) VALUES ('rebuild');
