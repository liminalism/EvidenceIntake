# Verdict

Your concern is correct. The project has a strong **epistemic and provenance foundation**, but it does not yet perform the kind of collation described by its authoritative design plan.

The live pipeline currently does this:

```text
original document/audio/video
        ↓
text, transcript, scene observations, and machine suggestions
        ↓
source-linked content records
        ↓
search, review queues, and a time/location index
        ↓
manually authored propositions and relationships
```

The missing section is the actual assembly stage:

```text
source-linked content
        ↓
reviewed semantic interpretation
        ↓
accounts, reporting chains, occurrence candidates, and source dependencies
        ↓
contested propositions
        ↓
layered, source-linked case digest
```

The project should **not** aim to merge everything into one narrative whole. The authoritative plan expressly rejects a single universal case summary and makes the **contested proposition**, rather than the event, the primary organizing object (`collation-first-plan.md:14`, `collation-first-plan.md:90`).

The right target is better described as a:

> **Source-linked contested case digest**

or:

> **Proposition-centered case assembly**

The central rule should be:

> **Do not merge media into a story. Assemble distinct accounts, observations, measurements, and reporting relationships around contested propositions.**

## Current assessment

| Area                             | Current condition                                                                         | Assessment            |
| -------------------------------- | ----------------------------------------------------------------------------------------- | --------------------- |
| Original-file provenance         | Immutable sources, exact locators, machine/human provenance                               | Strong                |
| Epistemic safety rules           | Good separation of evidence, propositions, machine suggestions, and attorney work product | Strong                |
| Basic ingestion                  | All three modalities produce normalized content                                           | Functional            |
| Time/location collation          | Implemented as a conservative navigation index                                            | Useful but mislabeled |
| Semantic interpretation          | Rich schema exists, but real adapters do not populate it adequately                       | Weak                  |
| Cross-modal assembly             | Mostly absent unless a human manually creates propositions and links                      | Weak                  |
| Contested timeline               | Supported by the schema and fixtures, but not realistically authorable from live imports  | Incomplete            |
| Witness/reporting-chain analysis | Designed but usually lacks speaker, attribution, and parent-report data                   | Incomplete            |
| Digest generation                | No enforced proposition-centered, sentence-linked case digest                             | Not implemented       |
| Risk of overstatement            | Reduced by design rules, but several current UI/data choices can still mislead            | Material              |

The product is currently best described as a:

> **Provenance-preserving evidence index and reviewer graph kernel**

That is worthwhile. It is not yet the complete collation product promised by the plan.

---

# What the current "collation" feature actually does

The current collation documentation accurately describes a conservative source-grounded navigation feature. It groups material by:

* normalized date;
* exact normalized location;
* possible relation where multiple sources share date and location;
* anchor coverage and placement gaps.

It expressly avoids claiming that grouped passages refer to the same event (`docs/collation.md:1–20`, `docs/collation.md:46–49`).

The implementation matches that narrow definition:

* `CollationEntry` contains source, locator, passage, several time fields, location, and review state, but not the full epistemic or proposition context needed for actual synthesis (`src/views.rs:140–175`).
* `CollationIndex` consists primarily of date groups, location groups, "possibly related" groups, coverage, and placement gaps (`src/views.rs:224–245`).
* The store query groups by date and exact normalized location; it does not assemble content by proposition, reporting chain, account, or occurrence (`src/store.rs:1307–1490`).
* The GUI calls this **TIME AND LOCATION COLLATION** and renders those groups (`src/gui/mod.rs:586–690`).

That is an index, not substantive collation.

There is nothing wrong with retaining it. It should be renamed:

* **Collation Groups** → **Time & Place Index**
* **Run Collation** → **Run Structural Checks**
* **Possibly Related** → **Shared Anchor — Relationship Unconfirmed**

A new **Case Assembly** workspace should perform the real collation.

## "Run Collation" is particularly misleading

The GUI's button invokes `suggest_all()` and announces that deterministic collation is complete (`src/gui/winsafe.rs:641–645`).

But those analyzers operate on a graph that must already contain:

* propositions;
* proposition-to-content links;
* events;
* speaker attributions;
* contradictory evidence relations.

They can identify tensions or structural gaps in an already assembled graph. They do not construct that graph from the imported evidence (`src/suggest.rs:1–17`, `src/store.rs:2440–2492`).

A more accurate label is:

> **Find Tensions and Gaps**

---

# The main end-to-end failure

The database model can represent much more than the adapters actually produce.

The hand-built fixtures contain:

* document assertions;
* witness statements;
* officer recollections;
* contemporaneous recorded statements;
* visual observations;
* test results;
* reporting chains;
* event lanes;
* propositions;
* supports, contradicts, qualifies, and explains relationships.

That is close to the intended product.

But the live adapters largely produce flattened excerpts.

## Documents

Every OCR region is currently mapped as a generic `Statement`. The adapter leaves the following unset:

* author or speaker;
* attributed person;
* parent reporting chain;
* asserted-event time;
* creation time;
* normalized time;
* location.

See `adapters/document/src/map.rs:108–157`.

This means the system cannot distinguish:

1. an officer stating a personal observation;
2. an officer paraphrasing a witness;
3. a direct quotation from the witness;
4. a legal characterization;
5. a reference to video or another report;
6. a lab result;
7. a contemporaneous dispatch entry;
8. a report written days after the alleged event.

The normalized-input documentation already anticipates these distinctions, including `document_assertion`, report time, asserted-event time, and reporting chains. The real adapter does not yet reproduce that contract.

## Audio

ASR segments become statements, but normally have no resolved:

* speaker;
* attributed person;
* reporting parent;
* content-level time;
* location.

Diarization labels are stored as separate observations instead of being connected to the utterance as a proposed speaker assignment (`adapters/audio/src/map.rs:228–251`, `adapters/audio/src/map.rs:302–318`).

This leaves the reviewer with transcript text and separate speaker labels, rather than an explicit proposition such as:

> "Diarization suggests Speaker 2 uttered this segment; reviewer has not resolved Speaker 2 to a person."

There is also a substantive classification problem: intervals in which no speech is aligned are represented as `RecordingGap` (`adapters/audio/src/map.rs:321–349`).

Those are not equivalent.

A no-speech interval might mean:

* silence;
* music or environmental sound;
* unintelligible speech;
* overlapping speakers;
* ASR failure;
* low-confidence speech detection;
* an actual missing portion of the recording.

`RecordingGap` should be reserved for demonstrated stream loss, truncation, corruption, or missing media. The ASR result should use something like:

* `NoSpeechAligned`;
* `UntranscribedInterval`;
* `SpeechDetectionNegative`.

## Video

Video scenes, detections, and captions are intentionally conservative. That is good. The caption prompt avoids identity, intent, causation, and event-sequence claims.

But the output remains mostly:

* generic scene observations;
* object or feature suggestions;
* source-relative locators;
* machine-generated navigational hints.

It does not create the reviewed semantic relationship:

> "This recording directly depicts a bounded visual condition during this interval."

Nor does it connect the soundtrack transcript to a visual scene except through their common source and approximate locators.

The video output is consequently useful for finding material, but not yet for assembling the evidentiary meaning of that material (`adapters/video/src/map.rs:243–426`).

---

# Medium is not the same as evidentiary role

The system should not encode the simplistic rule that video and audio are direct evidence while documents are retrospective reports.

A body-camera recording may contemporaneously capture an event. An interview recording may contain a recollection made three weeks later. A jail call may contain a later statement, hearsay, speculation, or discussion of a report.

Likewise, a document may be:

* a later police narrative;
* a contemporaneous CAD log;
* a signed witness statement;
* a laboratory measurement;
* a timestamped receipt;
* a medical record;
* an evidence inventory;
* a report summarizing a recording;
* a transcript derived from another source.

The semantic model therefore needs at least three independent axes:

### 1. Source medium and role

Examples:

* PDF police report;
* witness statement form;
* CAD export;
* body-camera recording;
* surveillance video;
* 911 call;
* recorded interview;
* laboratory result.

### 2. What the content unit is

Examples:

* recorded utterance;
* authored assertion;
* quoted statement;
* report of another person's statement;
* direct visual observation;
* instrument measurement;
* evidentiary reference;
* official or legal characterization;
* machine-generated suggestion.

### 3. How it relates to the alleged occurrence

Examples:

* contemporaneous capture;
* contemporaneous account;
* later first-person recollection;
* report of a report;
* later official interpretation;
* measurement made after the alleged event;
* temporal relation unknown.

File extension or modality can suggest defaults. It must not determine these classifications.

---

# The missing component: reviewed semantic annotation

Raw extracted content should remain immutable. The project should add a versioned, append-only semantic layer over it.

A possible object is:

```text
ContentInterpretation
    raw_content_id
    content_form
    source_role
    speaker_or_author
    attributed_person
    reporting_parent
    perception_basis
    temporal_stance
    source_relative_time
    creation_time
    asserted_event_interval
    normalized_case_interval
    time_alignment_basis
    location
    review_state
    created_by
    supersedes
```

## Important fields

### `content_form`

Suggested controlled vocabulary:

* `recorded_utterance`
* `authored_assertion`
* `quoted_statement`
* `reported_statement`
* `visual_observation`
* `measured_result`
* `evidence_reference`
* `official_characterization`
* `machine_suggestion`
* `no_speech_aligned`
* `recording_loss`

### `perception_basis`

This describes what grounds the assertion:

* saw;
* heard;
* measured;
* recorded;
* read in another source;
* told by another person;
* inferred or characterized;
* unknown.

### `temporal_stance`

This should be content-level rather than merely source-level:

* contemporaneous capture;
* contemporaneous account;
* retrospective recollection;
* report of prior statement;
* later measurement;
* later analysis;
* unknown.

One police report can contain all of these within a single page.

### Reporting chain

The system must be able to represent:

```text
Officer Ruiz's report
    reports that
Witness Lee said
    that
Morgan left the scene
```

That is not equivalent to:

```text
Officer Ruiz personally saw Morgan leave.
```

The product's existing domain rules understand this distinction, but the live ingestion and authoring workflow does not yet make it practical.

---

# Cross-source relations need a larger vocabulary

The current structural and evaluative edge types are not enough to prevent evidence from being double-counted or mischaracterized.

Add descriptive provenance relationships such as:

* `quotes`
* `reports`
* `summarizes`
* `transcribes`
* `depicts`
* `records_utterance`
* `measures`
* `based_on`
* `refers_to`
* `account_of`
* `created_after`
* `recorded_during`
* `candidate_same_occurrence`

Keep these separate from attorney evaluative relationships:

* `supports`
* `contradicts`
* `qualifies`
* `explains`
* `impeaches`

## "Corroborates" needs special treatment

Two sources that say the same thing are not necessarily independent corroboration.

For example:

```text
911 caller makes statement
        ↓
bodycam officer repeats what dispatcher said
        ↓
police report summarizes bodycam and 911
```

Those may appear as three separate originals while representing one information lineage.

I would replace a generic `corroborates` relation with:

* `consistent_with`
* `independently_corroborates`

The second should require an explicit reviewer determination about source independence and a written basis. The system should otherwise display:

> "Three records contain this assertion; two appear to derive from the same reporting chain."

That is far safer than:

> "Three sources corroborate the assertion."

---

# The correct collation unit: a proposition packet

The main unit shown to the PD should be a **Contested Fact Card** or **Proposition Packet**.

Each packet should contain:

1. **Neutral proposition**
   The factual proposition without argumentative wording.

2. **Directly captured material**
   Reviewed audio, video, photographs, or measurements that directly bear on it.

3. **Contemporaneous accounts**
   Statements made during or immediately around the alleged occurrence.

4. **Later first-person accounts**
   Recollections shown with both event time and statement time.

5. **Official or documentary assertions**
   Clearly identified as what a report or record states.

6. **Reporting and derivation chains**
   Which items quote, summarize, transcribe, or depend on others.

7. **Supporting, opposing, qualifying, and explanatory material**
   Each as an explicit reviewed relation.

8. **Temporal and location conflicts**
   Without automatically resolving them.

9. **Missing or unreviewed evidence**
   Including referenced but absent recordings and incomplete review.

10. **Exact locators**
    Every item should open the original page, timestamp, frame interval, or waveform segment.

There should be no overall truth score.

---

# Example of the desired output

Using the hit-and-run fixture, an impairment proposition should look roughly like this:

## Proposition

**Morgan was impaired while driving at approximately 21:07.**

**Status:** Contested. No conclusion entered.

### Material directly capturing the alleged driving period

No reviewed linked source directly records Morgan's physical condition at approximately 21:07.

### Later officer observation

At approximately 22:04, Officer Ruiz recorded observations concerning odor, watery eyes, and balance. These observations concern Morgan's condition approximately 57 minutes after the alleged driving period. They do not directly establish Morgan's condition at 21:07.

### Measurement

A test record at approximately 22:42 reports a result of 0.060. No linked material contains a reviewed retrograde estimate connecting that result to the alleged driving time.

### Client account

Morgan states that two drinks were consumed after arriving home. The currently linked evidence does not independently establish the arrival time or drinking interval.

### Official report

Officer Ruiz's later report expresses suspicion based on the later observations. The report is an official assertion and should not be counted as independent evidence from the observations it describes.

### Unresolved matters

* Time of arrival at home.
* Time and quantity of post-arrival drinking.
* Condition during the interval between approximately 21:07 and 22:04.
* Whether additional recordings or witnesses cover that interval.

This format does not tell the PD what happened. It shows:

* what each source actually contributes;
* when the observation or statement was made;
* what is direct versus retrospective;
* where the inferential gap lies;
* whether apparently separate records share one origin.

That is genuine collation.

---

# The final digest should be layered, not narratively flattened

A useful case-level digest could have these sections:

## 1. Case posture and charged elements

A compact element matrix showing what propositions and evidence currently bear on each element.

## 2. Recorded sequence

Only reviewed, contemporaneously captured material. Separate source lanes where clocks have not been aligned.

## 3. Contemporaneous accounts

911 calls, immediate statements, CAD entries, and similar records.

## 4. Later accounts

Witness interviews, client interviews, supplemental reports, and later recollections. Always display both:

* alleged-event time;
* statement or record-creation time.

## 5. Official documentary narrative

What reports, affidavits, and records state, preserving attribution and reporting chains.

## 6. Contested propositions

The proposition packets described above.

## 7. Source dependencies and unresolved conflicts

Statements repeated across documents, uncertain clock alignment, conflicting attribution, and non-independent records.

## 8. Missing and unreviewed material

Referenced recordings, missing attachments, incomplete transcription, unresolved speakers, and evidence not yet inspected.

## 9. Privileged issues and decisions

Human-authored defense analysis, client questions, investigation tasks, and legal issues.

This gives the PD a coherent case understanding without manufacturing one authoritative chronology.

---

# Current features that can lead the reviewer too far

Despite the strong domain rules, several implementation details should be corrected.

## 1. Different time meanings are collapsed in the UI

The collation renderer selects the first available value from normalized time, asserted time, creation time, and raw time, and displays it as an undifferentiated time (`src/gui/mod.rs:671–677`).

That can make:

* the date a report was written;
* the date an officer says an event occurred;
* a source-relative timestamp;
* a normalized event time

look interchangeable.

Every displayed time must carry a label:

```text
Recording time:
Statement made:
Report created:
Alleged event time:
Normalized case time:
Alignment status:
```

Where there are multiple times, show all of them.

## 2. Shared date and location can imply more than intended

The wording "possibly related" is cautious, but putting two passages under that heading still primes a reviewer to treat them as referring to one occurrence.

Use:

> **Shared reviewed date/location anchor — relationship not established**

Do not turn that grouping into an event until a reviewer accepts a `candidate_same_occurrence` relationship.

## 3. Source count can be mistaken for independent support

The product can count distinct originals, but distinct files are not necessarily distinct information sources.

Every source count should distinguish:

```text
3 original files
2 reporting lineages
1 apparently independent observation
```

## 4. Machine and reviewed observations need stronger visual separation

A video caption, detector result, ASR transcript, and human-reviewed observation should not appear as equivalent prose entries.

Use permanent badges such as:

* `MACHINE SUGGESTION`
* `RAW TRANSCRIPT`
* `HUMAN-VERIFIED TRANSCRIPT`
* `REVIEWED VISUAL OBSERVATION`
* `DOCUMENT ASSERTION`
* `REPORT OF STATEMENT`

## 5. The decision brief is not structurally source-linked

The plan calls for a human-written brief in which every factual sentence is linked to evidence. The current brief schema is mainly free-text fields (`migrations/0001_collation.sql:195–207`).

Add sentence- or paragraph-level references to:

* proposition IDs;
* content IDs;
* exact locators;
* issue IDs.

The system should refuse to mark a factual paragraph complete when it has no source chain.

---

# Do not solve this with one summarization prompt

Concatenating all generated text files and asking an LLM to write a narrative would be the most dangerous solution.

A free-form narrative model will tend to:

* combine report date with event date;
* treat a report's paraphrase as a second source;
* silently resolve inconsistent accounts;
* convert "does not appear in this camera view" into "did not happen";
* identify speakers from context without review;
* promote machine captions to facts;
* omit qualifiers and missing evidence;
* prefer a clean chronology over an honestly contested one.

An LLM can later assist with:

* proposed content classification;
* proposed reporting chains;
* proposed account clustering;
* proposition candidates;
* draft compression of already reviewed proposition packets.

But factual digest generation should initially be a **deterministic renderer over accepted graph records**.

A later language model should only be allowed to reorder or compress approved material. Every generated clause should return the exact proposition/content IDs and locators supporting it. Unsupported output should fail validation rather than merely display a warning.

Useful controlled templates include:

* "The report created on [date] states that…"
* "[Person] is recorded as saying at source time [time]…"
* "The ASR transcript renders the segment as…; transcription not yet verified."
* "Reviewed frames from [time range] show… within the camera's field of view."
* "No speech was aligned during [interval]; this does not establish recording loss."
* "These records share a reviewed time and location anchor; a common occurrence has not been established."
* "The report summarizes the same recorded statement and is not counted as independent corroboration."

---

# Recommended implementation order

## Priority 0: Correct misleading semantics

These changes should come first because they reduce current reviewer risk without requiring the full assembly system.

1. Rename the current collation page and button.
2. Replace ASR-derived `RecordingGap` with `NoSpeechAligned` or `UntranscribedInterval`.
3. Display every time with its semantic type.
4. Display modality, content form, review status, and machine provenance on every entry.
5. Replace "possibly related" with explicitly non-conclusive wording.
6. Do not display distinct-file counts as corroboration counts.

## Priority 1: Build semantic enrichment authoring

Add UI and append-only APIs to:

* classify a content unit;
* split or group extracted passages without altering the raw extraction;
* assign author, speaker, and attributed person;
* connect a diarization label to a person;
* create reporting-chain parents;
* enter creation and asserted-event times;
* align source-relative time to case time with a recorded basis;
* assign location;
* create and edit occurrence candidates;
* supersede an earlier semantic annotation.

The schema already supports portions of this, but the normal user workflow does not.

The unresolved AKR issue concerning later enrichment of an identical immutable source also needs resolution. The correct model is:

```text
one immutable original
many append-only extraction and interpretation passes
```

not repeated replacement of the source.

## Priority 2: Add source dependency and occurrence assembly

Implement reviewed candidate edges for:

* quote/report chains;
* derived transcripts;
* document summaries of recordings;
* soundtrack-to-video alignment;
* same-occurrence candidates;
* account-of relationships;
* shared source lineage.

The unresolved soundtrack PTS-offset problem should be fixed before presenting synchronized audio/video chronology. An offset error can move every spoken statement relative to the visual sequence.

## Priority 3: Implement proposition packets

Create the primary PD workspace around contested propositions rather than files or chronological excerpts.

Candidate generation may suggest that several content units concern the same proposition. The reviewer must accept or reject those links.

The system should always preserve the distinction between:

* "these passages concern similar subject matter";
* "these are accounts of the same occurrence";
* "these bear on the same proposition";
* "these are consistent";
* "these independently corroborate one another."

## Priority 4: Build the deterministic case digest

Render the layered case digest from accepted records.

Every factual sentence should be generated from a controlled template or carry a validated evidence chain. The privileged decision brief can select and discuss proposition packets, but its factual clauses should remain source-linked.

---

# Testing the actual product rather than the ideal fixture

The current rich SQL fixture demonstrates that the read models work when someone has already built the ideal graph. That does not validate live collation.

The end-to-end test set should begin with raw files:

* police-report PDF;
* witness-statement PDF;
* CAD or dispatch log;
* 911 audio;
* body-camera video;
* surveillance video;
* later recorded interview;
* laboratory report;
* a document referring to a missing recording.

Then measure whether the real workflow can produce the proposition packets and digest without direct database seeding.

## Required adversarial cases

Include cases where:

* a police report paraphrases a 911 call;
* an officer report summarizes body-camera observations;
* ASR creates a false contradiction;
* no speech is detected despite audible speech;
* video does not show an event because it occurs outside the field of view;
* two unrelated incidents share date and location;
* a supplemental report changes an earlier account;
* body-camera and dispatch clocks disagree;
* a witness gives one account immediately and another later;
* a report references video that was never produced;
* interview audio is retrospective while 911 audio is contemporaneous.

## Acceptance criteria

The most important criteria are not generic summarization quality scores.

* **Zero unsupported factual sentences** in a completed digest.
* **Zero automatic same-occurrence declarations** without a reviewed relationship.
* **Zero independent-corroboration counts** that include derived or reporting-dependent material.
* Every factual clause opens the exact original locator.
* Every statement clearly indicates who spoke or authored it, who is being quoted, and when the statement was made.
* Reviewers can distinguish direct capture, later recollection, report-of-report, measurement, and attorney proposition without opening a secondary screen.
* Unresolved time alignment remains visibly unresolved.
* Negative video/audio observations remain bounded by coverage, field of view, audibility, and review status.
* The PD can answer the plan's core questions about elements, contradictions, suppression, discovery completeness, and client decisions without first constructing a separate spreadsheet.

# Bottom line

The project does not need a new foundation. Its provenance rules, contested-proposition model, separate timeline lanes, and structure-not-verdict doctrine are sound.

It needs the missing middle:

> **raw extraction → reviewed semantic interpretation → source dependency and account assembly → proposition packets → layered digest**

At present, the system preserves and indexes multimodal outputs, while the fixture manually supplies most of the semantic work that "collation" is supposed to accomplish. Until that middle layer exists, calling the present time/location index "collation" overstates the product and obscures the actual gap.

This was a static review of the plan, AKR records, schema, adapters, fixtures, store queries, and GUI paths. I did not execute the Rust test suite because the available environment did not include the Rust toolchain; the recorded passing tests validate the implemented behaviors, but they do not change the end-to-end semantic gap described above.
