PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS cases (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    reference TEXT,
    jurisdiction TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TABLE IF NOT EXISTS productions (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    received_at TEXT,
    producing_party TEXT,
    notes TEXT,
    UNIQUE(case_id, label)
) STRICT;

CREATE TABLE IF NOT EXISTS sources (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    production_id TEXT REFERENCES productions(id),
    logical_name TEXT NOT NULL,
    media_type TEXT NOT NULL,
    source_kind TEXT NOT NULL DEFAULT 'other'
      CHECK(source_kind IN ('document','audio','video','image_set','structured_data','other')),
    temporal_relation TEXT NOT NULL DEFAULT 'unknown'
      CHECK(temporal_relation IN ('contemporaneous','after_event','mixed','unknown')),
    sha256 TEXT NOT NULL CHECK(length(sha256) = 64),
    byte_length INTEGER NOT NULL CHECK(byte_length >= 0),
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
      CHECK(review_state IN ('unreviewed','suggested','reviewed','verified','rejected')),
    integrity_status TEXT NOT NULL DEFAULT 'available'
      CHECK(integrity_status IN ('available','unreadable','corrupt','password_protected','truncated','missing')),
    supersedes_source_id TEXT REFERENCES sources(id),
    UNIQUE(case_id, sha256)
) STRICT;

CREATE TABLE IF NOT EXISTS source_segments (
    id TEXT PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    locator TEXT NOT NULL,
    page INTEGER CHECK(page > 0),
    start_ms INTEGER CHECK(start_ms >= 0),
    end_ms INTEGER CHECK(end_ms >= start_ms),
    bbox_json TEXT CHECK(bbox_json IS NULL OR json_valid(bbox_json)),
    content_hash TEXT,
    UNIQUE(source_id, locator)
) STRICT;

CREATE TABLE IF NOT EXISTS entities (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('person','organization','object','location')),
    display_name TEXT NOT NULL,
    is_client INTEGER NOT NULL DEFAULT 0 CHECK(is_client IN (0,1)),
    notes TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS entity_mentions (
    id TEXT PRIMARY KEY,
    segment_id TEXT NOT NULL REFERENCES source_segments(id) ON DELETE CASCADE,
    mention_text TEXT NOT NULL,
    resolved_entity_id TEXT REFERENCES entities(id),
    resolver_confidence REAL CHECK(resolver_confidence BETWEEN 0.0 AND 1.0),
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
) STRICT;

CREATE TABLE IF NOT EXISTS content (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    segment_id TEXT NOT NULL REFERENCES source_segments(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN ('statement','observation','document_assertion','evidence_reference','recording_gap')),
    text TEXT NOT NULL,
    speaker_entity_id TEXT REFERENCES entities(id),
    attributed_to_entity_id TEXT REFERENCES entities(id),
    parent_content_id TEXT REFERENCES content(id),
    raw_time TEXT,
    content_created_at TEXT,
    asserted_time TEXT,
    normalized_start TEXT,
    normalized_end TEXT,
    time_basis TEXT,
    location_text TEXT,
    extractor TEXT,
    extractor_version TEXT,
    machine_generated INTEGER NOT NULL DEFAULT 0 CHECK(machine_generated IN (0,1)),
    extractor_confidence REAL CHECK(extractor_confidence BETWEEN 0.0 AND 1.0),
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
      CHECK(review_state IN ('unreviewed','suggested','reviewed','verified','rejected'))
) STRICT;

CREATE TABLE IF NOT EXISTS propositions (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    text TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'contested'
      CHECK(status IN ('contested','undisputed','withdrawn')),
    review_state TEXT NOT NULL DEFAULT 'unreviewed',
    created_by TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS events (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    lane TEXT NOT NULL CHECK(lane IN ('recorded','witness_account','police_narrative','client_account','attorney_hypothesis')),
    raw_time TEXT,
    normalized_start TEXT,
    normalized_end TEXT,
    time_basis TEXT,
    location_text TEXT,
    proposition_id TEXT REFERENCES propositions(id),
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
) STRICT;

CREATE TABLE IF NOT EXISTS edges (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    source_id TEXT NOT NULL,
    relation TEXT NOT NULL CHECK(relation IN (
      'supports','contradicts','corroborates','impeaches','qualifies','explains',
      'derived_from','refers_to','temporally_overlaps','possibly_same_person',
      'expected_but_missing','requires_follow_up','relevant_to'
    )),
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    rationale TEXT,
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
      CHECK(review_state IN ('unreviewed','suggested','reviewed','verified','rejected')),
    created_by TEXT NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS charges (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    citation TEXT,
    posture TEXT NOT NULL DEFAULT 'charged'
      CHECK(posture IN ('charged','lesser_candidate','alternative','dismissed')),
    grade TEXT,
    disposition TEXT
) STRICT;

CREATE TABLE IF NOT EXISTS elements (
    id TEXT PRIMARY KEY,
    charge_id TEXT NOT NULL REFERENCES charges(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal > 0),
    text TEXT NOT NULL,
    UNIQUE(charge_id, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS element_links (
    id TEXT PRIMARY KEY,
    element_id TEXT NOT NULL REFERENCES elements(id) ON DELETE CASCADE,
    proposition_id TEXT NOT NULL REFERENCES propositions(id) ON DELETE CASCADE,
    assessment TEXT NOT NULL CHECK(assessment IN ('supports','opposes','uncertain','excluded')),
    notes TEXT,
    UNIQUE(element_id, proposition_id, assessment)
) STRICT;

CREATE TABLE IF NOT EXISTS advocacy_items (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK(kind IN (
      'defense_theory','prosecution_theory','legal_issue','motion_issue',
      'cross_examination_point','investigation_task','negotiation_consideration',
      'mitigation_theme','attorney_conclusion'
    )),
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    privileged INTEGER NOT NULL DEFAULT 1 CHECK(privileged IN (0,1)),
    version INTEGER NOT NULL DEFAULT 1 CHECK(version > 0),
    author TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TABLE IF NOT EXISTS annotations (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    body TEXT NOT NULL,
    version INTEGER NOT NULL CHECK(version > 0),
    supersedes_annotation_id TEXT REFERENCES annotations(id),
    author TEXT NOT NULL,
    privileged INTEGER NOT NULL DEFAULT 1 CHECK(privileged IN (0,1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE TABLE IF NOT EXISTS decision_briefs (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    posture TEXT NOT NULL CHECK(posture IN ('release','motions','negotiation','trial','sentencing','appeal')),
    summary TEXT NOT NULL,
    strengths TEXT NOT NULL,
    risks TEXT NOT NULL,
    unresolved_questions TEXT NOT NULL,
    client_topics TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    author TEXT NOT NULL,
    privileged INTEGER NOT NULL DEFAULT 1 CHECK(privileged IN (0,1))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_sources_case ON sources(case_id);
CREATE INDEX IF NOT EXISTS idx_content_case_kind ON content(case_id, kind);
CREATE INDEX IF NOT EXISTS idx_content_speaker ON content(speaker_entity_id, attributed_to_entity_id);
CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(case_id, target_kind, target_id, relation);
CREATE INDEX IF NOT EXISTS idx_events_case_time ON events(case_id, normalized_start);
CREATE INDEX IF NOT EXISTS idx_advocacy_case_kind ON advocacy_items(case_id, kind);
