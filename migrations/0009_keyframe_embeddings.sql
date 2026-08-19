PRAGMA foreign_keys = ON;

-- Finder index for scene keyframes. Not evidence: export_case never reads
-- this table, and Store::index_keyframes writes no content and no edge.
-- One vector per (derived still, model); brute-force cosine in process.

CREATE TABLE IF NOT EXISTS keyframe_embeddings (
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    model TEXT NOT NULL CHECK(length(model) > 0),
    dim INTEGER NOT NULL CHECK(dim > 0),
    vector BLOB NOT NULL CHECK(length(vector) = dim * 4),
    extractor TEXT NOT NULL CHECK(length(extractor) > 0),
    extractor_version TEXT NOT NULL CHECK(length(extractor_version) > 0),
    PRIMARY KEY (source_id, model)
) STRICT;

CREATE INDEX IF NOT EXISTS idx_keyframe_embeddings_case_model
    ON keyframe_embeddings(case_id, model);

CREATE TRIGGER IF NOT EXISTS keyframe_embeddings_same_case_insert
BEFORE INSERT ON keyframe_embeddings
WHEN EXISTS (
    SELECT 1 FROM sources s
    WHERE s.id = NEW.source_id AND s.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'keyframe embedding source must belong to the same case');
END;

CREATE TRIGGER IF NOT EXISTS keyframe_embeddings_same_case_update
BEFORE UPDATE ON keyframe_embeddings
WHEN EXISTS (
    SELECT 1 FROM sources s
    WHERE s.id = NEW.source_id AND s.case_id <> NEW.case_id)
BEGIN
    SELECT RAISE(ABORT, 'keyframe embedding source must belong to the same case');
END;
