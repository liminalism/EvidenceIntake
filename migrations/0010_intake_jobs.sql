PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS intake_jobs (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id),
    production_id TEXT NOT NULL REFERENCES productions(id),
    source_id TEXT NOT NULL,
    modality TEXT NOT NULL CHECK(modality IN ('document','audio','video')),
    profile TEXT NOT NULL,
    original_path TEXT NOT NULL,
    original_sha256 TEXT NOT NULL CHECK(length(original_sha256) = 64),
    original_byte_length INTEGER NOT NULL CHECK(original_byte_length > 0),
    logical_name TEXT NOT NULL,
    request_json TEXT NOT NULL CHECK(json_valid(request_json)),
    artifact_dir TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','importing','completed','failed','interrupted')),
    attempt INTEGER NOT NULL DEFAULT 1 CHECK(attempt > 0),
    stage TEXT,
    progress_completed INTEGER CHECK(progress_completed IS NULL OR progress_completed >= 0),
    progress_total INTEGER CHECK(progress_total IS NULL OR progress_total >= 0),
    message TEXT,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    started_at TEXT,
    finished_at TEXT,
    UNIQUE(case_id, source_id)
) STRICT;

CREATE INDEX IF NOT EXISTS intake_jobs_case_state
ON intake_jobs(case_id, state, created_at, id);

CREATE TABLE IF NOT EXISTS intake_artifacts (
    job_id TEXT NOT NULL REFERENCES intake_jobs(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    path TEXT NOT NULL,
    sha256 TEXT CHECK(sha256 IS NULL OR length(sha256) = 64),
    PRIMARY KEY(job_id, kind, path)
) STRICT;

CREATE TABLE IF NOT EXISTS source_locations (
    source_id TEXT PRIMARY KEY REFERENCES sources(id) ON DELETE CASCADE,
    case_id TEXT NOT NULL REFERENCES cases(id),
    path TEXT NOT NULL,
    last_verified_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
) STRICT;

CREATE TRIGGER IF NOT EXISTS source_locations_same_case_insert
BEFORE INSERT ON source_locations
WHEN EXISTS (
    SELECT 1 FROM sources s
    WHERE s.id = NEW.source_id AND s.case_id <> NEW.case_id
)
BEGIN
    SELECT RAISE(ABORT, 'source location must belong to the source case');
END;

CREATE TRIGGER IF NOT EXISTS intake_jobs_production_same_case_insert
BEFORE INSERT ON intake_jobs
WHEN NOT EXISTS (
    SELECT 1 FROM productions p
    WHERE p.id = NEW.production_id AND p.case_id = NEW.case_id
)
BEGIN
    SELECT RAISE(ABORT, 'intake production must belong to the job case');
END;
