# Evidence Intake

A local-first, defender-oriented evidence collation kernel written entirely in
Rust. This repository follows [`collation-first-plan.md`](collation-first-plan.md):
the primary object is a contested proposition connected to source-grounded
evidence, legal issues, people, and defense decisions. OCR, transcription, and
video analysis are later ingestion adapters.

This initial milestone provides:

- immutable production, source, source-version, and exact-segment provenance;
- statements, observations, document assertions, recording gaps, and nested
  reporting chains;
- unresolved entity mentions, raw and normalized times, events, and contested
  propositions;
- typed, reviewable evidence relationships that preserve disagreement;
- a separate privileged advocacy layer with versioned annotations;
- an append-only human review trail in which verification must cite the exact
  original it was checked against;
- human authoring of contested propositions and typed, reasoned evidence links,
  which enter unreviewed because writing something down is not checking it;
- charges recorded with their statutory elements, and attributed element
  assessments that record a direction rather than a score;
- privileged work product — issues, notes, and decision briefs — versioned by
  superseding, so an earlier reading stays readable next to the current one;
- audience-aware source-linked export, in which every factual line resolves to an
  exact original and a disclosable export never reads the privileged tables;
- deterministic assisted collation: eight analyzers that either propose a
  relationship for review — overlaps, contradiction candidates, conflicting
  attributions, possible duplicate people — or report a gap to close, each
  stating a reason a defender can reconstruct;
- decision-oriented discovery, element, witness, timeline, issue, and brief
  read models;
- an adapter-neutral contract for already-OCRed documents, timestamped audio
  transcripts, and scene-by-scene video observations;
- hand-authored vehicle-stop and hit-and-run fixtures containing clock disagreement, a
  superseding report, conflicting witness attributions, an interrupted
  recording, referenced-but-missing evidence, later impairment observations,
  and a charged-versus-lesser offense comparison.

The CLI is currently the thinnest useful shell around the kernel. It emits JSON
so the planned Windows-only WinSafe GUI can consume stable Rust read models
without coupling itself to SQLite. No cross-platform GUI framework is planned.

## Run

The checked-in toolchain file selects current Rust nightly.

```sh
cargo run -- init
cargo run -- seed vehicle-stop
cargo run -- seed hit-and-run
cargo run -- cases
cargo run -- view case-vehicle-stop-001 overview
cargo run -- view case-vehicle-stop-001 discovery
cargo run -- view case-vehicle-stop-001 elements
cargo run -- view case-vehicle-stop-001 witness person-patel
cargo run -- view case-vehicle-stop-001 timeline
cargo run -- view case-vehicle-stop-001 issues
cargo run -- view case-vehicle-stop-001 brief motions
cargo run -- view case-hit-run-001 offenses
cargo run -- view case-hit-run-001 proposition hr-prop-impaired-driving
cargo run -- review case-hit-run-001 queue
cargo run -- review case-hit-run-001 history
cargo run -- author case-hit-run-001 proposition \
  --text "Morgan did not perceive the impact." --author "A. Reyes"
cargo run -- author case-hit-run-001 link \
  --from-kind content --from hr-content-client-driving --relation supports \
  --to-kind proposition --to <proposition-id> \
  --rationale "Morgan expressly disputes awareness of any impact." \
  --author "A. Reyes"
cargo run -- author case-hit-run-001 work \
  --kind motion-issue --title "Timing of the stop" \
  --body "First reading of the interval." --author "A. Reyes"
cargo run -- view case-hit-run-001 work-history <item-id>
cargo run -- view case-hit-run-001 notes content hr-content-911-injury
cargo run -- export case-hit-run-001
cargo run -- export case-hit-run-001 --audience work-file
cargo run -- suggest case-vehicle-stop-001
cargo run -- author case-vehicle-stop-001 entity --kind person --name "Patel"
```

`export` produces the source-linked record: every factual line carries the exact
original it rests on, and the default `disclosable` audience never reads the
privileged tables. See [`docs/export.md`](docs/export.md).

Machine suggestions stay suggestions until a person acts on them. `review
queue` lists what is waiting with the exact locator to open, and `review apply`
records the decision; see [`docs/review-workflow.md`](docs/review-workflow.md)
for what each state costs.

`author` is how a person adds their own reading rather than an adapter's: a
contested proposition, and typed relationships tying content to it. Both enter
unreviewed and join the same queue, and every relationship carries a written
rationale, because an edge spans sources and has no original of its own. See
[`docs/authoring.md`](docs/authoring.md).

Use `--database PATH` before the subcommand to select another case database.
SQLite databases are ignored by Git.

## Current boundary

This is the collation kernel and its first curated fixture, not an evidence
conclusion engine. It deliberately does not:

- compute a truth/confidence score for propositions;
- label evidence globally admissible or inadmissible;
- merge uncertain person mentions automatically;
- overwrite device or spoken times with normalized time;
- expose privileged advocacy records in the discovery ledger;
- perform OCR, ASR, diarization, or video inference itself.

The input boundary assumes those modality pipelines already ran. Their outputs
enter through `NormalizedBatch`; machine content must begin as `suggested` and
preserves model version, confidence, original-source hash, and exact locator.

Human review and human authoring are now first-class mutations, the first with
its own immutable trail and the second with versioned work product that
supersedes rather than overwrites. Source-linked export completed the
defender workspace, and assisted collation now covers the Phase D
analyzer set — deterministically, with no model and no score. Four analyzers
propose relationships that wait on a person; four report gaps that are dismissed
only by closing them. The remaining milestones are the modality adapters that
feed `NormalizedBatch` and the WinSafe GUI.

One thing it deliberately still refuses: changing an element assessment. Reading
evidence differently later is an honest act that should leave a trail, but
whether that trail is the defender's own working record or an accountability
audit log decides its schema, so it is recorded as an open question rather than
guessed at.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```
