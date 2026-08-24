PRAGMA foreign_keys = ON;

-- Human semantic work is append-only. The immutable `content` rows remain the
-- adapter's exact extraction; every later reading lives here and replaces a
-- predecessor only by pointing at it.
CREATE TABLE IF NOT EXISTS source_profiles (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
    source_role TEXT NOT NULL CHECK(source_role IN (
      'police_report','supplemental_report','witness_statement_form','cad_log',
      'dispatch_audio','nine_one_one_call','body_camera','surveillance_video',
      'recorded_interview','jail_call','lab_report','medical_record','receipt',
      'evidence_inventory','photograph_set','derived_transcript','other'
    )),
    author_entity_id TEXT REFERENCES entities(id),
    created_at_claim TEXT,
    default_content_form TEXT CHECK(default_content_form IS NULL OR default_content_form IN (
      'recorded_utterance','authored_assertion','quoted_statement','reported_statement',
      'visual_observation','measured_result','evidence_reference',
      'official_characterization','machine_suggestion','no_speech_aligned',
      'recording_loss','boilerplate'
    )),
    default_temporal_stance TEXT CHECK(default_temporal_stance IS NULL OR default_temporal_stance IN (
      'contemporaneous_capture','contemporaneous_account','retrospective_recollection',
      'report_of_prior_statement','later_measurement','later_analysis','unknown'
    )),
    default_perception_basis TEXT CHECK(default_perception_basis IS NULL OR default_perception_basis IN (
      'saw','heard','measured','recorded','read_in_source','told_by_person',
      'inferred_or_characterized','unknown'
    )),
    clock_offset_ms INTEGER,
    clock_offset_basis TEXT,
    review_state TEXT NOT NULL CHECK(review_state IN (
      'unreviewed','suggested','reviewed','verified','rejected'
    )),
    created_by TEXT NOT NULL,
    supersedes_profile_id TEXT REFERENCES source_profiles(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK(clock_offset_ms IS NULL OR length(trim(clock_offset_basis)) > 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_source_profiles_supersedes
  ON source_profiles(supersedes_profile_id)
  WHERE supersedes_profile_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_source_profiles_source
  ON source_profiles(case_id, source_id, created_at);

CREATE TABLE IF NOT EXISTS content_groups (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    label TEXT,
    review_state TEXT NOT NULL CHECK(review_state IN (
      'unreviewed','suggested','reviewed','verified','rejected'
    )),
    created_by TEXT NOT NULL,
    supersedes_group_id TEXT REFERENCES content_groups(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_content_groups_supersedes
  ON content_groups(supersedes_group_id)
  WHERE supersedes_group_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS content_group_members (
    group_id TEXT NOT NULL REFERENCES content_groups(id) ON DELETE CASCADE,
    content_id TEXT NOT NULL REFERENCES content(id),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    PRIMARY KEY(group_id, content_id),
    UNIQUE(group_id, ordinal)
) STRICT;

CREATE TABLE IF NOT EXISTS content_interpretations (
    id TEXT PRIMARY KEY,
    case_id TEXT NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
    content_id TEXT REFERENCES content(id),
    content_group_id TEXT REFERENCES content_groups(id),
    char_start INTEGER CHECK(char_start >= 0),
    char_end INTEGER CHECK(char_end > char_start),
    content_form TEXT CHECK(content_form IS NULL OR content_form IN (
      'recorded_utterance','authored_assertion','quoted_statement','reported_statement',
      'visual_observation','measured_result','evidence_reference',
      'official_characterization','machine_suggestion','no_speech_aligned',
      'recording_loss','boilerplate'
    )),
    perception_basis TEXT CHECK(perception_basis IS NULL OR perception_basis IN (
      'saw','heard','measured','recorded','read_in_source','told_by_person',
      'inferred_or_characterized','unknown'
    )),
    temporal_stance TEXT CHECK(temporal_stance IS NULL OR temporal_stance IN (
      'contemporaneous_capture','contemporaneous_account','retrospective_recollection',
      'report_of_prior_statement','later_measurement','later_analysis','unknown'
    )),
    speaker_entity_id TEXT REFERENCES entities(id),
    attributed_entity_id TEXT REFERENCES entities(id),
    reporting_parent_interpretation_id TEXT REFERENCES content_interpretations(id),
    content_created_at TEXT,
    asserted_start TEXT,
    asserted_end TEXT,
    normalized_start TEXT,
    normalized_end TEXT,
    time_alignment_basis TEXT,
    location_text TEXT,
    location_entity_id TEXT REFERENCES entities(id),
    materiality TEXT NOT NULL DEFAULT 'unknown'
      CHECK(materiality IN ('material','boilerplate','administrative','unknown')),
    field_provenance_json TEXT NOT NULL DEFAULT '{}'
      CHECK(json_valid(field_provenance_json) AND json_type(field_provenance_json) = 'object'),
    basis TEXT,
    review_state TEXT NOT NULL CHECK(review_state IN (
      'unreviewed','suggested','reviewed','verified','rejected'
    )),
    created_by TEXT NOT NULL,
    supersedes_interpretation_id TEXT REFERENCES content_interpretations(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    CHECK((content_id IS NOT NULL) <> (content_group_id IS NOT NULL)),
    CHECK((char_start IS NULL) = (char_end IS NULL)),
    CHECK(content_group_id IS NULL OR char_start IS NULL),
    CHECK(normalized_start IS NULL OR length(trim(time_alignment_basis)) > 0)
) STRICT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_interpretations_supersedes
  ON content_interpretations(supersedes_interpretation_id)
  WHERE supersedes_interpretation_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_interpretations_content
  ON content_interpretations(case_id, content_id, char_start, char_end, created_at);
CREATE INDEX IF NOT EXISTS idx_interpretations_group
  ON content_interpretations(case_id, content_group_id, created_at);

CREATE VIEW IF NOT EXISTS current_source_profiles AS
SELECT profile.*
FROM source_profiles profile
WHERE NOT EXISTS (
  SELECT 1 FROM source_profiles successor
  WHERE successor.supersedes_profile_id = profile.id
);

CREATE VIEW IF NOT EXISTS current_content_groups AS
SELECT content_group.*
FROM content_groups content_group
WHERE NOT EXISTS (
  SELECT 1 FROM content_groups successor
  WHERE successor.supersedes_group_id = content_group.id
);

CREATE VIEW IF NOT EXISTS current_content_interpretations AS
SELECT interpretation.*
FROM content_interpretations interpretation
WHERE NOT EXISTS (
  SELECT 1 FROM content_interpretations successor
  WHERE successor.supersedes_interpretation_id = interpretation.id
);

CREATE TABLE IF NOT EXISTS brief_paragraphs (
    id TEXT PRIMARY KEY,
    brief_id TEXT NOT NULL REFERENCES decision_briefs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    kind TEXT NOT NULL CHECK(kind IN ('factual','analytical')),
    body TEXT NOT NULL,
    references_json TEXT NOT NULL DEFAULT '[]'
      CHECK(json_valid(references_json) AND json_type(references_json) = 'array'),
    UNIQUE(brief_id, ordinal)
) STRICT;

CREATE TRIGGER IF NOT EXISTS source_profiles_require_author
BEFORE INSERT ON source_profiles
WHEN length(trim(NEW.created_by)) = 0
BEGIN
    SELECT RAISE(ABORT, 'source profile must name its author');
END;

CREATE TRIGGER IF NOT EXISTS interpretations_require_author
BEFORE INSERT ON content_interpretations
WHEN length(trim(NEW.created_by)) = 0
BEGIN
    SELECT RAISE(ABORT, 'interpretation must name its author');
END;

CREATE TRIGGER IF NOT EXISTS content_groups_require_author
BEFORE INSERT ON content_groups
WHEN length(trim(NEW.created_by)) = 0
BEGIN
    SELECT RAISE(ABORT, 'content group must name its author');
END;

CREATE TRIGGER IF NOT EXISTS suggested_profiles_require_suggest_author
BEFORE INSERT ON source_profiles
WHEN (NEW.review_state = 'suggested') <> (NEW.created_by LIKE 'suggest:%')
BEGIN
    SELECT RAISE(ABORT, 'suggested profile must use a suggest: author and people may not use one');
END;

CREATE TRIGGER IF NOT EXISTS suggested_interpretations_require_suggest_author
BEFORE INSERT ON content_interpretations
WHEN (NEW.review_state = 'suggested') <> (NEW.created_by LIKE 'suggest:%')
BEGIN
    SELECT RAISE(ABORT, 'suggested interpretation must use a suggest: author and people may not use one');
END;

CREATE TRIGGER IF NOT EXISTS factual_brief_paragraphs_require_references
BEFORE INSERT ON brief_paragraphs
WHEN NEW.kind = 'factual' AND json_array_length(NEW.references_json) = 0
BEGIN
    SELECT RAISE(ABORT, 'a factual brief paragraph requires at least one source reference');
END;
