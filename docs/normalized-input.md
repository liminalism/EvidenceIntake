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
  edges[]
    id
    from_kind/from_id  (source or content)
    relation           (temporally_overlaps, derived_from, refers_to)
    to_kind/to_id      (source or content)
    rationale          (required; the measurement lives here)
    extractor name/version, machine-generated flag and review state
```

`edges[]` is optional and absent from batch documents written before it existed.
An adapter may say how records sit relative to each other; it may never say what
they establish, so `supports`, `contradicts`, and the rest of the evaluative
vocabulary are refused. Endpoints are resolved *after* this batch's own sources
and content are inserted, so a delivery may relate the stills it brings, and a
batch carrying no sources at all is how a later pass relates two originals
imported on different days. A proposal is written `suggested` and attributed
`suggest:<extractor>@<version>` -- the same prefix the analyzers use, which is
how the review queue recognises a machine-proposed relationship. Confidence is
not stored on an edge; the rationale is what a reviewer weighs.

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
- Camera B `temporally_overlaps` camera A: the audio cross-correlation offset is
  written into the rationale, and the pair waits in the review queue as a
  machine proposal rather than as an established fact.

The importer is atomic. It validates hashes, time-offset ranges, production
ownership, confidence range, identifier uniqueness through SQLite constraints,
the rule that machine-generated records cannot self-verify, and that a speaker,
attributed person, or parent content already exists *inside this case*. Cases
do not share records.

A proposed edge is refused when its relation is outside the three structural
kinds, when either endpoint is neither a source nor content, when an endpoint is
missing from the case or belongs to another one, when the rationale is blank,
when the provenance is human-attributed or in any state but `suggested`, when
the edge identifier is already in use, or when the case already relates that
pair that way in either direction. Every refusal rolls the whole batch back,
including the sources it carried.

The case and its production must already exist. Open them with `evidence new-case`
and `evidence new-production` (or the workspace **Untitled Case** command)
before submitting a batch.

