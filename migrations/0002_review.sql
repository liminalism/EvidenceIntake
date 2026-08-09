PRAGMA foreign_keys = ON;

-- Append-only record of human review decisions.
--
-- Review state on an evidentiary record is a cache of the latest decision here.
-- The history is the authoritative artifact: it shows who moved an item out of
-- a machine suggestion, when, on what basis, and against which original
-- locator. Deleting a case with recorded decisions is refused rather than
-- cascaded, because the audit trail outlives convenience.
CREATE TABLE IF NOT EXISTS review_events (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id),
    target_kind TEXT NOT NULL
      CHECK(target_kind IN ('content','source','edge','proposition','event')),
    target_id TEXT NOT NULL,
    from_state TEXT NOT NULL
      CHECK(from_state IN ('unreviewed','suggested','reviewed','verified','rejected')),
    -- Only a person can produce these three states; `unreviewed` and
    -- `suggested` are intake states and are never the result of a decision.
    to_state TEXT NOT NULL CHECK(to_state IN ('reviewed','verified','rejected')),
    actor TEXT NOT NULL CHECK(length(trim(actor)) > 0),
    basis TEXT CHECK(basis IS NULL OR length(trim(basis)) > 0),
    verified_against_locator TEXT
      CHECK(verified_against_locator IS NULL OR length(trim(verified_against_locator)) > 0),
    decided_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    -- Verification is a claim about an original. It must cite the locator it
    -- was checked against, or explain why the item has no single locator.
    CHECK(to_state <> 'verified'
          OR verified_against_locator IS NOT NULL
          OR basis IS NOT NULL),
    -- Rejecting produced evidence always requires a written reason.
    CHECK(to_state <> 'rejected' OR basis IS NOT NULL)
) STRICT;

CREATE TRIGGER IF NOT EXISTS review_events_no_update
BEFORE UPDATE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review events are append-only');
END;

CREATE TRIGGER IF NOT EXISTS review_events_no_delete
BEFORE DELETE ON review_events
BEGIN
    SELECT RAISE(ABORT, 'review events are append-only');
END;

CREATE INDEX IF NOT EXISTS idx_review_events_target
  ON review_events(case_id, target_kind, target_id, decided_at);
