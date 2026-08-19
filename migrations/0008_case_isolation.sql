PRAGMA foreign_keys = ON;

-- Cases do not share records. Most tables carry case_id, but several foreign
-- keys name a row by id alone, so a speaker, a parent, a superseded original,
-- or an element mapping could otherwise point at another case's material.
--
-- These triggers fire only when the named row *exists in a different case*.
-- A missing endpoint is a separate problem (a foreign key, or the store API).
-- Refusing a not-yet-inserted endpoint would also break fixtures that write
-- edges before the advocacy items they name.

CREATE TRIGGER IF NOT EXISTS content_speaker_same_case_insert
BEFORE INSERT ON content
WHEN NEW.speaker_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   WHERE e.id = NEW.speaker_entity_id AND e.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'content speaker must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_speaker_same_case_update
BEFORE UPDATE ON content
WHEN NEW.speaker_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   WHERE e.id = NEW.speaker_entity_id AND e.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'content speaker must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_attributed_same_case_insert
BEFORE INSERT ON content
WHEN NEW.attributed_to_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   WHERE e.id = NEW.attributed_to_entity_id AND e.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'attributed person must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_attributed_same_case_update
BEFORE UPDATE ON content
WHEN NEW.attributed_to_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   WHERE e.id = NEW.attributed_to_entity_id AND e.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'attributed person must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_parent_same_case_insert
BEFORE INSERT ON content
WHEN NEW.parent_content_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM content parent
   WHERE parent.id = NEW.parent_content_id AND parent.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'parent content must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_parent_same_case_update
BEFORE UPDATE ON content
WHEN NEW.parent_content_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM content parent
   WHERE parent.id = NEW.parent_content_id AND parent.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'parent content must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_segment_same_case_insert
BEFORE INSERT ON content
WHEN EXISTS (
   SELECT 1 FROM source_segments seg
   JOIN sources s ON s.id = seg.source_id
   WHERE seg.id = NEW.segment_id AND s.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'content segment must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS content_segment_same_case_update
BEFORE UPDATE ON content
WHEN EXISTS (
   SELECT 1 FROM source_segments seg
   JOIN sources s ON s.id = seg.source_id
   WHERE seg.id = NEW.segment_id AND s.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'content segment must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS sources_production_same_case_insert
BEFORE INSERT ON sources
WHEN NEW.production_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM productions p
   WHERE p.id = NEW.production_id AND p.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'production must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS sources_production_same_case_update
BEFORE UPDATE ON sources
WHEN NEW.production_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM productions p
   WHERE p.id = NEW.production_id AND p.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'production must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS sources_supersedes_same_case_insert
BEFORE INSERT ON sources
WHEN NEW.supersedes_source_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM sources prior
   WHERE prior.id = NEW.supersedes_source_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded source must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS sources_supersedes_same_case_update
BEFORE UPDATE ON sources
WHEN NEW.supersedes_source_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM sources prior
   WHERE prior.id = NEW.supersedes_source_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded source must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS events_proposition_same_case_insert
BEFORE INSERT ON events
WHEN NEW.proposition_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM propositions p
   WHERE p.id = NEW.proposition_id AND p.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'event proposition must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS events_proposition_same_case_update
BEFORE UPDATE ON events
WHEN NEW.proposition_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM propositions p
   WHERE p.id = NEW.proposition_id AND p.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'event proposition must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS entity_mentions_resolved_same_case_insert
BEFORE INSERT ON entity_mentions
WHEN NEW.resolved_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   JOIN source_segments seg ON seg.id = NEW.segment_id
   JOIN sources s ON s.id = seg.source_id
   WHERE e.id = NEW.resolved_entity_id AND e.case_id <> s.case_id)
BEGIN
    SELECT RAISE(ABORT, 'resolved entity must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS entity_mentions_resolved_same_case_update
BEFORE UPDATE ON entity_mentions
WHEN NEW.resolved_entity_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM entities e
   JOIN source_segments seg ON seg.id = NEW.segment_id
   JOIN sources s ON s.id = seg.source_id
   WHERE e.id = NEW.resolved_entity_id AND e.case_id <> s.case_id)
BEGIN
    SELECT RAISE(ABORT, 'resolved entity must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS element_links_same_case_insert
BEFORE INSERT ON element_links
WHEN EXISTS (
   SELECT 1 FROM elements el
   JOIN charges ch ON ch.id = el.charge_id
   JOIN propositions p ON p.id = NEW.proposition_id
   WHERE el.id = NEW.element_id AND ch.case_id <> p.case_id)
BEGIN
    SELECT RAISE(ABORT, 'element mapping must stay inside one case');
END;

CREATE TRIGGER IF NOT EXISTS element_links_same_case_update
BEFORE UPDATE ON element_links
WHEN EXISTS (
   SELECT 1 FROM elements el
   JOIN charges ch ON ch.id = el.charge_id
   JOIN propositions p ON p.id = NEW.proposition_id
   WHERE el.id = NEW.element_id AND ch.case_id <> p.case_id)
BEGIN
    SELECT RAISE(ABORT, 'element mapping must stay inside one case');
END;

CREATE TRIGGER IF NOT EXISTS advocacy_supersedes_same_case_insert
BEFORE INSERT ON advocacy_items
WHEN NEW.supersedes_advocacy_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM advocacy_items prior
   WHERE prior.id = NEW.supersedes_advocacy_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded work product must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS advocacy_supersedes_same_case_update
BEFORE UPDATE ON advocacy_items
WHEN NEW.supersedes_advocacy_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM advocacy_items prior
   WHERE prior.id = NEW.supersedes_advocacy_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded work product must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS annotations_supersedes_same_case_insert
BEFORE INSERT ON annotations
WHEN NEW.supersedes_annotation_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM annotations prior
   WHERE prior.id = NEW.supersedes_annotation_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded note must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS annotations_supersedes_same_case_update
BEFORE UPDATE ON annotations
WHEN NEW.supersedes_annotation_id IS NOT NULL
 AND EXISTS (
   SELECT 1 FROM annotations prior
   WHERE prior.id = NEW.supersedes_annotation_id AND prior.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'superseded note must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS edges_source_same_case_insert
BEFORE INSERT ON edges
WHEN
    (NEW.source_kind = 'content' AND EXISTS (SELECT 1 FROM content WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'source' AND EXISTS (SELECT 1 FROM sources WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'proposition' AND EXISTS (SELECT 1 FROM propositions WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'event' AND EXISTS (SELECT 1 FROM events WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'edge' AND EXISTS (SELECT 1 FROM edges WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'entity' AND EXISTS (SELECT 1 FROM entities WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'advocacy' AND EXISTS (SELECT 1 FROM advocacy_items WHERE id = NEW.source_id AND case_id <> NEW.case_id))
BEGIN
    SELECT RAISE(ABORT, 'edge source must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS edges_source_same_case_update
BEFORE UPDATE ON edges
WHEN
    (NEW.source_kind = 'content' AND EXISTS (SELECT 1 FROM content WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'source' AND EXISTS (SELECT 1 FROM sources WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'proposition' AND EXISTS (SELECT 1 FROM propositions WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'event' AND EXISTS (SELECT 1 FROM events WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'edge' AND EXISTS (SELECT 1 FROM edges WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'entity' AND EXISTS (SELECT 1 FROM entities WHERE id = NEW.source_id AND case_id <> NEW.case_id))
 OR (NEW.source_kind = 'advocacy' AND EXISTS (SELECT 1 FROM advocacy_items WHERE id = NEW.source_id AND case_id <> NEW.case_id))
BEGIN
    SELECT RAISE(ABORT, 'edge source must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS edges_target_same_case_insert
BEFORE INSERT ON edges
WHEN
    (NEW.target_kind = 'content' AND EXISTS (SELECT 1 FROM content WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'source' AND EXISTS (SELECT 1 FROM sources WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'proposition' AND EXISTS (SELECT 1 FROM propositions WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'event' AND EXISTS (SELECT 1 FROM events WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'edge' AND EXISTS (SELECT 1 FROM edges WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'entity' AND EXISTS (SELECT 1 FROM entities WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'advocacy' AND EXISTS (SELECT 1 FROM advocacy_items WHERE id = NEW.target_id AND case_id <> NEW.case_id))
BEGIN
    SELECT RAISE(ABORT, 'edge target must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS edges_target_same_case_update
BEFORE UPDATE ON edges
WHEN
    (NEW.target_kind = 'content' AND EXISTS (SELECT 1 FROM content WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'source' AND EXISTS (SELECT 1 FROM sources WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'proposition' AND EXISTS (SELECT 1 FROM propositions WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'event' AND EXISTS (SELECT 1 FROM events WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'edge' AND EXISTS (SELECT 1 FROM edges WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'entity' AND EXISTS (SELECT 1 FROM entities WHERE id = NEW.target_id AND case_id <> NEW.case_id))
 OR (NEW.target_kind = 'advocacy' AND EXISTS (SELECT 1 FROM advocacy_items WHERE id = NEW.target_id AND case_id <> NEW.case_id))
BEGIN
    SELECT RAISE(ABORT, 'edge target must belong to the same case');
END;
