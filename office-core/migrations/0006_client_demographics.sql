PRAGMA foreign_keys = ON;

-- Client demographics: sex and preferred language, retrofitted onto `clients`.
--
-- Both columns are added by `add_column_if_missing` in Rust before this batch
-- runs, and both are nullable by necessity: SQLite cannot add a NOT NULL or a
-- CHECK constraint to a table that already has rows. Absence is a value -- a
-- client with no sex or language is one nobody asked, which is not the same as
-- one who answered. The sex vocabulary is enforced forward by triggers, the
-- kernel's pattern for every retrofitted guarantee.
--
-- Preferred language is deliberately NOT a vocabulary. An office serves
-- whoever walks in, and a CHECK over a list of languages would refuse a real
-- answer. The only rule a language value carries is that blank is not one:
-- record the language or record nothing.
--
-- Sex is deliberately NOT in the search index. It is a demographic fact on an
-- intake sheet, not a retrieval key; the language is searchable because "the
-- Somali-speaking client from Tuesday" is a real question an office asks.

CREATE TRIGGER IF NOT EXISTS clients_sex_vocabulary_on_insert
BEFORE INSERT ON clients
WHEN NEW.sex IS NOT NULL AND NEW.sex NOT IN ('female', 'male', 'another')
BEGIN
    SELECT RAISE(ABORT, 'sex is female, male, or another');
END;

CREATE TRIGGER IF NOT EXISTS clients_sex_vocabulary_on_update
BEFORE UPDATE OF sex ON clients
WHEN NEW.sex IS NOT NULL AND NEW.sex NOT IN ('female', 'male', 'another')
BEGIN
    SELECT RAISE(ABORT, 'sex is female, male, or another');
END;

CREATE TRIGGER IF NOT EXISTS clients_language_is_never_blank_on_insert
BEFORE INSERT ON clients
WHEN NEW.preferred_language IS NOT NULL AND length(trim(NEW.preferred_language)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a preferred language is a name or nothing');
END;

CREATE TRIGGER IF NOT EXISTS clients_language_is_never_blank_on_update
BEFORE UPDATE OF preferred_language ON clients
WHEN NEW.preferred_language IS NOT NULL AND length(trim(NEW.preferred_language)) = 0
BEGIN
    SELECT RAISE(ABORT, 'a preferred language is a name or nothing');
END;

CREATE INDEX IF NOT EXISTS idx_clients_preferred_language
  ON clients(preferred_language) WHERE preferred_language IS NOT NULL;

-- The client search document gains the preferred language. Every trigger that
-- rebuilds the whole text is recreated with the extra column; the append-only
-- triggers (alias insert, contact insert) are untouched because they extend
-- text a rebuilt form already carries.

DROP TRIGGER IF EXISTS search_documents_from_client_insert;
CREATE TRIGGER search_documents_from_client_insert
AFTER INSERT ON clients BEGIN
    INSERT INTO search_documents(id, kind, subject_id, title, text)
      VALUES (new.id, 'client', new.id, new.display_name,
              new.display_name || ' ' || COALESCE(new.date_of_birth, '') || ' '
                || COALESCE(new.preferred_language, '') || ' '
                || COALESCE(new.notes, ''))
      ON CONFLICT(kind, subject_id) DO UPDATE
        SET title = excluded.title, text = excluded.text;
END;

DROP TRIGGER IF EXISTS search_documents_from_client_update;
CREATE TRIGGER search_documents_from_client_update
AFTER UPDATE ON clients BEGIN
    UPDATE search_documents
       SET title = new.display_name,
           text = new.display_name || ' ' || COALESCE(new.date_of_birth, '') || ' '
                    || COALESCE(new.preferred_language, '') || ' '
                    || COALESCE(new.notes, '') || ' '
                    || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                  WHERE client_id = new.id), '') || ' '
                    || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                   FROM client_contacts WHERE client_id = new.id), '')
     WHERE kind = 'client' AND subject_id = new.id;
END;

DROP TRIGGER IF EXISTS search_documents_from_alias_delete;
CREATE TRIGGER search_documents_from_alias_delete
AFTER DELETE ON client_aliases BEGIN
    UPDATE search_documents
       SET text = (SELECT display_name || ' ' || COALESCE(date_of_birth, '') || ' '
                          || COALESCE(preferred_language, '') || ' '
                          || COALESCE(notes, '') FROM clients WHERE id = old.client_id)
                  || ' ' || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                       WHERE client_id = old.client_id), '')
                  || ' ' || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                        FROM client_contacts WHERE client_id = old.client_id), '')
     WHERE kind = 'client' AND subject_id = old.client_id;
END;

DROP TRIGGER IF EXISTS search_documents_from_contact_delete;
CREATE TRIGGER search_documents_from_contact_delete
AFTER DELETE ON client_contacts BEGIN
    UPDATE search_documents
       SET text = (SELECT display_name || ' ' || COALESCE(date_of_birth, '') || ' '
                          || COALESCE(preferred_language, '') || ' '
                          || COALESCE(notes, '') FROM clients WHERE id = old.client_id)
                  || ' ' || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                       WHERE client_id = old.client_id), '')
                  || ' ' || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                        FROM client_contacts WHERE client_id = old.client_id), '')
     WHERE kind = 'client' AND subject_id = old.client_id;
END;

-- Recompute every client document once, so a database migrated from v5 indexes
-- the language of clients written before the column existed. A full recompute
-- rather than an append: re-running this migration cannot double the text.
UPDATE search_documents
   SET text = (SELECT display_name || ' ' || COALESCE(date_of_birth, '') || ' '
                      || COALESCE(preferred_language, '') || ' '
                      || COALESCE(notes, '') FROM clients WHERE id = search_documents.subject_id)
              || ' ' || COALESCE((SELECT group_concat(alias, ' ') FROM client_aliases
                                   WHERE client_id = search_documents.subject_id), '')
              || ' ' || COALESCE((SELECT group_concat(value || ' ' || COALESCE(digits, ''), ' ')
                                    FROM client_contacts
                                    WHERE client_id = search_documents.subject_id), '')
 WHERE kind = 'client';
