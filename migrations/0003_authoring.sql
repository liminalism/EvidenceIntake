PRAGMA foreign_keys = ON;

-- Review state is constrained by a CHECK on `content`, `sources`, and `edges`,
-- but `propositions`, `events`, and `entity_mentions` were created without one.
-- A CHECK cannot be added to an existing STRICT table without rebuilding it, so
-- the same guarantee is enforced by triggers instead. Propositions and events
-- are both review targets; nothing may write them into a state the review
-- vocabulary does not contain.

CREATE TRIGGER IF NOT EXISTS propositions_review_state_insert
BEFORE INSERT ON propositions
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on proposition');
END;

CREATE TRIGGER IF NOT EXISTS propositions_review_state_update
BEFORE UPDATE ON propositions
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on proposition');
END;

CREATE TRIGGER IF NOT EXISTS events_review_state_insert
BEFORE INSERT ON events
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on event');
END;

CREATE TRIGGER IF NOT EXISTS events_review_state_update
BEFORE UPDATE ON events
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on event');
END;

CREATE TRIGGER IF NOT EXISTS entity_mentions_review_state_insert
BEFORE INSERT ON entity_mentions
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on entity mention');
END;

CREATE TRIGGER IF NOT EXISTS entity_mentions_review_state_update
BEFORE UPDATE ON entity_mentions
WHEN NEW.review_state NOT IN ('unreviewed','suggested','reviewed','verified','rejected')
BEGIN
    SELECT RAISE(ABORT, 'unrecognized review state on entity mention');
END;

-- One relationship, asserted once. Writing the same claim twice would put two
-- rows in front of a reviewer that say the same thing and would double-count it
-- in every view; the second assertion is refused so the first keeps its own
-- rationale and review history.
CREATE UNIQUE INDEX IF NOT EXISTS idx_edges_unique_claim
  ON edges(case_id, source_kind, source_id, relation, target_kind, target_id);

-- Authoring reads back propositions by their author; the review queue and the
-- element matrix both scan them per case.
CREATE INDEX IF NOT EXISTS idx_propositions_case ON propositions(case_id, status);
