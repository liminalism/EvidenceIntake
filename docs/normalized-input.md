# Normalized extraction input

The collation kernel begins after modality processing. It does not embed an OCR,
speech-recognition, diarization, or video model.

Each adapter submits a `NormalizedBatch` containing untouched original sources,
exact segments, and extracted content:

```text
NormalizedBatch
  case_id
  sources[]
    id, production_id, logical_name
    media_type, source_kind, temporal_relation
    sha256, byte_length
    segments[]
      id, locator
      page and optional bounding box
      or original-media start_ms/end_ms
      content[]
        kind, text
        speaker and attributed person
        parent content for reporting chains
        raw_time
        content_created_at
        asserted_time
        proposed normalized interval and basis
        extractor name/version/confidence
        machine-generated flag and review state
```

Examples:

- OCR from a crash report is a `document_assertion` whose source is an
  `after_event` document. `content_created_at` is the report time;
  `asserted_time` may be the earlier collision time.
- A 911 ASR line is a `statement` pointing to millisecond offsets in original
  audio. The raw call/device time remains intact.
- A video scene model result is a bounded `observation`, not a fact. It points
  to the original frame interval and begins in `suggested` review state.
- An officer's report that a witness previously said something uses reporter,
  attributed-person, and parent-content fields rather than flattening the
  reporting chain.

The importer is atomic. It validates hashes, time-offset ranges, production
ownership, confidence range, identifier uniqueness through SQLite constraints,
and the rule that machine-generated records cannot self-verify.

