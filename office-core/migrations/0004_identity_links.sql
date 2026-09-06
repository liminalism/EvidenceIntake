PRAGMA foreign_keys = ON;

-- The client-to-evidence-entity identity link, conservative by construction.
--
-- There is no `proposed` or `candidate` state in this table, and that absence
-- is the design. A candidate pair is computed live -- one layer up, since this
-- crate cannot see the evidence kernel at all -- and never persisted. Only a
-- decision a named person actually made becomes a row. Nothing is ever merged,
-- and no link is ever inferred from a matching name.
--
-- `dismissed` earns its place by stopping a declined prompt from being offered
-- again on every future visit to the same matter. Declining leaves both the
-- client and the kernel entity exactly as they were; the only thing that
-- changes is that the office stops asking.
CREATE TABLE IF NOT EXISTS client_evidence_links (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL REFERENCES clients(id),
    evidence_case_id TEXT NOT NULL,
    evidence_entity_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('linked','dismissed')),
    matched_on TEXT,
    author_user_id TEXT NOT NULL REFERENCES users(id),
    decided_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(client_id, evidence_case_id, evidence_entity_id)
) STRICT;

-- Unlike a note, a decision here may legitimately be revisited: "not the same
-- person" today and "actually, yes" next month is ordinary work. What is fixed
-- is that a row is always written by a real decision and never inferred, so the
-- author is required on the way in and cannot be rewritten afterwards.
CREATE TRIGGER IF NOT EXISTS client_evidence_links_require_author
BEFORE INSERT ON client_evidence_links
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'an identity-link decision must name the person who made it');
END;

CREATE TRIGGER IF NOT EXISTS client_evidence_links_require_author_on_update
BEFORE UPDATE ON client_evidence_links
WHEN NEW.author_user_id IS NULL OR length(trim(NEW.author_user_id)) = 0
BEGIN
    SELECT RAISE(ABORT, 'an identity-link decision must name the person who made it');
END;

CREATE INDEX IF NOT EXISTS idx_client_evidence_links_client
  ON client_evidence_links(client_id);
CREATE INDEX IF NOT EXISTS idx_client_evidence_links_case
  ON client_evidence_links(evidence_case_id, evidence_entity_id);
