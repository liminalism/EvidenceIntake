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
  a single `rusqlite::Connection`. Migrations are `include_str!`d from `migrations/` and run by
  `Store::migrate` (idempotent `CREATE ... IF NOT EXISTS`); schema changes still go in a new
  numbered migration file that is additive and re-runnable. **A new migration must bump
  `SCHEMA_VERSION` in `src/store.rs`** — it is compared against `PRAGMA user_version` so an
  already-current database skips the DDL on open, and a database at any lower version (including
  zero, which is every database written before the stamp) re-runs all of them. Forgetting the bump
  means the migration never reaches an existing database.
  SQLite has no re-runnable `ALTER TABLE ADD COLUMN` and cannot retrofit `NOT NULL`, so a
  retrofitted column goes through `Store::add_column_if_missing` and its guarantee is enforced
  forward by a trigger; prefer a new table when the choice exists.
  Queries go through `prepare_cached` (or the `query_one`/`exists` helpers), never `prepare`:
  the cache is what keeps a read model from recompiling the same SQL once per row.
- `src/model.rs` — the domain vocabulary (`SourceKind`, `ContentKind`, `EdgeKind`, `ReviewState`,
  `AdvocacyKind`, `TimelineLane`). Each enum has `as_str()` returning the **stable database
  representation**, mirrored by a `CHECK(... IN (...))` constraint in the migration. Adding a
  variant means editing both the enum and the SQL constraint.
- `src/ingest.rs` — adapter-neutral input types only (no behavior). `Store::import_normalized`
  is atomic and validates hashes, offsets, confidence range, production ownership, and the rule
  that machine-generated content cannot arrive already verified.
- `src/authoring.rs` — human authoring input and result types only (no behavior), the
  counterpart to `ingest.rs`. `Store::author_proposition`, `link_evidence`, `record_charge`,
  `map_element`, `author_advocacy_item`/`revise_advocacy_item`, `annotate`/`revise_annotation`,
  and `record_brief` enforce the rules: authored records enter `unreviewed` and `contested`,
  every link carries a written rationale, every mapping and every work-product version names
  its author, a charge is written with its elements or not at all, both endpoints must exist
  inside the case, the same claim is never asserted twice, and work product is revised by
  superseding rather than overwriting.
- `src/review.rs` — review vocabulary plus `transition_allowed`, the state-machine predicate.
- `src/suggest.rs` — assisted-collation vocabulary (`SuggestionKind`, `SuggestionRun`). Analyzers
  are deterministic SQL in `Store::suggest`/`candidates`/`findings` — no model, no score, no
  tolerance windows. An analyzer either *proposes* (writes a `suggested` edge with
  `created_by = "suggest:<analyzer>@<version>"`, which is how `review_queue` knows an edge is
  machine-generated) or *reports findings* (derived every run, stored nowhere, dismissed only
  by closing the gap) — never both; `SuggestionKind::proposes_relationships` decides which.
  A new analyzer is one variant plus one query returning `(from, to, rationale)` ordered by
  identifier, or `(subject_id, subject, summary)` for a finding.
- Search is SQLite FTS5 (`migrations/0007_search.sql`), an external-content table over
  `content.text` kept current by triggers, so the index cannot disagree with the record. Only
  `content` is indexed; privileged tables are excluded structurally, as in `export_case`.
- `src/export.rs` — audience-aware export read models (`ExportAudience`, `CaseExport`). Every
  factual line resolves to an exact locator or the proposition is reported as unsupported;
  omissions and unreviewed inclusions are counted in the header rather than left implicit.
- `src/views.rs` — serializable read models (`Overview`, `CaseStanding`, `DiscoveryItem`,
  `ElementRow`, `WitnessStatement`, `TimelineEntry`, `IssueWorkspace`, `DecisionBrief`,
  `PropositionEvidence`, `OffenseComparison`). `CaseStanding` (`Store::case_standing`) is the
  decision view: per element, direction counts, distinct originals behind the support, the sole
  source when there is one, unchecked support, unbacked mappings; plus load-bearing sources, live
  disputes, and analyzer gaps ordered by whether they touch a charge. Rule 35 governs it — report
  structure, never a verdict — and `nothing_in_the_standing_view_scores_the_case` enforces it by
  serializing the view and failing on scoring vocabulary. `SearchHit` carries the exact locator
  and what the case has already tied the passage to. These read models are the stable contract for
  the planned Windows-only WinSafe GUI, which will consume Rust read models rather than SQL. No
  cross-platform GUI is planned.
- `src/fixture.rs` — hand-authored `DemoFixture` cases seeded from raw SQL (`fixtures/*.sql` for
  hit-and-run, inline SQL for vehicle-stop). Seeding is transactional and idempotent. The
  fixtures deliberately encode clock disagreement, a superseding report, conflicting witness
  attribution, an interrupted recording, referenced-but-missing evidence, and a charged-versus-
  lesser offense comparison — tests assert those tensions survive collation.

The graph is node tables (`sources`, `source_segments`, `content`, `entities`, `propositions`,
`events`, `charges`/`elements`, `advocacy_items`) joined by one polymorphic `edges` table keyed
`(source_kind, source_id, relation, target_kind, target_id)`. All tables are `STRICT`.

### The office layer (`office-core`, `src/office.rs`, the Court pane)

A sibling of the kernel, never a layer inside it. `office-core/` is a separate workspace member
with its own `office.sqlite` beside `evidence.sqlite`: `users`, `clients`, `matters`, `courts`,
`appearances`, `deadlines`, `notes`, `client_evidence_links`, and an FTS5 office search over a
`search_documents` shadow table. It repeats the kernel's store discipline exactly — `STRICT`
tables, numbered idempotent migrations behind a `SCHEMA_VERSION`/`PRAGMA user_version` stamp,
`prepare_cached` on every query, enums whose `as_str` is mirrored by a `CHECK(... IN (...))`.
Dates are TEXT `YYYY-MM-DD` with a hand-rolled `civil_date` module (no date crate); business
dates are local, audit timestamps UTC.

- **`office-core` must never depend on `evidence-intake`.** That dependency edge is what makes
  the privileged boundary structural: a docket row is built by code that cannot open an evidence
  database, so `advocacy_items`, `annotations`, and `decision_briefs` are unreachable rather than
  filtered. `the_office_layer_cannot_reach_privileged_kernel_material` asserts it from above.
- `src/office.rs` — `OfficeDesk`, the only module in the workspace holding both databases open.
  It folds `CaseStanding` into `EvidencePosture` (counts and names, never a score — rule 35) and
  builds `CourtDocket`. A matter with no linked case reports `None`; a matter naming a case the
  kernel does not hold is a third state, `matters_with_a_missing_case`, and none of the three may
  be collapsed into the others.
- `src/gui/mod.rs` — `Pane { Court, Office }`, `DocketGridRow`, `DeadlineGridRow`, and the date
  navigation. `Workspace` owns both halves. The Court pane is testable without a frontend.
  Office data entry is typed, never JSON: `ClientDraft`/`MatterDraft`/`SettingDraft`/
  `DeadlineDraft`/`NoteDraft` plus `EvidenceLink { OpenNewCase, Existing, None }` feed
  `create_client`, `open_matter` (which opens the kernel case prefilled from the matter and
  links it — the one combined-write flow), `schedule_setting`, `record_office_deadline`, and
  `write_office_note`; every write is attributed to the session's `ActingUser`, set once via
  `set_acting_user` (`user_named` underneath), and `client_duplicates` is the advisory
  conservative-identity check the client form asks before writing.
- `src/gui/winsafe.rs` — the pane switch is **two buttons plus `ShowWindow` over two control
  groups**, not a `gui::Tab`: a tab page is repositioned only on `TCN::SELCHANGE` and fixing that
  needs an unsafe `SendMessage(tcm::AdjustRect)`, which `unsafe_code = "forbid"` blocks. Every Alt
  letter is claimed, so the pane switch, the Court commands, and the Office entry row carry
  `Ctrl+Shift` chords only (`OFFICE_COMMANDS`: Acting As `A`, New Client `L`, New Matter `M`,
  New Setting `H`, New Deadline `D`, New Note `J`, id block `0x0260`);
  `every_command_has_its_own_accelerator` and `every_chord_reaches_its_own_virtual_key` keep the
  namespaces unambiguous, and a new Office control must be listed in `office_windows()` or it
  bleeds through the Court pane. Entry dialogs are keyboard-first: tab order is creation order,
  vocabulary combos quick-select by first letter, and the generalized `enter_submits` accepts
  Enter from edits, combos, and list boxes. `paint_chrome` skips invisible controls.
- CLI: `evidence office <subcommand>`, with a global `--office-database` defaulting to
  `office.sqlite` beside `--database`. Office commands are dispatched **before** `Store::open`, so
  an office with no discovery yet still has a docket.

Office-layer invariants are domain rules 39–44: kernel stays clean; two databases and no foreign
key; privileged material unreachable rather than filtered; identity across the boundary is a named
person's decision; notes are append-only with immutable authorship; operational time is not
evidentiary time.

### Invariants that constrain almost every change

`docs/domain-rules.md` is the normative list; the ones that bite most often:

- **Nothing is scored.** Propositions stay contested; supporting and contradicting evidence
  coexist. Do not add truth/confidence aggregation, global admissibility flags, or automatic
  merging of `possibly_same_person` mentions.
- **Machines cannot confer verification.** Import and the analyzers produce only
  `unreviewed`/`suggested`; only a named person reaches `reviewed`/`verified`/`rejected`, and no
  decision may return a record to an intake state. An analyzer may not merge, score, alter a
  record a person wrote, or re-propose something a reviewer rejected — and the "already held"
  check is direction-blind, since an analyzer points at a pair, not an orientation.
- **Authoring is not review.** A proposition or link a person writes enters `unreviewed` and
  waits in the same queue; authoring can never produce a reviewed state, and an authored
  proposition is always `contested`. Every link carries a written rationale.
- **Case boundaries hold in the views, not just the writes.** `element_links` has no case
  column, so `element_matrix` and `offense_comparison` constrain `propositions.case_id`
  themselves. Any new query joining through a table without a case column must do the same.
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
  discovery ledger and any routine export. They carry no review state — review asks whether
  an extraction represents an original, and attorney analysis is not an extraction.
  `export_case(_, Disclosable)` excludes them *structurally* — it never queries those tables —
  rather than filtering the `privileged` flag; keep it that way, and keep
  `a_disclosable_export_carries_no_privileged_material` passing.
- **Work product is versioned by superseding, never overwritten.** Only the current version
  may be revised; every view must filter superseded rows (`NOT EXISTS (... supersedes ...)`)
  or a rewritten issue appears twice.

Tests are named as assertions about these rules (`a_reviewer_cannot_return_a_record_to_an_intake_state`,
`timeline_keeps_competing_lanes_and_raw_time`). Every integration file builds a
`Store::in_memory()` and seeds a fixture; keep new tests in that style rather than
unit-testing SQL strings. The one exception is the `schema` module inside `store.rs`, which
asserts what the *migrations* enforce (triggers, unique indexes) and therefore needs the
connection — behavior still belongs in `tests/`.

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
