PRAGMA foreign_keys = ON;

-- `element_links` was created without an author, alone among the records a
-- person writes. A mapping from a proposition to a statutory element is an
-- attorney judgment like any other, and the defender reading the element matrix
-- needs to know whose reading it is. The column itself is added by
-- `Store::add_column_if_missing`, because SQLite has no re-runnable form of
-- ALTER TABLE ADD COLUMN and every migration here runs on every open.
--
-- The column is nullable at the schema level for the same reason — SQLite
-- cannot retrofit NOT NULL onto an existing table — so the guarantee is
-- enforced forward, on insert. Rows written before this migration keep a null
-- author rather than being backfilled with a name nobody actually stood behind.
CREATE TRIGGER IF NOT EXISTS element_links_require_author
BEFORE INSERT ON element_links
WHEN NEW.created_by IS NULL OR length(trim(NEW.created_by)) = 0
BEGIN
    SELECT RAISE(ABORT, 'an element mapping must name the person making it');
END;

-- One proposition bears on one element in one direction. The table's original
-- UNIQUE(element_id, proposition_id, assessment) permitted the same proposition
-- to be filed under an element as both `supports` and `opposes`, which is not a
-- richer reading but a contradictory one — `uncertain` is how the vocabulary
-- says the effect is unresolved.
CREATE UNIQUE INDEX IF NOT EXISTS idx_element_links_unique_mapping
  ON element_links(element_id, proposition_id);

CREATE INDEX IF NOT EXISTS idx_elements_charge ON elements(charge_id, ordinal);
