# Case assembly: closing the collation gap

**Status:** plan. Governing AKR milestone: `@evidence-intake-project.assembly.case-assembly`.
**Origin:** the external static review registered as AKR source
`collation-gap-review-2026-08-23` (`sources/external/collation-gap-review-2026-08-23--ab4591b1.md`).
Where this document says "the review recommends", it is citing that source; where it
says "the plan is", an AKR record holds it.

## 1. What the review found, in one paragraph

The kernel's provenance and epistemic rules are sound, and the fixtures show the read
models working — but only because the fixtures hand-build the semantic graph. The live
adapters produce flattened excerpts: every OCR region is a generic `statement` with no
author, attribution, reporting parent, creation time, asserted time or location
(`adapters/document/src/map.rs:108–157`); ASR segments carry no speaker, and diarization
labels sit beside them as unlinked observations (`adapters/audio/src/map.rs:228–251`);
no-speech intervals are mislabelled `recording_gap` (`adapters/audio/src/map.rs:321–349`).
The feature called "collation" is a time/place navigation index (`src/store.rs:1307`,
`src/views.rs:140–245`), and "Run Collation" runs analyzers that can only find tensions
in a graph someone has already assembled. The missing middle is:

```text
raw extraction
   → reviewed semantic interpretation
   → source dependency and account assembly
   → proposition packets
   → layered, source-linked case digest
```

The product must not merge media into a story. It assembles distinct accounts,
observations, measurements and reporting relationships around contested propositions.

## 2. The constraint this plan adds: manual entry is the bottleneck

The review lists fields the adapters must populate. Most of them **cannot be populated
algorithmically** without a language model, and this project leans away from a local
LLM. Honest inventory of the fields the review names:

| Field | Detectable without a model? | How |
| --- | --- | --- |
| `source_role` (police report, CAD export, 911 call, bodycam…) | **No** — but it is one choice per *source*, so it is cheap | Entered at intake, one combo box, remembered per production |
| `content_created_at` (report written on…) | **Partly** | Document header date patterns; PDF metadata; media container timestamps; otherwise inherited from the source |
| `speaker_or_author` | **Partly** | Document: inherited from the source's author; audio: diarization label is a *candidate*, not a person |
| `attributed_person` / reporting parent | **No** | Cue phrases ("stated that", "advised", "told me") can flag a *candidate* span; the person and parent need a human |
| `content_form` (utterance, authored assertion, quoted, reported, observation, measurement, reference…) | **Partly** | Modality gives a default (ASR → `recorded_utterance`, OCR in a report → `authored_assertion`); quotation marks, "stated", "§", numeric-with-unit patterns give candidates |
| `perception_basis` (saw, heard, told, read, measured…) | **No** | Cue verbs give candidates only |
| `temporal_stance` | **Partly** | Source default; must be overridable per passage |
| `asserted_time` | **Partly** | Clock patterns in the text are candidates; which event they belong to needs a human |
| normalized case interval + alignment basis | **No** | Requires a human to choose the anchor and write the basis |
| `location` | **Partly** | Address-like patterns are candidates |
| reporting chain, quote/summary/derivation edges | **No** | Cue phrases and cross-document string matches propose; a human accepts |
| same-occurrence candidates | **No** | Shared anchors propose; a human accepts |
| proposition links | **No** | Entirely human |

So the adapters will never fill the schema. **The product's throughput is therefore
bounded by how many human decisions per passage the workflow costs**, and the design
target for the whole plan is:

> A trained paralegal classifies a 20-page police report — author, form, stance, creation
> time, the reported-statement chains and the asserted times — in under fifteen minutes,
> without touching the mouse, and every keystroke is recorded as an append-only,
> reviewable interpretation.

Section 5 is the design that meets that target. Everything else in the plan is in service
of making what the human enters worth entering.

### Why not a local LLM for the classification

It would be the obvious way to pre-fill these fields, and the review itself allows a model
to *propose* classifications later. The reasons to defer it, recorded here so the choice is
revisited deliberately rather than drifted into:

- every proposal still needs a human accept, so the model saves reading time, not
  decision time — and the keystroke design below makes the decision itself nearly free;
- a wrong speaker, attribution or stance proposal is *worse* than an empty field because
  it primes the reviewer (the review's "current features that can lead the reviewer too
  far" applies to pre-fills as much as to views);
- deterministic cue rules are inspectable, testable in `tests/`, versioned in
  `created_by = "suggest:<rule>@<version>"`, and never hallucinate an entity that does not
  exist in the case.

If a later measurement shows the cue rules leave the paralegal reading too much, the
`suggest:` contract is already the seam where a model plugs in — as a proposer, never a
writer of reviewed state.

## 3. Architecture of the missing middle

### 3.1 Layering rule

`content` stays exactly what it is: the immutable extraction, one row per adapter output,
with its exact locator. Nothing in this plan edits a `content` row after import. All
semantic work lands in **append-only tables that point at content**, versioned by
superseding, with their own review state and a named author. This is the same shape as
`annotations` and `advocacy_items` (migration 0005), applied to the evidentiary layer.

### 3.2 New tables (migration `0011_interpretation.sql`, bumps `SCHEMA_VERSION`)

**`content_interpretations`** — one reviewed reading of one content row.

```text
id, case_id, content_id            -- which passage
char_start, char_end               -- NULL = whole passage; a sub-span otherwise (§5.6)
content_form                       -- controlled vocabulary (§3.3)
perception_basis                   -- controlled vocabulary
temporal_stance                    -- controlled vocabulary
speaker_entity_id                  -- who uttered / authored this passage
attributed_entity_id               -- whose words it reports, if any
reporting_parent_interpretation_id -- the interpretation that reports this one
content_created_at                 -- when the passage itself was made
asserted_start, asserted_end       -- the time the passage claims for the occurrence
normalized_start, normalized_end   -- the reviewer's placement on the case clock
time_alignment_basis               -- written; required when normalized_* is set
location_text
location_entity_id
materiality                        -- material | boilerplate | administrative | unknown
field_provenance_json              -- per field: entered | inherited:<source> | accepted:<rule@ver>
basis                              -- free text, optional
review_state                       -- unreviewed | suggested | reviewed | verified | rejected
created_by                         -- named person, or suggest:<rule>@<version>
supersedes_interpretation_id       -- UNIQUE where not null, as in 0005
created_at
```

Triggers, as the existing tables do it: `created_by` non-blank; a `suggest:` author may
only insert `suggested`; `normalized_*` without `time_alignment_basis` is refused; a
`reporting_parent` must be in the same case (enforced in `Store`, as `import_normalized`
does for `parent_content_id`). Exactly one current interpretation per
`(content_id, char_start, char_end)` is a view-level filter, not a constraint — a rejected
reading stays in the trail.

**`source_profiles`** — one reviewed reading of one *source*, the thing that makes
inheritance possible.

```text
id, case_id, source_id
source_role        -- police_report | supplemental_report | witness_statement_form |
                      cad_log | dispatch_audio | nine_one_one_call | body_camera |
                      surveillance_video | recorded_interview | jail_call | lab_report |
                      medical_record | receipt | evidence_inventory | photograph_set |
                      derived_transcript | other
author_entity_id   -- default speaker for every passage in a document
created_at_claim   -- the date the source says it was made
default_content_form, default_temporal_stance, default_perception_basis
clock_offset_ms, clock_offset_basis   -- for recordings: device clock vs case clock
review_state, created_by, supersedes_profile_id, created_at
```

**`content_groups` / `content_group_members`** — a reviewer's semantic unit spanning
several extraction rows (a paragraph OCR split into four regions; an ASR utterance cut at
a pause). A group can be the subject of an interpretation exactly like a content row
(`NodeKind::ContentGroup`). Raw rows are untouched; the group is the reviewer's reading of
where the unit boundaries lie.

**`brief_paragraphs`** — the structural source-linking the review asks for on decision
briefs (`migrations/0001_collation.sql:195–207`): one row per paragraph with
`kind = factual | analytical`, an ordered list of `(node_kind, node_id)` references, and a
trigger/`Store` rule that a `factual` paragraph with no references cannot be written.

### 3.3 Controlled vocabularies (new enums in `src/model.rs`, `as_str` + `CHECK`)

`ContentForm`: `recorded_utterance`, `authored_assertion`, `quoted_statement`,
`reported_statement`, `visual_observation`, `measured_result`, `evidence_reference`,
`official_characterization`, `machine_suggestion`, `no_speech_aligned`, `recording_loss`,
`boilerplate`.

`PerceptionBasis`: `saw`, `heard`, `measured`, `recorded`, `read_in_source`,
`told_by_person`, `inferred_or_characterized`, `unknown`.

`TemporalStance`: `contemporaneous_capture`, `contemporaneous_account`,
`retrospective_recollection`, `report_of_prior_statement`, `later_measurement`,
`later_analysis`, `unknown`.

`SourceRole` as listed above. `SourceKind` (modality) stays; the review is explicit that
medium is not evidentiary role and the schema must keep the axes independent.

`content.kind` is **not** widened. The `CHECK` on a `STRICT` table cannot be altered
without rebuilding `content`, which carries the FTS external-content triggers and every
foreign key in the graph. `kind` remains the coarse intake category; `content_form` is
the fine one. The one adapter fix it forces — no-speech intervals — emits
`ContentKind::Observation` with extractor `asr-no-speech`, and the interpretation layer
or a later analyzer classifies it `no_speech_aligned`. `recording_gap` is reserved for
demonstrated stream loss, truncation or missing media.

### 3.4 Edge vocabulary (migration `0012_edge_vocabulary.sql`)

`edges.relation` has a `CHECK`, so widening it means the SQLite twelve-step rebuild of
`edges` inside the migration (create new, copy, drop, rename, recreate indexes
`idx_edges_target` and `idx_edges_unique_claim`). `review_events` references edges by
`(kind, id)` string, not foreign key, so the trail survives. This is done once, with the
whole vocabulary, and the `schema` module test asserts the unique-claim index survived.

Descriptive provenance relations (who depends on whom; **structural**, proposable by
rules and adapters, reviewed by a person):

`quotes`, `reports`, `summarizes`, `transcribes`, `depicts`, `records_utterance`,
`measures`, `based_on`, `account_of`, `created_after`, `recorded_during`,
`candidate_same_occurrence`, `speaker_candidate` (diarization label → entity).

Evaluative relations (what the defender says it establishes; **human only**):

`supports`, `contradicts`, `qualifies`, `explains`, `impeaches`, `consistent_with`,
`independently_corroborates`.

`corroborates` is retired: the migration rewrites existing rows to `consistent_with` and
records that in the migration comment; `independently_corroborates` requires a written
`rationale` *and* is refused by `link_evidence` when the lineage walk (§3.6) finds the two
endpoints share a reporting lineage — the refusal names the shared root. This is the one
place the kernel computes something from provenance edges, and it computes a
*dependency*, not a weight.

`ingest.rs` widens the adapter-proposable set from three to the descriptive list; the
evaluative list stays refused at import, as `docs/normalized-input.md` promises.

### 3.5 Occurrences

No new node table. An occurrence candidate is a reviewed `candidate_same_occurrence` edge
between two content rows, groups or events, and an account is an `account_of` edge from
content to an `events` row in the appropriate lane. `events` already has lanes and
`proposition_id`; a reviewer creating an occurrence creates an event in the `recorded` or
account lane and links accounts to it. Nothing promotes a time/place group to an event
without that reviewed edge (the review's rule 2 under "features that lead the reviewer
too far").

### 3.6 Lineage

`Store::lineage(case, node)` walks reviewed (`reviewed`/`verified`) descriptive edges
(`quotes`, `reports`, `summarizes`, `transcribes`, `based_on`, `derived_from`,
`records_utterance`) upward to roots. A **lineage** is the set of nodes sharing a root.
Every count in every packet, standing view and digest is then reported three ways, as
the review asks:

```text
3 original files · 2 reporting lineages · 1 without a reviewed dependency
```

The third phrase is deliberately not "independent": the kernel reports that no reviewed
dependency edge exists, and only a person's `independently_corroborates` says more.
`nothing_in_the_standing_view_scores_the_case` extends to the packet view.

## 4. Priority 0 — stop the current views from misleading (no schema change)

These ship first and alone; they lower present risk and need none of the above.

1. **Rename.** `Collation &Groups` → `Time && Place Index` (keeping `&G` on a letter of
   the caption); `Run Co&llation` → `Find Tensions and Gaps` (keeping `&L`, since `p` is
   taken by the work-file export); `possibly_related` → `shared_anchor_unconfirmed` in
   `CollationIndex` with the group header "Shared reviewed date/location anchor —
   relationship not established". `docs/collation.md` retitled. The Alt-key test
   (`every_command_has_its_own_alt_key`) decides the mnemonics.
2. **Label every time.** `src/gui/mod.rs:671–677` picks the first of
   normalized/asserted/created/raw and prints it bare. Replace with a `TimeLabels`
   renderer that prints every present value under its own label (`Normalized case time`,
   `Alleged event time`, `Report created`, `Recording time`) and `Alignment: not set`
   when normalized is absent. Same renderer is reused by every view in P3/P4.
3. **Badges.** Every rendered content line carries a fixed-width prefix from
   `(machine_generated, review_state, kind, extractor)`: `MACHINE SUGGESTION`,
   `RAW TRANSCRIPT`, `HUMAN-VERIFIED TRANSCRIPT`, `REVIEWED OBSERVATION`, `DOCUMENT
   ASSERTION`, `REPORT OF STATEMENT`. One function, `badge(&CollationEntry)`, tested.
4. **No-speech is not a gap.** Audio adapter emits `Observation` + `asr-no-speech`; text
   "No speech was aligned between … and …; this does not establish recording loss."
   `RecordingGap` is reserved for integrity failures. Test: the audio mapping fixture
   produces zero `recording_gap` rows.
5. **Diarization as a candidate.** Audio adapter emits, per utterance, a
   `NormalizedEdge` `speaker_candidate` (after P1's vocabulary lands; until then,
   `refers_to` with the label in the rationale) from the utterance to a per-source
   `Speaker N` placeholder entity created by the adapter with `notes = "diarization label,
   unresolved"`. Rule 7 (a mention is not a person) holds: placeholder entities are a kind
   of mention.
6. **Counts.** `distinct_sources` (`src/store.rs:4269`) is renamed `distinct_originals`
   and its label in every view becomes "original files" — never "sources" or
   "corroborating".

Acceptance: GUI snapshot tests for the renamed views; `tests/collation.rs` asserts the
new group key and the labelled-time output; audio mapping test for items 4–5.

## 5. Priority 1 — semantic enrichment, designed for throughput

This is the centre of the plan. Three principles, then the concrete workflow.

**Principle A — decide once, inherit everywhere.** A source profile is one screen per
file. Its author, role, creation date, default form and stance are inherited by every
passage in it. A passage's *current* reading is `explicit interpretation ?? inherited
profile default`, and the inheritance is visible (`field_provenance_json` says
`inherited:source_profile`). The human only touches exceptions — and in a police report,
the exceptions (the reported statements, the measurements, the asserted times) are
exactly the passages that matter.

**Principle B — sweep one field across many passages, not all fields on one passage.**
Filling a form per passage is the slow shape. The fast shape is a column sweep: pick a
field, walk the passages with `j`/`k`, press one key per passage. Attention stays on one
question ("who is speaking here?") and the rhythm is one keystroke per row.

**Principle C — a candidate costs one key to accept and one to refuse.** Deterministic cue
rules (§5.4) pre-fill `suggested` interpretations. In a sweep, `Enter` accepts the
candidate under the cursor, `x` rejects it (with the rule name as the recorded basis),
anything else overrides. Candidates are never silently adopted; an unreviewed candidate
renders with the `MACHINE SUGGESTION` badge and never reaches a packet.

### 5.1 The Enrichment workspace (WinSafe)

The window has claimed 25 of the 26 Alt mnemonics (`every_command_has_its_own_alt_key`
is what keeps them distinct), so the new views cannot get letters. Workspace navigation
moves to `Ctrl+1`…`Ctrl+9`, `Ctrl+0` for the existing ten views and `Ctrl+Shift+1`… for
the new ones, with the Alt captions kept as they are; the test is extended to assert
every view has exactly one accelerator. The P0 renames free `l` and `g` but do not spend
them.

A single-window, grid-first view:

```text
┌ Source: Ruiz supplemental.pdf  Role: [supplemental_report]  Author: [Ofc. Ruiz]  Created: [2024-03-02] ┐
│ Sweep field: [content_form ▾]   (F2 form · F3 speaker · F4 attributed · F5 stance · F6 time · F7 loc) │
├────┬──────┬──────────────────────────────────────────────┬──────────────┬──────────┬──────────────────┤
│ #  │ loc  │ passage                                      │ content_form │ prov     │ speaker          │
│ 12 │ p3 r2│ I observed the vehicle travelling north…     │ authored_ass.│ inherited│ Ofc. Ruiz (inh.) │
│>13 │ p3 r3│ Lee stated that the driver exited and…       │ reported_st.?│ cue:stated│ Ofc. Ruiz (inh.) │
│ 14 │ p3 r4│ BAC 0.060 at 22:42                           │ measured_re.?│ cue:unit │ Ofc. Ruiz (inh.) │
├────┴──────┴──────────────────────────────────────────────┴──────────────┴──────────┴──────────────────┤
│ Original: [page 3 rendered, region 3 highlighted]                                                      │
│ u utterance · a assertion · q quoted · r reported · o observation · m measured · e reference ·        │
│ c characterization · b boilerplate · Enter accept · x reject · . repeat last · / find · ? keys        │
└────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- The original is always visible beside the grid (page render with the region box;
  waveform/frame strip with the interval) so accepting a candidate is a verification, not
  a guess. Rule 17 holds: `verified` still requires the locator, and the grid supplies it.
- The grid is a `ListView` with in-place editing; all commands are single keys while the
  grid has focus, prefixed with the existing Alt mnemonics only for window-level actions.

### 5.2 Keystroke grammar

| Key | Meaning |
| --- | --- |
| `j` / `k`, `↓` / `↑` | next / previous passage |
| `J` / `K` | next / previous passage *with a candidate or missing value* in the sweep field |
| `F2`…`F7` | switch sweep field |
| one letter (per field) | set the value for this passage and advance |
| `Enter` | accept the candidate and advance |
| `x` | reject the candidate (basis = rule name) and advance |
| `.` | repeat the last entered value and advance |
| `Shift+letter` | set the value and **extend to the end of the current group/page** (range apply, §5.5) |
| `v` … `letter` | visual range: mark, move, apply one value to the range |
| `s` | split: open the sub-span editor on this passage (§5.6) |
| `g` | group: merge this passage with the next into one unit (§5.6) |
| `p` | set reporting parent = the nearest preceding passage whose speaker is the same author (the usual "Ruiz reports that Lee said") — or type an id |
| `@` | entity autocomplete for speaker/attributed/location; `Ctrl+N` inside it creates a new entity with the typed name, `Ctrl+M` marks it `possibly_same_person` with the highlighted match instead of reusing it |
| `t` | time entry: accepts `22:42`, `2024-03-02 22:42`, `+57m` (relative to the last entered anchor), `~` prefix for approximate; asks for the basis once per sweep and re-uses it until changed |
| `n` | materiality = boilerplate/administrative; the passage leaves every later sweep's `J`/`K` path |
| `u` (in command mode) | undo = supersede the last interpretation with its predecessor's values |
| `?` | key sheet for the current field |

Every keystroke that changes a value writes one `content_interpretations` row (or
supersedes the current one) with `created_by = actor` and the field's provenance marked
`entered`. The actor is set once per session and shown in the title bar; the existing
rule that every decision names a person is unchanged.

### 5.3 Throughput budget (measured, not asserted)

A keystroke harness in `tests/enrichment.rs` drives the `Workspace` API with a scripted key
sequence over the hit-and-run fixture's raw-file counterpart (§9) and asserts:

- the 20-page report reaches a state where every `material` passage has a reviewed form,
  speaker and stance in **≤ 1.6 keystrokes per passage** (candidates accepted by `Enter`
  count as one; inherited values count as zero);
- every reported statement in the fixture has a reporting parent and attributed person
  in ≤ 4 keystrokes each;
- no keystroke in the script touches a mouse-only path (the harness has none).

The number is a ratchet: a change that raises it fails the test and needs a recorded
reason. Wall-clock is measured by hand on the real GUI and recorded as an observation.

### 5.4 Deterministic candidate rules (`src/enrich.rs`, `SuggestionKind` extended)

Each rule is one versioned analyzer in the existing `suggest:` contract: deterministic,
no score, no tolerance, writes `suggested` interpretations or descriptive edges, never
touches a human-written row, never re-proposes a rejected one. Initial set:

| Rule | Proposes | Cue |
| --- | --- | --- |
| `inherit-profile` | form/stance/basis/speaker/created_at from the source profile | profile exists; only where no explicit interpretation |
| `reported-statement` | `reported_statement`, attributed = the entity mention nearest before the verb, parent = preceding same-author passage | `stated that`, `said`, `advised`, `told (me\|officers)`, `reported that`, `related` |
| `quoted-statement` | `quoted_statement` on the quoted span (sub-span) | balanced `"…"` ≥ 6 words |
| `measured-result` | `measured_result` | number + unit/ratio (`0.060`, `mph`, `ng/mL`, `°F`, `ft`) |
| `official-characterization` | `official_characterization` | `§`, statute citation shape, `in violation of`, `probable cause` |
| `evidence-reference` | `evidence_reference` + `refers_to` edge when a case source matches by name | `see attached`, `body worn camera`, `BWC`, `exhibit`, `recording`, `CAD` |
| `asserted-clock` | `asserted_start` candidate | `\b\d{1,2}:\d{2}\b` with optional `hrs`, `at approximately` |
| `header-date` | `content_created_at` on the profile | page-1 date near `Date of Report`, `Prepared`, PDF `CreationDate` |
| `diarization-speaker` | `speaker_candidate` edge | diarization label observation over the same interval |
| `cross-document-echo` | `summarizes` / `quotes` candidate edge between two sources | ≥ 12-token exact run shared by a later-created document and an earlier transcript or document |
| `shared-anchor` | `candidate_same_occurrence` | reviewed normalized date + location equal across ≥ 2 originals (replaces today's grouping as a *proposal*) |

Every rule has a test with a true positive, a true negative and the adversarial case from
§9 it is most likely to get wrong (e.g. `reported-statement` on "I stated to Lee that…",
where the officer is the speaker, not the reporter).

### 5.5 Range apply

`Shift+letter` and visual range write one interpretation per passage in the range, each
with `basis = "range apply with <n> others"` so the trail shows it was a sweep decision
and a later reader can re-examine the whole range. No row is ever a reference to another
row's value: the inheritance that matters lives in the profile, not in a range.

### 5.6 Split and group

OCR regions and ASR segments are the wrong granularity for meaning. `s` opens the passage
text with the cursor; marking a span writes an interpretation with `char_start/char_end`
(the quoted-statement rule does the same). `g` creates a `content_group` of this passage
and the next; interpretations then target the group. Both leave the `content` row
untouched and keep its locator as the citation for `verified`.

### 5.7 Just-in-time enrichment

Sweeps are for the high-value sources. For the long tail, the packet workspace (§7) asks
for the missing field at the moment a passage is attached to a proposition — the same
single-key grammar in a three-line prompt — so nothing is classified that nobody needs.

### 5.8 A bulk contract for people (`InterpretationBatch`)

The human counterpart to `NormalizedBatch`: a JSON (and CSV) document of source profiles
and interpretations keyed by content id, imported atomically through
`Store::import_interpretations`. It is what the keystroke harness drives, what a paralegal
can produce in a spreadsheet from an exported grid, and what makes the whole layer testable
from `tests/` without the GUI. Every row names its author; `suggest:` authors are refused
(a person's batch is not a machine's).

### 5.9 Open AKR question resolved: later passes on an identical source

`@evidence-intake-project.ingest.append-to-identical-source-question` is answered by the
model the review states: one immutable original, many append-only passes.
`import_normalized` accepts a batch whose source matches an existing row on `id`,
`production_id`, `sha256` and `byte_length` exactly, provided every segment and content
id is new; it never touches the source row; a differing hash or reused id is still
refused. This is needed so re-extraction, overnight captioning, and a second OCR pass
have somewhere to land — and interpretations, being a separate table, need it not at all.

## 6. Priority 2 — source dependency and occurrence assembly

- Edge vocabulary and `link_evidence` rules from §3.4; `ingest.rs` widened; the
  `cross-document-echo`, `diarization-speaker` and `shared-anchor` rules from §5.4.
- `Store::lineage` from §3.6 and the three-way count in `CaseStanding`.
- An **Assembly** view in the GUI: for a selected passage, the descriptive edges in and
  out, rendered as a chain (`Ruiz report → reports → Lee said → that → Morgan left`), with
  the same single-key accept/reject for candidates.
- Occurrence creation: from the Time & Place Index or the Assembly view, `O` on a
  selected set of passages creates an event in a chosen lane and `account_of` edges,
  each `unreviewed`, with one written rationale.
- **Prerequisite:** `@evidence-intake-project.video.soundtrack-pts-offset-question` is
  resolved before any view shows soundtrack utterances against visual intervals on one
  axis. Until then the two stay in separate lanes and the view says so.

## 7. Priority 3 — proposition packets

`PropositionPacket` in `src/views.rs`, `Store::proposition_packet(case, proposition)`,
the new primary workspace ("Packets", reached by its `Ctrl+Shift` accelerator — see §5.1):

1. neutral proposition text and `contested` status (never anything else);
2. directly captured material: linked content whose current interpretation is
   `contemporaneous_capture` and `reviewed`/`verified`;
3. contemporaneous accounts; 4. later first-person accounts (both times shown, labelled);
5. official/documentary assertions; 6. reporting and derivation chains (lineage groups);
7. evaluative relations, each with its rationale and review state;
8. temporal and location conflicts (raw disagreement, no resolution);
9. missing/unreviewed: `expected_but_missing` edges, unreviewed links, unresolved
   speaker candidates, unset alignment;
10. every item carries its exact locator.

Each section's count is the three-way count. Attaching a passage to the proposition from
search or from the Index is one command plus the just-in-time prompt (§5.7); the link is
written `unreviewed` with a required rationale, as `link_evidence` already demands. The
`nothing_in_the_standing_view_scores_the_case` serialization test is extended to packets;
a new test asserts the packet of the fixture's impairment proposition renders the
"no reviewed linked source directly records Morgan's condition at 21:07" section from
the graph, not from prose.

## 8. Priority 4 — deterministic layered digest

`src/digest.rs`, `Store::case_digest(case, audience)`. Sections, in the order the review
gives: case posture and element matrix; recorded sequence (separate lanes where clocks
are unaligned); contemporaneous accounts; later accounts; official documentary narrative;
contested propositions (the packets); source dependencies and unresolved conflicts;
missing and unreviewed material; privileged issues and decisions (work-file audience
only — the disclosable audience never queries those tables, as `export_case` already
does).

- Every factual sentence is produced by a `Template` with typed slots; `DigestSentence`
  carries `template_id` and `Vec<Locator>`. A template whose slots cannot all be
  filled from reviewed records is **not emitted**, and the digest header counts what
  was omitted and why — the `export.md` pattern.
- `Store::validate_digest` fails (`Error::UnsupportedSentence`) on any sentence without a
  locator; the CLI and GUI refuse to save such a digest.
- Decision briefs get `brief_paragraphs` (§3.2); `record_brief` refuses a `factual`
  paragraph with no references. Revision by superseding is unchanged.
- The initial templates are the review's list, verbatim, with the no-speech and
  shared-anchor disclaimers included.

No language model participates in the digest. A later compressor, if ever added, may only
reorder or shorten emitted sentences and must return each sentence's ids and locators
unchanged, with the validator run on its output.

## 9. Testing the product, not the fixture

A new fixture family under `fixtures/raw/` of **synthetic raw files** (small PDFs with
real text layers and scanned-style pages, short WAVs with scripted speech, short MP4s)
standing in for: police report, witness statement, CAD log, 911 audio, body-camera video,
surveillance video, later interview, lab report, and a document referring to a missing
recording. Each adversarial case the review lists gets one scenario and one test in
`tests/assembly.rs`:

| Scenario | What must hold |
| --- | --- |
| report paraphrases the 911 call | `cross-document-echo` proposes `summarizes`; after acceptance the packet counts 2 files, 1 lineage |
| report summarizes bodycam | same; `independently_corroborates` between them is refused |
| ASR creates a false contradiction | the contradiction is a `suggested` finding; verifying the transcript against the original is the only way it leaves the queue |
| no speech detected despite audible speech | the interval is `no_speech_aligned`, never `recording_gap`; the digest sentence carries the disclaimer |
| event outside the field of view | the negative observation stays bounded (Rule 8) and the digest says "within the camera's field of view" |
| two unrelated incidents share date and location | `shared-anchor` proposes; rejecting it leaves them ungrouped and never re-proposes |
| supplemental report changes an account | both versions appear under later accounts with both dates; `supersedes_source_id` is shown, not collapsed |
| bodycam and dispatch clocks disagree | two lanes; `Alignment: not set` until a basis is written; no tolerance window (Rule 34) |
| witness gives two accounts | both under the witness, each with statement time; `contradicts` is human-only |
| report references video never produced | `expected_but_missing` appears in the packet's missing section and the digest |
| interview audio is retrospective, 911 is contemporaneous | same modality, different `temporal_stance`; the packet files them in different sections |

Plus the keystroke budget test (§5.3) and the existing suites.

## 10. Acceptance criteria (the milestone's checks)

- zero factual sentences without a locator in a saved digest (`validate_digest` test);
- zero occurrence declarations without a reviewed `candidate_same_occurrence` /
  `account_of` edge;
- zero `independently_corroborates` edges between nodes sharing a lineage;
- every displayed time carries its semantic label; `Alignment: not set` where unset;
- every rendered passage carries a provenance badge;
- the keystroke budget holds on the raw-file fixture;
- the hit-and-run packets and digest are producible from the raw-file fixture through
  the real workflow (adapters + `InterpretationBatch` + authoring), with no SQL seeding;
- `nothing_in_the_standing_view_scores_the_case` passes over packets and digest;
- `a_disclosable_export_carries_no_privileged_material` still passes and the digest's
  disclosable audience is covered by the same structural exclusion;
- clippy clean under `-D warnings`, `cargo test --all-features` green, both build
  profiles produced.

## 11. Sequencing and AKR

One milestone, `@evidence-intake-project.assembly.case-assembly`, with work records in
this order; each depends on the one before except where noted:

1. `assembly.p0-honest-views` — §4. No dependencies; ships alone.
2. `assembly.interpretation-schema` — §3.2–3.3, §5.9. Migrations 0011, enums, `Store`
   writers, `InterpretationBatch`.
3. `assembly.enrichment-workspace` — §5.1–5.7 GUI and keystroke harness.
4. `assembly.candidate-rules` — §5.4 (can run in parallel with 3 once 2 lands).
5. `assembly.edge-vocabulary-and-lineage` — §3.4, §3.6, §6 (depends on 2; the PTS
   question blocks only the synchronized audio/video view).
6. `assembly.proposition-packets` — §7 (depends on 2, 5).
7. `assembly.deterministic-digest` — §8 (depends on 6).
8. `assembly.raw-file-fixtures` — §9; starts with 1 and grows with every item.

Each work record's acceptance is the subset of §10 it can satisfy; evidence is recorded
with `knowledge.evidence_add` and checked off with `knowledge.complete`. The two open
questions named above are resolved by decisions recorded when items 2 and 5 land.

## 12. What this plan does not do

- It does not add a local language model. The `suggest:` seam is where one would go.
- It does not merge entities, score propositions, resolve clock disagreements, or declare
  occurrences. Every such step is a reviewed edge a person wrote.
- It does not rebuild `content`. The fine classification lives beside the extraction, not
  in it.
- It does not promise the adapters will fill the semantic fields. They fill the locators
  and the text; the workflow makes the rest cheap.
