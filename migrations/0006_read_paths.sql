-- Indexes serving the read paths, added after the read models settled.
--
-- Nothing here changes what the kernel stores or what it will accept. Each
-- index answers a query that was scanning a whole table, and each is written so
-- it stays small: the review indexes are partial, covering only the two intake
-- states, so they hold the work queue and nothing else. A case whose material
-- has all been reviewed carries almost no index at all.

-- `review_queue` and the `pending_review` count read exactly these rows, across
-- five tables. Partial on the intake states because a reviewed record is never
-- looked up this way, and because only a person moves a row out of them --
-- which means these indexes shrink as the case is worked rather than growing.
CREATE INDEX IF NOT EXISTS idx_content_pending
  ON content(case_id, id) WHERE review_state IN ('unreviewed','suggested');

CREATE INDEX IF NOT EXISTS idx_sources_pending
  ON sources(case_id, id) WHERE review_state IN ('unreviewed','suggested');

CREATE INDEX IF NOT EXISTS idx_edges_pending
  ON edges(case_id, id) WHERE review_state IN ('unreviewed','suggested');

CREATE INDEX IF NOT EXISTS idx_propositions_pending
  ON propositions(case_id, id) WHERE review_state IN ('unreviewed','suggested');

CREATE INDEX IF NOT EXISTS idx_events_pending
  ON events(case_id, id) WHERE review_state IN ('unreviewed','suggested');

-- `element_links` is keyed (element_id, proposition_id), which answers "what
-- bears on this element" but not "is this proposition mapped anywhere", which
-- is what the unmapped-proposition finding asks once per proposition.
CREATE INDEX IF NOT EXISTS idx_element_links_proposition
  ON element_links(proposition_id);

-- `duplicate-entity` compares people, and only people.
CREATE INDEX IF NOT EXISTS idx_entities_case_kind ON entities(case_id, kind);
