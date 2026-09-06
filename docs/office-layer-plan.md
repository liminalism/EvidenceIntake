# Office layer plan: a public-defender case system around the evidence kernel

Status: adopted direction, planning detail. The governing record is
`@evidence-intake-project.decision.office-layer-around-the-kernel`, which supersedes
`@evidence-intake-project.decision.not-a-case-management-system`. The first milestone is
`@evidence-intake-project.office.court-office-foundation`. The outside review this plan
responds to is registered as source `defenderdata-cms-expansion-review-2026-08-30`
(non-authoritative; what the project adopted from it is what the ledger records say).

## Thesis

> A public-defender case system that understands the evidence, not merely where the
> evidence file is stored.

The project stops positioning itself as an evidence-intake utility and becomes an
open-source alternative to DefenderData — but not by cloning it. The evidence kernel
already holds the part of the product no CMS can cheaply replicate: immutable originals,
exact locators, source lineage, human review states, privileged-analysis separation,
charge-element analysis, deterministic tension and gap detection. The missing surface is
broad but shallower: clients, matters, calendar, notes, assignments, office search,
reporting, multi-user. That surface is built **around** the kernel, never inside it.

## The boundary that makes the expansion safe

The superseded decision's rationale survives as the *inner* boundary instead of the
product boundary:

| Kernel concept | Office concept | Why they never merge |
| --- | --- | --- |
| `events` (evidentiary timelines) | `appearances`, `deadlines` | Kernel events express competing accounts of what happened; calendar events are operational scheduling. |
| `advocacy_items(investigation_task)` | `tasks` | An investigative issue is privileged work product; an operational task has assignee, due date and supervision. An office task may reference an advocacy item by id, but privileged content stays in the evidence database. |
| `entities` (case-scoped mentions) | `clients` (one person, many matters) | Entities are conservative per-case mentions; a client record carries cross-matter contacts, aliases and a client-notes stream. |
| case (`case_id` in the kernel) | `matter` | A matter is the office's unit: court numbers, status, custody, offers, assignments — with an *optional* `evidence_case_id` into the kernel. |

Hard rules, restated from the decision's claims:

- **Kernel stays clean** — no office concern enters the evidence schema; the kernel's
  invariants (nothing scored, machines confer nothing, append-only review trail,
  privileged separation) are untouched.
- **Sibling domain** — office-core is a separate crate and a separate database
  (`office.sqlite` beside `evidence.sqlite`); no cross-database foreign key; the
  `evidence_case_id` boundary is enforced in the application layer.
- **Conservative identity** — a client is linked to an evidence entity only by a named
  person resolving a "possible existing person" prompt. Never by name match.
- **Privileged boundary** — advocacy items, annotations and decision briefs are
  structurally unreachable from the office database, its views, and its exports —
  the same structural (not filtered) exclusion `export_case` already practices.
- **File-level interop** — unchanged: ingest exported folders; never authenticate to
  Axon, a prosecutor portal, or any platform.

## Architecture

```
clients:      WinSafe desktop (Court | Office panes)      … later: browser/PWA
                         │
application:  read models + write APIs (Rust, platform-neutral)
                         │
        ┌────────────────┼────────────────────┐
   evidence-core     office-core         document-core (later)
   evidence.sqlite   office.sqlite       case documents / templates
        │
   intake/runtime: adapters (document, audio, video) + TensorRT broker
```

- **evidence-core** — the current `evidence_intake` crate, unchanged in semantics.
- **office-core** — new workspace member. Same store discipline as the kernel: STRICT
  tables, numbered idempotent migrations with a version stamp, `prepare_cached`,
  append-only where authorship matters. Written for PostgreSQL portability (avoid
  SQLite-only constructs where a portable form is cheap) because CMS-3 puts this schema
  behind `evidence-server`; ships local-first on SQLite.
- **document-core** (later) — ordinary case documents (motions, orders, letters,
  client PDFs) and generated documents from templates. Three concepts stay distinct:
  *discovery original* (immutable, kernel), *case document*, *generated document*.
  Large media lives in configured office storage with hash/integrity metadata in the
  database, not inside it.
- **evidence-server** (later, CMS-3) — auth, sessions, RBAC, TLS, audit, sync;
  PostgreSQL for office metadata. Single-machine embedded mode remains for the solo
  defender. AGPL-3.0 (already the license) fits the open-core/server model.

## The GUI: two panes, calendar first

Top-level split, owner's design:

**Court** — the daily driver; opens first.
- *Today / Docket* table: client, charge, setting time and type, custody, offer, last
  contact, **evidence posture**, open work. Evidence posture is joined from the kernel's
  `CaseStanding` for the linked evidence case — disputed elements, sole-source support,
  missing referenced evidence, unreviewed count — reported as structure, never a score
  (Rule 35 applies here exactly as it does in the standing view).
- Deadline list and day/week navigation.
- One court setting links all related matters of a client (`appearance_matters`) and
  renders **one row**; a matter can be individually unlinked. This kills the
  duplicate-setting class of errors DefenderData users complain about.
- Selecting a row expands in place: client → matters → charges → notes → evidence
  standing → timeline → witnesses.

**Office** — the working surface for paralegals and PD review.
- Client and matter management: aliases, contacts, related matters, assignments,
  custody/offer state.
- The existing evidence surfaces — intake orchestration (document/audio/video, TensorRT
  sweep), review queue, enrichment workspace, collation views — rehosted here and
  reached through the matter rather than a bare case list.
- Notes and office search (below).
- **The entry row** (landed with the Foundation follow-up): Court reads, Office writes.
  Six keyboard-first dialogs on `Ctrl+Shift` chords — Acting As (`A`), New Client (`L`),
  New Matter (`M`), New Setting (`H`), New Deadline (`D`), New Note (`J`). The person
  entering records is asked once a session and every write is attributed to them. The
  client form runs the possible-duplicate check and asks before writing; the matter form
  opens the evidence case prefilled from the matter's own fields (or links an existing
  one), so nothing is typed twice; the setting form multi-selects matters so the one-row
  rule is enterable, not just readable. Tab moves, first letters pick vocabulary values,
  Enter records.

Two UI rules govern both panes, taken directly from documented DefenderData complaints:

1. **Show context; edit in place.** Never make the user navigate away from data already
   on screen to change it, and never hide the record while a note is being written.
2. **Everything identifying is copyable** — case numbers, phones, emails, charges, names.

## Notes

Three scopes: client note (follows the person), matter note, appearance note.
Authorship and creation time are immutable; edits write a superseding revision and all
views filter superseded rows — the kernel's work-product versioning rule applied to
office data. No user can silently alter another author's note. Mentions
(`@investigator`, `@socialwork`, `@immigration`, `@supervisor`) ride on notes;
office-defined tag defaults come with CMS-2.

## Search and conflicts

Two scopes, deliberately different:

- **Evidence search** stays case-isolated (existing FTS5 over `content` only).
- **Office search** spans clients, matters, contacts, and numbers: a name across
  matters, a phone, `CR-2026-491`, a colleague's client, open matters by next setting.

Conflict checking falls out of office-level representation of clients, witnesses,
officers and related persons: on intake, show *possible existing person* candidates by
name/contact overlap; a person resolves or dismisses each. No automatic merge, ever.

## Roadmap

| Milestone | AKR | Outcome |
| --- | --- | --- |
| **Foundation (CMS-0 + calendar core)** | `@evidence-intake-project.office.court-office-foundation` | office-core crate and schema v1 (users, clients, matters, assignments, evidence-case link); courts/appearances/deadlines and `DocketDay`; Court/Office GUI split; notes; office search + conflict prompts. Local-first, single-machine. |
| **Daily workflow (rest of CMS-1)** | proposed when foundation nears completion | tasks with assignees and due dates, reminders, document repository (document-core), richer court-day view, "what changed in this production" digest surfaced in Office. |
| **Office adoption (CMS-2)** | later | query/report designer (rows × filters × columns × save × export — not hundreds of fixed reports), typed custom fields and flags on client/matter/charge/event, timekeeping, bulk editing, imports/migration, audit and backup. |
| **Collaboration & automation (CMS-3)** | later | evidence-server (PostgreSQL, MFA, RBAC, TLS, audit, sealed cases), browser/PWA client (calendar + notes + documents at court; desktop stays the heavyweight review workstation), SMS/reminders, templates/autodocs, jurisdiction packs (`jurisdictions/<state>/…` YAML: court types, reporting definitions, statutory elements, templates, flags). |

Foundation's work items:
`office-core-crate` → `calendar-and-docket` + `notes-and-office-search` →
`court-office-gui` (Windows sessions; everything below the GUI builds on Linux).

The evidence roadmap — `@evidence-intake-project.assembly.case-assembly` and
`@evidence-intake-project.inference.universal-tensorrt-integration` — continues in
parallel and is the reason to switch, not a subordinate of the CMS.

### Gates

1. **Docket gate** (foundation, `#docket-first`): an attorney with 150 active
   misdemeanors determines what they must do tomorrow without opening another program.
2. **Reporting gate** (CMS-2): an administrator satisfies required funding/caseload
   reporting without a parallel spreadsheet. Without this, individual attorneys can love
   the tool and the chief defender still cannot adopt it.

## Explicitly not yet

Billing/invoicing, trust accounting, payment processing, e-filing, legal research,
video conferencing, generic CRM, platform API integration (Axon included), AI drafting.
DefenderData's strength is being unusually complete for public-defense workflows, not
being Clio-plus-everything; the same discipline applies here.

## Open questions this plan touches

- `@evidence-intake-project.distribution.open-source-for-public-defenders` — the
  DefenderData-alternative positioning presumes open source, but access, sustainability,
  governance, security review, model-weight redistribution and paid-support coexistence
  are still that record's to decide. It gains urgency; it is not decided here.
- PostgreSQL timing — the office schema is written portable from day one; the actual
  server split is CMS-3 and deserves its own decision record when it starts.
- CJIS posture — never claim "CJIS compliant software"; make a compliant deployment
  possible and documented when evidence-server exists.
