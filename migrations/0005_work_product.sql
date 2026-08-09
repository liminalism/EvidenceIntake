PRAGMA foreign_keys = ON;

-- Work product is versioned by superseding, never by overwriting. An attorney's
-- earlier reading of an issue is not a mistake to be erased: it is what they
-- thought when they made a decision, and a later reader — including the same
-- attorney — needs to be able to see that it changed and when.
--
-- `annotations` already carried `supersedes_annotation_id`. `advocacy_items`
-- carried a version number with nothing to attach it to, so revising one had no
-- representation at all; `supersedes_advocacy_id` is added by
-- `Store::add_column_if_missing` for the reasons given in 0004.

-- Every version of a work-product record names its author, for the same reason
-- every review decision does: a later reader has to be able to ask whoever
-- wrote it. These are enforced forward, on insert.
CREATE TRIGGER IF NOT EXISTS advocacy_items_require_author
BEFORE INSERT ON advocacy_items
WHEN NEW.author IS NULL OR length(trim(NEW.author)) = 0
BEGIN
    SELECT RAISE(ABORT, 'work product must name the person writing it');
END;

CREATE TRIGGER IF NOT EXISTS annotations_require_author
BEFORE INSERT ON annotations
WHEN NEW.author IS NULL OR length(trim(NEW.author)) = 0
BEGIN
    SELECT RAISE(ABORT, 'work product must name the person writing it');
END;

CREATE TRIGGER IF NOT EXISTS decision_briefs_require_author
BEFORE INSERT ON decision_briefs
WHEN NEW.author IS NULL OR length(trim(NEW.author)) = 0
BEGIN
    SELECT RAISE(ABORT, 'work product must name the person writing it');
END;

-- A version supersedes exactly one predecessor. Two rows claiming to replace the
-- same one would fork the history and leave no single current reading.
CREATE UNIQUE INDEX IF NOT EXISTS idx_advocacy_supersedes
  ON advocacy_items(supersedes_advocacy_id)
  WHERE supersedes_advocacy_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_annotations_supersedes
  ON annotations(supersedes_annotation_id)
  WHERE supersedes_annotation_id IS NOT NULL;

-- One brief per posture per version, so `ORDER BY version DESC LIMIT 1` names a
-- single current brief rather than picking arbitrarily between two.
CREATE UNIQUE INDEX IF NOT EXISTS idx_briefs_posture_version
  ON decision_briefs(case_id, posture, version);

CREATE INDEX IF NOT EXISTS idx_annotations_target
  ON annotations(case_id, target_kind, target_id);
