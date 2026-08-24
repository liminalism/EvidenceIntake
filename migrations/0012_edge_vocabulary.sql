PRAGMA foreign_keys = OFF;

-- `edges.relation` is a CHECK on a STRICT table, so widening the vocabulary
-- requires SQLite's table-rebuild procedure. Generic review events address an
-- edge by kind/id rather than a foreign key and therefore retain their trail.
CREATE TABLE edges_0012 (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relation TEXT NOT NULL CHECK(relation IN (
      'quotes','reports','summarizes','transcribes','depicts','records_utterance',
      'measures','based_on','account_of','created_after','recorded_during',
      'candidate_same_occurrence','speaker_candidate',
      'derived_from','refers_to','temporally_overlaps','possibly_same_person',
      'expected_but_missing','requires_follow_up','relevant_to',
      'supports','contradicts','qualifies','explains','impeaches',
      'consistent_with','independently_corroborates'
    )),
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    rationale TEXT,
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
      CHECK(review_state IN ('unreviewed','suggested','reviewed','verified','rejected')),
    created_by TEXT NOT NULL
) STRICT;

INSERT INTO edges_0012
    (id, case_id, source_kind, source_id, relation, target_kind, target_id,
     rationale, review_state, created_by)
SELECT id, case_id, source_kind, source_id,
       CASE relation WHEN 'corroborates' THEN 'consistent_with' ELSE relation END,
       target_kind, target_id, rationale, review_state, created_by
FROM edges;

DROP TABLE edges;
ALTER TABLE edges_0012 RENAME TO edges;

CREATE INDEX idx_edges_target
  ON edges(case_id, target_kind, target_id, relation);
CREATE UNIQUE INDEX idx_edges_unique_claim
  ON edges(case_id, source_kind, source_id, relation, target_kind, target_id);

PRAGMA foreign_keys = ON;
