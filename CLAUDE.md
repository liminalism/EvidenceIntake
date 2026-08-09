# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings   # lints are warn-level in Cargo.toml; CI-style runs deny
cargo test --all-features
cargo test --test review verifying_content_requires_the_records_own_locator   # single test
cargo test --test collation                                                    # one integration file
```

`rust-toolchain.toml` pins nightly; edition 2024, `unsafe_code = "forbid"`, `missing_docs = "warn"`
(every public item needs a doc comment, or the build warns).

Manual exercise of the CLI:

```sh
cargo run -- init                      # create/migrate ./evidence.sqlite
cargo run -- seed hit-and-run          # or vehicle-stop; idempotent
cargo run -- view case-hit-run-001 overview
cargo run -- review case-hit-run-001 queue
```

`--database PATH` is global and must precede the subcommand. Every command prints pretty JSON.

## Architecture

A local-first, defender-oriented **evidence collation kernel**: one library crate
(`evidence_intake`) plus a thin `evidence` CLI. There is no server, no network, no OCR/ASR/video
model — those are external adapters that feed the kernel through a normalized contract.

Data flow: `NormalizedBatch` (ingest) → SQLite (store) → read-model structs (views) → JSON (main).

- `src/store.rs` — the only module that touches SQLite, and where nearly all logic lives. Holds
  a single `rusqlite::Connection`. Migrations are `include_str!`d from `migrations/` and executed
  on every `Store::open`/`in_memory` (idempotent `CREATE ... IF NOT EXISTS`); there is no version
  table, so schema changes go in a new numbered migration file that is additive and re-runnable.
- `src/model.rs` — the domain vocabulary (`SourceKind`, `ContentKind`, `EdgeKind`, `ReviewState`,
  `AdvocacyKind`, `TimelineLane`). Each enum has `as_str()` returning the **stable database
  representation**, mirrored by a `CHECK(... IN (...))` constraint in the migration. Adding a
  variant means editing both the enum and the SQL constraint.
- `src/ingest.rs` — adapter-neutral input types only (no behavior). `Store::import_normalized`
  is atomic and validates hashes, offsets, confidence range, production ownership, and the rule
  that machine-generated content cannot arrive already verified.
- `src/review.rs` — review vocabulary plus `transition_allowed`, the state-machine predicate.
- `src/views.rs` — serializable read models (`Overview`, `DiscoveryItem`, `ElementRow`,
  `WitnessStatement`, `TimelineEntry`, `IssueWorkspace`, `DecisionBrief`, `PropositionEvidence`,
  `OffenseComparison`). These are the stable contract for the planned Windows-only WinSafe GUI,
  which will consume Rust read models rather than SQL. No cross-platform GUI is planned.
- `src/fixture.rs` — hand-authored `DemoFixture` cases seeded from raw SQL (`fixtures/*.sql` for
  hit-and-run, inline SQL for vehicle-stop). Seeding is transactional and idempotent. The
  fixtures deliberately encode clock disagreement, a superseding report, conflicting witness
  attribution, an interrupted recording, referenced-but-missing evidence, and a charged-versus-
  lesser offense comparison — tests assert those tensions survive collation.

The graph is node tables (`sources`, `source_segments`, `content`, `entities`, `propositions`,
`events`, `charges`/`elements`, `advocacy_items`) joined by one polymorphic `edges` table keyed
`(source_kind, source_id, relation, target_kind, target_id)`. All tables are `STRICT`.

### Invariants that constrain almost every change

`docs/domain-rules.md` is the normative list; the ones that bite most often:

- **Nothing is scored.** Propositions stay contested; supporting and contradicting evidence
  coexist. Do not add truth/confidence aggregation, global admissibility flags, or automatic
  merging of `possibly_same_person` mentions.
- **Machines cannot confer verification.** Import produces only `unreviewed`/`suggested`; only a
  named person reaches `reviewed`/`verified`/`rejected`, and no decision may return a record to
  an intake state.
- **`verified` must cite the original.** Content and sources have one exact locator and the cited
  locator must match verbatim (`Error::LocatorMismatch`); edges, propositions, and events span
  sources, so they require a written `basis` instead. `rejected` always requires a `basis`.
- **`review_events` is append-only**, enforced by SQLite triggers, and read in insertion order
  (not by `decided_at`) so clock skew cannot reorder the trail. `review_state` on a record is a
  cache of the latest decision; the trail is the artifact.
- **Raw time is never overwritten.** `raw_time`, `content_created_at`, `asserted_time`, and the
  normalized interval are separate columns; normalization is a reviewable hypothesis with a
  `time_basis`.
- **Timeline lanes never collapse** into a single authoritative sequence.
- **Advocacy items, annotations, and decision briefs are privileged** and stay out of the
  discovery ledger and any routine export.

Tests are named as assertions about these rules (`a_reviewer_cannot_return_a_record_to_an_intake_state`,
`timeline_keeps_competing_lanes_and_raw_time`). Both integration files build a `Store::in_memory()`
and seed a fixture; keep new tests in that style rather than unit-testing SQL strings.

## Project knowledge (AKR)

`AGENTS.md` is the binding protocol and takes precedence over anything summarized here: durable
project knowledge lives in `.akr/` as typed records, not Markdown. Run `knowledge.context` before
starting a task, `knowledge.validate` before handing work back, record durable changes via
`knowledge.propose`/`revise`/`supersede`/`complete`, and log friction with `knowledge.papercut`.
Never hand-edit `.akr/`, never read `.akr/cache/`, never delete a record, never touch
`docs/generated/`.

Markdown at the root is reference, not truth: `collation-first-plan.md` is the design thesis this
repo implements (collation is the product; OCR/ASR/video are replaceable adapters),
`PLAN.md` is the earlier market/product plan it corrects, and `README.md` states the current
boundary. `lingbot-map/` and `2503.06317v1.md` are vendored external research material unrelated
to the Rust crate.
