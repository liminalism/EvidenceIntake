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
- a case-standing view answering what each element actually rests on: how many
  distinct originals are behind it, whether one source alone carries it, what
  nobody has checked, where evidence pulls both ways, and which gaps in the
  record touch a charge — structure a defender can act on, never a score;
- full-text search over extracted content, with the exact original locator on
  every hit and privileged work product structurally out of reach;
- text-to-frame retrieval over coverage- and scene-sampled keyframes, selecting
  a bounded candidate pool internally and presenting it chronologically,
  never reporting a similarity number, with vectors stored per case and
  excluded from every export;
- decision-oriented discovery, element, witness, timeline, issue, and brief
  read models;
- an adapter-neutral contract for already-OCRed documents, timestamped audio
  transcripts, and scene-by-scene video observations;
- hand-authored vehicle-stop and hit-and-run fixtures containing clock disagreement, a
  superseding report, conflicting witness attributions, an interrupted
  recording, referenced-but-missing evidence, later impairment observations,
  and a charged-versus-lesser offense comparison.

The CLI remains the thinnest shell around the kernel. A native WinSafe workspace
now exposes the same read models and mutations on Windows. WinSafe is an opt-in
feature behind a platform-neutral application layer, so a future Linux shell can
reuse the workflow without pulling Win32 into the default build.

## Run

The checked-in toolchain file selects current Rust nightly.

```sh
cargo run -- init
cargo run -- new-case --name "State v. Hall" --reference PD-2026-0900
cargo run -- new-production case-id-from-new-case --label "Brady disk 1" --from Prosecution
cargo run -- seed vehicle-stop
cargo run -- seed hit-and-run
cargo run -- cases
cargo run -- productions case-hit-run-001
cargo run -- view case-vehicle-stop-001 overview
cargo run -- view case-vehicle-stop-001 standing
cargo run -- view case-vehicle-stop-001 discovery
cargo run -- view case-vehicle-stop-001 elements
cargo run -- view case-vehicle-stop-001 witness person-patel
cargo run -- view case-vehicle-stop-001 timeline
cargo run -- view case-vehicle-stop-001 collation
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
cargo run -- search case-hit-run-001 'hatchback'
cargo run -- search case-hit-run-001 '"paint transfer"' --limit 5
cargo run -- index-frames case-hit-run-001 embeddings.json
cargo run -- find-frames case-hit-run-001 --model test-clip '[1.0, 0.0]'
cargo run -- suggest case-vehicle-stop-001
cargo run -- author case-vehicle-stop-001 entity --kind person --name "Patel"
```

`standing` is the view to open first: what each element rests on, how many
distinct originals are behind it, which source alone carries it, and which gaps
touch a charge. It reports structure and never a verdict — see
[`docs/standing.md`](docs/standing.md). `search` finds a passage by its words and
hands back the exact locator to open it in the original; see
[`docs/search.md`](docs/search.md).

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

### Windows GUI

```powershell
cargo run --features gui-winsafe --bin evidence-gui -- evidence.db
```

The GUI is deliberately workflow-first: choose a case, navigate the standing,
discovery, element, timeline, issue, review, search, collation, packet, digest,
and export views, then use the lower action panel for named review decisions,
normalized-batch intake, and typed authoring JSON. Buttons use Win32 mnemonic
markers, so `Alt` plus the underlined letter activates the corresponding
command; every view also carries a `Ctrl` accelerator, because the Alt
namespace ran out before the views did. The safe export is `disclosable`; the
privileged work-file export is a separate action. **Time & Place Index**
presents time/location anchors and placement gaps as a readable review sheet
rather than raw JSON; `Alt+G` opens it directly.

**Enrichment Sweep** (`Alt+X` or `Ctrl+E`) is where the semantic layer is
entered. No adapter can say whether a sentence is an assertion, a quotation or
a report of somebody else's words, so a person does — one source profile
inherited by every passage in the file, then one keystroke per exception, with
the original beside the grid. See [`docs/enrichment.md`](docs/enrichment.md).

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
only by closing them.

The kernel now also answers the question the rest of it was built to serve:
`standing` reports what each charge actually rests on, and `search` finds the
passage behind it. Those are read models, not new claims — the line they hold is
rule 35, that structure may be reported and a verdict may not. The first WinSafe
workspace consumes them now; remaining product work is deeper native workflow
polish and the modality adapters that feed `NormalizedBatch`.

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
