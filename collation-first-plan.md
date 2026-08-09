## The central correction

The project should not treat **collation as a final intelligence feature built after transcription, OCR, and video analysis**. Collation is the product. Text, audio, and video processing are replaceable ingestion adapters.

Your current plan says that the unified event schema is the heart of the product, but the proposed schema is essentially a search record—source, offset, time, location, text, confidence—and substantive cross-evidence assembly is deferred to Phase 4.   That should be reversed:

1. Build the defender-oriented case model.
2. Populate it manually with test cases.
3. Build the workflows that answer defense questions.
4. Connect OCR, transcription, and video systems afterward.

The core conceptual change is:

> **The primary object is not an event. It is a contested proposition connected to evidence, legal issues, people, and defense decisions.**

A detective-oriented tool tries to reconstruct the most likely account of what happened. A defense-oriented tool asks:

> What can the prosecution prove, with what evidence, through which witness, subject to what challenge—and what materially changes the client’s options?

That orientation follows defense practice standards. The ABA describes defense investigation as including prosecution-evidence evaluation, inconsistencies, impeachment opportunities, other suspects, and alternative theories; it also calls for continually developing and revising a theory of the case. ([American Bar Association][1]) NLADA similarly treats continuing case-theory reassessment, discovery, suppression issues, plea decisions, trial preparation, and sentencing as interconnected defense functions. ([nlada.org][2])

## What the system must answer

Start by defining a canonical set of **defender questions**, independent of modality.

### Case and charge questions

* What are the elements of every charged offense?
* What evidence does the prosecution appear to rely on for each element?
* Which elements have direct evidence, circumstantial evidence, disputed evidence, or no identified evidence?
* What alternative innocent or less culpable explanations fit the same evidence?
* Does the evidence support a lesser offense more strongly than the charged offense?
* What facts are harmful even though they do not establish an element?

This leads naturally to an **element matrix**, not merely a timeline.

### Contradiction and credibility questions

* What has each witness said on every occasion?
* Where did the witness’s account change?
* Does the officer’s report differ from bodycam, dispatch, photographs, or another officer’s report?
* Was the witness in a position to observe what they claim?
* What evidence corroborates or contradicts the statement?
* Is the contradiction substantial, explainable, or probably an extraction error?
* What motive, benefit, bias, prior relationship, or uncertainty may matter?

The result should be a witness dossier containing all attributed statements, ordered by when they were made—not just when the underlying event allegedly occurred.

### Suppression and procedure questions

The tool should not decide that a search or statement was unlawful. It should assemble the factual predicates an attorney needs to evaluate:

* When did the encounter begin?
* When was the client detained, searched, handcuffed, questioned, or advised of rights?
* What reason did the officer give at the time, and what reason appears later in the report?
* Who requested or purportedly gave consent?
* What exactly was said before and after an alleged consent or admission?
* Were there recording gaps at material moments?
* What warrant, affidavit, return, inventory, or dispatch record exists?
* What facts support possible Fourth-, Fifth-, Sixth-Amendment, identification, discovery, or evidentiary issues?

NLADA expressly identifies suppression of searches, statements, right-to-counsel violations, and unreliable identifications among issues defense counsel should consider after factual investigation. ([nlada.org][2])

### Discovery-completeness questions

This may be one of the most valuable portions of the product:

* What production batches were received, and when?
* What files, exhibits, attachments, photographs, recordings, or reports are referenced but absent?
* Are there missing sequence numbers or unexplained time gaps?
* Was a file corrupted, password-protected, proprietary, truncated, or unreadable?
* Did a later production replace or supplement an earlier version?
* What remains unreviewed?
* What potentially material item should be requested, preserved, subpoenaed, or examined by an expert?

The ABA’s electronic-discovery standards emphasize usability, integrity, security, production records, preservation of native material and metadata, and disclosure of an existing index for substantial ESI. ([American Bar Association][3]) Your system should therefore include a **production ledger**, not merely a file library.

### Client-decision questions

The public defender ultimately needs to advise a client, not simply understand a scene:

* What are the strongest and weakest parts of the prosecution case?
* What unresolved factual question could materially change the recommendation?
* What additional investigation is worth doing before a plea deadline?
* What is the evidentiary effect of winning or losing a particular motion?
* What portions of the client’s account are corroborated, contradicted, or not addressed?
* What facts affect release, plea leverage, trial risk, sentencing, restitution, or collateral consequences?
* What does the client need to see or hear to participate meaningfully in the decision?

This means the system must support multiple decision views: release, motions, negotiation, trial, sentencing, and appeal preservation. There is no single universal “case summary.”

## Replace the flat event schema

The existing schema is a useful extraction interchange format, but it is not sufficient for collation. Use several explicit layers.

### 1. Source layer

This remains immutable:

* `Production`
* `Source`
* `SourceVersion`
* `SourceSegment`
* hashes and custody metadata
* original timestamps and device metadata
* exact locators: page, bounding box, timestamp, frame range
* processing history

This preserves the plan’s sound rule that machine products remain pointers to untouched originals. 

### 2. Evidentiary-content layer

This describes what a source actually contains:

* `Statement`
* `Observation`
* `DocumentAssertion`
* `ObjectMention`
* `PersonMention`
* `TimeMention`
* `LocationMention`
* `RecordingGap`
* `ReferenceToOtherEvidence`

A crucial distinction is between:

* a witness saying, “I saw a gun,”
* an officer writing that the witness said this,
* a transcript attributing those words to the witness,
* a machine detector suggesting that a gun-shaped object is visible.

These cannot be stored as equivalent facts.

Statements also need a **reporting chain**:

```text
Police report
  → officer asserts
      → witness previously stated
          → client possessed an object
```

That structure matters for impeachment, hearsay analysis, attribution errors, and source verification.

### 3. Factual-model layer

This contains attorney-reviewable hypotheses:

* `Entity`
* `Event`
* `Proposition`
* `IdentityHypothesis`
* `TimeAlignmentHypothesis`
* `LocationHypothesis`

A proposition might be:

```text
P-104: The client possessed the handgun at approximately 22:14.
```

Evidence links then say:

```text
Officer report paragraph 18       supports
Witness interview at 00:42:17     supports
Bodycam frames 22:13–22:15        does_not_show
Witness’s earlier 911 statement   contradicts
Fingerprint report                inconclusive
Client interview note             disputes
```

The system must not collapse these into a confidence-weighted “truth.” It should preserve the contest.

Useful relationship types include:

* `supports`
* `contradicts`
* `corroborates`
* `impeaches`
* `qualifies`
* `explains`
* `derived_from`
* `refers_to`
* `temporally_overlaps`
* `possibly_same_person`
* `expected_but_missing`
* `requires_follow_up`

### 4. Advocacy layer

This is privileged attorney work product:

* `Charge`
* `Element`
* `DefenseTheory`
* `ProsecutionTheory`
* `LegalIssue`
* `MotionIssue`
* `CrossExaminationPoint`
* `InvestigationTask`
* `NegotiationConsideration`
* `MitigationTheme`
* `AttorneyConclusion`

The advocacy layer links legal or strategic questions to propositions and sources. It must be separately permissioned and excluded from routine exports.

Do not make “admissible” a global boolean. Admissibility can depend on purpose, foundation, jurisdiction, witness availability, rulings, and procedural posture. Track instead:

```text
potential_issue
possible_objection
proposed_purpose
foundation_needed
court_ruling
attorney_status
```

## Time and identity need first-class uncertainty

Criminal evidence frequently contains several different times:

* file creation time;
* device clock time;
* dispatch time;
* time spoken by a witness;
* time inferred from surrounding events;
* attorney-normalized case time.

Never silently replace one with another. Store a raw value, its basis, a possible normalized interval, and any proposed clock offset.

Likewise, separate a `PersonMention` from a resolved `Person`. “Male in red shirt,” “suspect,” and the client may be linked as a hypothesis without being permanently merged. False identity merges are more damaging than leaving two unresolved mentions.

Negative evidence needs similar care:

* “The camera does not show a firearm” is an observation.
* “No firearm was present” is an inference.
* “No firearm evidence was produced” is a discovery statement.
* “The firearm did not exist” is a much stronger proposition.

The software should preserve those distinctions.

## Build it without building the inputs

Define a narrow normalized input contract and create the data by hand initially:

```text
Source
SourceSegment
ExtractedContent
SpeakerOrAuthor
RawTime
AssertedTime
LocationMention
EntityMentions
ExtractorConfidence
OriginalLocator
```

Then construct three manually curated case fixtures, perhaps:

1. A vehicle stop and possession case with a suppression issue.
2. An assault case with conflicting witness accounts and several bodycams.
3. An identification case involving CCTV, reports, photographs, and lineup material.

For each fixture, manually enter perhaps 100–300 statements, observations, references, and propositions. Include deliberate problems:

* inconsistent clocks;
* duplicate productions;
* a report referencing a missing attachment;
* two statements attributed to the same witness;
* uncertain speaker identity;
* partial recording;
* conflicting descriptions;
* a late supplemental report;
* potentially exculpatory material buried in an unrelated file.

This becomes the permanent collation test suite. Later, OCR or Whisper output merely attempts to reproduce the same normalized records.

## Build the user interface around decisions

The first useful interface should contain six views:

1. **Discovery ledger**
   Productions received, missing references, unreadable items, review status, superseded versions and requested material.

2. **Element matrix**
   Each charge and element, with supporting, opposing, uncertain and excluded evidence.

3. **Witness dossier**
   Every statement by or attributed to the witness, contradictions, corroboration, observation conditions and cross-examination notes.

4. **Contested timeline**
   Separate lanes for recorded events, witness accounts, police narrative, client account and attorney hypotheses. Do not display one falsely authoritative timeline.

5. **Issue workspace**
   A suppression, identification, discovery, evidentiary or procedural issue with its required factual questions, linked excerpts, missing facts and follow-up tasks.

6. **Client/decision brief**
   A human-written summary of strengths, risks, unresolved questions and options, with every factual sentence linked to its source.

Search remains useful, but search should land in one of these structures. A list of matching transcript fragments is not collation.

## Conduct different interviews

The market-validation interviews in the plan focus mainly on deployment, cloud policy, hardware and willingness to pilot.  Add a separate **cognitive-task interview**.

Ask a defender to bring a closed or suitably redacted case and walk through it chronologically:

* What decision were you making at this point?
* What did you need to know?
* Which two or three records did you compare?
* What discrepancy mattered?
* What did you write in your own notes?
* What did you need to ask the client?
* What caused you to request more discovery?
* What changed your plea or trial assessment?
* What did you eventually need in a motion, cross-examination, client letter, or sentencing submission?
* What information did you discover too late?

Do not ask primarily, “What software features would you like?” The useful output is a catalogue of decision points, questions, source comparisons, and attorney-created artifacts.

## Revised project order

### Phase A — Defense question model

Produce:

* canonical defender-question catalogue;
* charge/element template format;
* legal-issue template format;
* epistemic and provenance rules;
* confidentiality and export boundaries.

### Phase B — Collation kernel

Implement:

* sources and segments;
* statements and observations;
* entities, events and propositions;
* typed evidence relationships;
* production ledger;
* review and verification states;
* versioned attorney annotations.

SQLite is adequate. Use ordinary relational tables plus a typed `edges` table; a dedicated graph database is unnecessary initially.

### Phase C — Defender workspace

Implement the element matrix, witness dossier, contested timeline, issue workspace and source-linked exports. Populate everything from hand-authored fixtures.

### Phase D — Assisted collation

Add machine suggestions for:

* possible duplicate entities;
* temporal alignment;
* statement clustering;
* contradiction candidates;
* report-to-video discrepancies;
* referenced-but-missing evidence;
* candidate links to charges and issues.

Every suggestion should remain visibly unconfirmed until reviewed.

### Phase E — Modality adapters

Connect OCR, ASR, diarization, object detection and scene descriptions. The gun-detection paper, for example, describes an efficient classify-then-localize input pipeline; it does not solve legal collation.  LingBot-Map likewise belongs much later as a possible spatial-analysis source because it produces pose and depth estimates rather than defense reasoning. 

## The MVP test

The first success criterion should not be transcription accuracy or the number of indexed files. Give a defender an unfamiliar fixture case and measure whether the system lets them reliably answer:

* What is the state’s evidence for each element?
* What material contradictions exist?
* What evidence appears to be missing?
* What are the strongest potential motion issues?
* What should be discussed with the client?
* What additional investigation is likely to matter?
* Can every answer be verified against the original in one action?

That is a defensible collation MVP. The input systems can remain manual until it works.

[1]: https://www.americanbar.org/groups/criminal_justice/resources/standards/defense-function/ "Defense Function"
[2]: https://www.nlada.org/defender-standards/performance-guidelines/black-letter "Performance Guidelines for Criminal Defense Representation (Black Letter) | National Legal Aid & Defender Association"
[3]: https://www.americanbar.org/groups/criminal_justice/resources/standards/discovery/ "Discovery"
