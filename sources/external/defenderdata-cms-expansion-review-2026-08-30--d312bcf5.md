# Outside review: expanding scope to an open-source DefenderData alternative (2026-08-30)

Outside agent advice supplied by the owner on 2026-08-30, verbatim. Non-authoritative
until adopted by ledger records.

---

Yes, but it requires a deliberate change in product scope.

Your current project is already positioned to be **much better than DefenderData at discovery review**, but the attached AKR explicitly says the product is *not* a case-management system and rejects calendar, scheduling, CMS notes, SMS, timekeeping, funder reporting, client demographics, conflict checks, and document assembly. If the goal is now "open-source DefenderData alternative," that decision should be superseded—not muddied with exceptions.

The right design is **not to turn the evidence graph itself into DefenderData**. Keep the evidence-intake kernel conceptually clean, and add an office/case-management layer around it.

### What DefenderData is actually doing for defenders

The official marketing makes DefenderData sound enormous: case management, documents, calendaring, SMS, voucher/payment management, reporting, bulk editing, administration, security, integrations, migration and mobile access. ([DefenderData][1])

But the Reddit thread is much more useful because it shows which pieces become indispensable in practice. Attorneys repeatedly describe essentially the same core workflow:

* find **my active cases / my cases for tomorrow's docket**;
* see the client's cases, charges, contact information, offers and custody state;
* keep **all case notes and discovery together**;
* see the office calendar and who can cover court;
* assign work to investigators/paralegals;
* find another attorney's case when needed;
* perform a quick conflict/name check;
* associate several criminal cases with one client;
* upload documents;
* produce reports and required administrative statistics;
* sometimes log time and communicate by SMS.

One attorney with roughly 150 misdemeanors says searching open cases by court date is how they stay organized; another says their office puts schedules, notes, discovery, Axon links and investigator/paralegal assignments in DefenderData. Others describe conflict checks, linked cases, documents and daily-schedule reports as routine. ([Reddit][2])

That is important because **your software presently starts where DefenderData mostly stops**.

Your project knows:

> What does the evidence actually establish?
> Where did this fact come from?
> Which originals independently support this element?
> Where do accounts contradict?
> What is missing?
> Which proposition rests on an unchecked source?
> What happened across documents, audio and video?

DefenderData is primarily helping answer:

> Who is my client?
> What cases do they have?
> What happens in court tomorrow?
> What did I write last time?
> Who is assigned to do this?
> Where is the file?

Those two systems fit together extremely naturally.

## I would change the architecture to this

Don't expand `src/model.rs` until `Case` gradually means four different things. Introduce a new sibling domain.

| Layer               | Responsibility                                                                 |
| ------------------- | ------------------------------------------------------------------------------ |
| **evidence-core**   | Your current immutable/source-grounded evidence graph                          |
| **office-core**     | Clients, matters, assignments, schedules, notes, tasks, contacts, flags, time  |
| **document-core**   | Pleadings, correspondence, client documents, templates and ordinary case files |
| **evidence-server** | Authentication, permissions, concurrency, sync, auditing, API                  |
| **intake/runtime**  | Your existing OCR/audio/video/TensorRT pipeline                                |
| **clients**         | Desktop + browser/PWA/mobile-capable UI                                        |

That separation matters.

For example, your existing `events` table should **never become court-calendar events**. Those events express competing evidentiary timelines.

Likewise, `advocacy_items(kind = investigation_task)` should not become the office task manager. An investigative issue such as *"determine whether the camera covered the parking lot"* is privileged defense work product. An operational task such as *"Smith — obtain medical records by Tuesday"* has assignment, due date, notifications and supervisory workflow.

And your current `entities.is_client` isn't enough for CMS clients because entities are case-scoped. A CMS must understand:

**Jane Doe → one person → five matters → perhaps three attorneys → one common telephone number → one common client-notes stream → separate matter-specific notes.**

That distinction directly fixes one of the strongest DefenderData complaints: information becoming fragmented when one person has several criminal cases. ([Reddit][2])

## The minimum new schema

I would create roughly these new conceptual records:

| Domain         | Important records                                                           |
| -------------- | --------------------------------------------------------------------------- |
| Office         | `organizations`, `users`, `teams`, `roles`                                  |
| Client         | `clients`, `client_aliases`, `contacts`, `addresses`, `relationships`       |
| Matter         | `matters`, `matter_numbers`, `matter_status`, `matter_links`                |
| Assignment     | `matter_assignments`, `work_assignments`                                    |
| Court          | `courts`, `judges`, `calendar_events`, `deadlines`, `appearances`           |
| Notes          | `case_notes`, `client_notes`, `note_mentions`                               |
| Tasks          | `tasks`, `task_assignees`, `task_dependencies`, `reminders`                 |
| Communications | `communications`, `sms_threads`, `email_links`                              |
| Documents      | `case_documents`, `document_versions`, `templates`                          |
| Administration | `flags`, `custom_fields`, `time_entries`, `custody_periods`                 |
| Reporting      | saved queries/report definitions rather than hundreds of hard-coded reports |
| Security       | `audit_events`, access grants, sealed-case restrictions                     |
| Integration    | external IDs, import provenance, synchronization records                    |

A `matter` would have an optional `evidence_case_id`, giving the CMS access to your existing collation kernel without making either schema depend semantically on the other.

### The most important screen wouldn't be the case screen

I think your killer screen should be **Today / Docket**.

Suppose the attorney walks into court at 8:50. They see:

| Client | Charge        | Setting      | Custody | Offer | Last contact | Evidence posture                             | Open work            |
| ------ | ------------- | ------------ | ------- | ----- | ------------ | -------------------------------------------- | -------------------- |
| Morgan | Leaving scene | 09:00 plea   | Out     | ...   | Aug 27       | 1 disputed element; missing referenced video | Talk to client       |
| Patel  | DUI           | 09:00 prelim | In      | ...   | Aug 29       | Sole-source support on element 3             | Investigator overdue |

Click Morgan and the same workspace expands into:

**Client → Matters → Current matter → Charges → Court → Notes → Discovery → Evidence standing → Timeline → Witnesses → Work product.**

This combines what DefenderData users presently have to retrieve from several places with what your evidence engine already knows.

It would be a substantial practical advantage. The Reddit complaints aren't really about aesthetics; they're about **information architecture**. Users complain that charge information, case information and demographics are unnecessarily separated, that calendar entries are hard to read, and that they cannot see information while writing a note. ([Reddit][2])

Your UI should therefore aggressively follow the opposite rule:

**Show context; edit in place; don't make the user navigate somewhere else to manipulate data already visible on screen.**

Case numbers, telephone numbers, emails, charges and names should all be copyable wherever displayed.

## Calendar deserves much more attention than it sounds like

I'd actually make calendaring one of the first CMS features.

The Reddit thread shows it isn't administrative decoration. For high-caseload defenders, **the calendar is effectively the task-selection mechanism**. Attorneys organize their work around who is appearing next. One commenter calls the calendar arguably the most important mobile feature. ([Reddit][2])

Your model should support one real-world concept that DefenderData apparently handles poorly:

**one court setting → multiple related matters.**

So instead of duplicating:

`Morgan / case A / Sep 17 09:00`
`Morgan / case B / Sep 17 09:00`
`Morgan / case C / Sep 17 09:00`

have a court appearance with links to A+B+C.

The user can explicitly override one matter later.

That eliminates an entire class of duplicate-data errors the Reddit thread complains about. ([Reddit][2])

Likewise, the calendar should support team/office views, coverage, recurring events, external calendar synchronization and conflict detection. Those are things JusticeWorks itself treats as core product functionality. ([DefenderData][3])

## Notes need a better model than DefenderData

You already have excellent provenance principles. Apply some of them here without confusing CMS notes with evidence.

I would have three obvious note contexts:

**Client note** — follows the person across matters.
**Matter note** — belongs to one criminal matter.
**Event note** — belongs to a hearing/meeting/appointment.

Every note has immutable authorship. Editing creates a revision or visible edit record.

That preserves something one DefenderData user explicitly values: other users cannot silently alter their notes. ([Reddit][2])

Tagging/mentions can handle:

`@investigator`
`@socialwork`
`@immigration`
`@supervisor`

But avoid DefenderData's apparent problem of presenting a huge undifferentiated category list. Let offices define defaults/favorites.

## Search and conflicts should become office-wide

Your current case isolation is a strength for evidence.

It becomes a weakness for CMS retrieval.

You need two search scopes:

**Evidence search** remains case-isolated by default.

**Office search** can answer:

> Jordan Patel
> plate ABC123
> Officer Ruiz
> phone 555-1234
> case CR-2026-491
> tomorrow Judge Smith
> all my open felony cases

That also gives you conflict checking almost for free once clients, witnesses, alleged victims, officers and related persons are represented at the office level.

Don't automatically unify people merely because names match. Your current conservative entity philosophy is exactly right. Show:

> Possible existing person: Jordan Patel — client in CR-2025-1234.

A person resolves it.

## Reporting is not optional

This is the part I would have underestimated without the Reddit discussion.

A commenter specifically notes that DefenderData is unusually suitable for criminal/public-defense practice because it supports **reporting for funding rather than billing**, and that many apparently obscure features exist because different offices have mandated reporting requirements. ([Reddit][2])

So if you omit administrative reporting, an individual defender may love your application while the chief defender cannot adopt it.

Don't duplicate DefenderData's fixed-report approach, though. Their own users complain that it can't simply let them select the fields they want and produce, for example:

> Name + case number + charge + next setting for every active case.

([Reddit][2])

Build a query/report designer:

**Rows:** clients / matters / events / time entries / assignments
**Filters:** status=open, attorney=King, date=September
**Columns:** user-selectable
**Group/sort:** arbitrary
**Save:** named report
**Export:** CSV/XLSX/PDF
**Schedule:** optional

That could replace dozens of specialized reports. JusticeWorks currently advertises dataset builders, report writers, saved reports and scheduled reports, so administrator expectations are already at that level. ([DefenderData][4])

## Custom fields should be first-class, but constrained

Public defender offices vary too much for a completely fixed schema.

DefenderData markets extensive customization, and Reddit users mention office-specific flags such as social-work involvement, investigator involvement and immigration status. ([DefenderData][5])

I would support typed office-defined fields:

`boolean`
`enum`
`date`
`number`
`short text`
`person`
`event`

and let them attach to a client, matter, charge or event.

But don't build a no-code database designer where everything can be redefined. That produces schemas nobody can interoperate with.

Instead have a stable common PD schema plus extension fields.

This suggests another good open-source feature: **jurisdiction packs**.

For example:

```text
jurisdictions/
    texas/
        criminal.yaml
        reporting.yaml
        event-types.yaml
        templates/
    california/
        ...
```

A county or state could publish its court types, reporting definitions, statutory elements, document templates and common flags without forking the program.

That is a natural advantage of open source.

## Your evidence system becomes the reason to switch

This is where I would *not* imitate DefenderData.

After importing a discovery production, your system should eventually give the attorney something like:

> **New discovery — 2.8 GB / 37 files**
>
> 14 documents processed
> 3 body-camera recordings processed
> 2 interviews transcribed
>
> **What changed**
>
> New account from Ruiz
> Two statements bear differently on proposition X
> Report references video that was not included
> New material bears on element 3
> Source clock disagrees with bodycam timeline
> Three items remain unreviewed

The attorney can then move directly into the proposition packet, standing view, witness view or original.

That's substantially more useful than:

> `Discovery / Disc 6 / bodycam3.mp4`

Your current architecture—immutable originals, exact locators, source lineage, human review states, separated privileged analysis, statutory elements, deterministic tension/gap detection—is precisely the part I would preserve.

It gives the product an actual thesis instead of becoming an open-source clone.

## Multi-user/security is probably the largest engineering jump

Right now this is a Rust + SQLite local-first program with a WinSafe client.

For a replacement CMS, you need office-wide simultaneous use.

I would not attempt to put the existing SQLite database on a network share.

Instead:

```text
Desktop / Browser / Mobile
          │
          ▼
   evidence-server
          │
     ┌────┴─────┐
     │          │
 PostgreSQL   File/Object
 metadata      storage
     │
 evidence/intake workers
```

For a one-attorney installation, an embedded/single-machine mode can remain.

For an office, use PostgreSQL or another real client/server DB.

The service needs:

MFA, sessions, RBAC, TLS, audit logs, sealed cases, backup/restore, retention policies, and granular access to privileged information. DefenderData explicitly advertises MFA, role-based permissions, encryption, sealing/locking cases, audits, NIST alignment and CJIS-oriented deployment. ([DefenderData][6])

You don't need to claim "CJIS compliant software"—CJIS compliance also depends on the deployment and organization—but you do need to make **a compliant deployment possible and documented**.

Your AGPL-3.0 license is already sensible for the open-source/server model.

## Document storage needs a careful middle ground

I agree with the existing AKR decision not to recreate Axon Evidence.

But a DefenderData replacement does need ordinary document management.

Keep three separate concepts:

**Discovery original**

Immutable evidence your existing pipeline processes.

**Case document**

Motions, court orders, letters, client-provided PDFs, correspondence.

**Generated document**

A file instantiated from a template.

A 60 GB bodycam export doesn't have to live *inside* PostgreSQL. It can reside in configured office storage with hash/integrity metadata in the database.

That preserves your current provenance model without trying to become a full digital-evidence-management platform.

JusticeWorks does advertise centralized storage for ordinary documents and very large evidence, so at least managed/linkable file access is table stakes for complete replacement. ([DefenderData][5])

## Mobile should probably mean PWA/browser first

I would not build separate Android and iOS applications initially.

The Reddit thread gives you a nearly perfect mobile specification:

> At court, show my complete calendar well beyond two weeks; let me see client names, find colleagues' clients, read the case, write notes and get documents.

Those are the exact areas where users complain about DefenderData's mobile implementation. ([Reddit][2])

A responsive browser UI/PWA backed by `evidence-server` gets you Windows/Linux/macOS/tablet/phone simultaneously.

Your WinSafe interface can remain the heavyweight evidence-review workstation where local GPU processing and side-by-side source review matter.

That's actually a good split:

**Browser:** case management / court / docket / mobile.

**Desktop:** intensive discovery review.

Both call the same application/service layer.

## What I would make the roadmap

I wouldn't immediately implement every DefenderData feature. I'd change the master plan to four milestones:

| Milestone                              | Outcome                                                                                            |
| -------------------------------------- | -------------------------------------------------------------------------------------------------- |
| **CMS-0: Office foundation**           | Server, users, RBAC, client/matter model, linked cases, assignments, notes, global search          |
| **CMS-1: Defender daily workflow**     | Calendar/docket, deadlines, tasks, custody/offers, document repository, court-day view             |
| **CMS-2: Office adoption**             | Reporting builder, custom fields/flags, timekeeping, bulk editing, imports/migration, audit/backup |
| **CMS-3: Communications & automation** | SMS, reminders, templates/autodocs, integrations, jurisdiction packs                               |

The existing evidence roadmap should continue essentially parallel to this rather than being subsumed by it.

And I would make one milestone gate particularly unforgiving:

> **An attorney with 150 active misdemeanors must be able to determine what they need to do tomorrow without opening another program.**

That seems to be the real test DefenderData passes today. ([Reddit][2])

Then a second:

> **An administrator must be able to satisfy required funding/caseload reporting without maintaining a parallel spreadsheet.**

Without the second one, offices won't fully migrate.

## Things I would explicitly *not* add yet

Don't build billing/invoicing, trust accounting, payment processing, e-filing, legal research, video conferencing, generic CRM, Axon API integration or elaborate AI drafting simply because other legal systems contain them.

DefenderData's advantage isn't that it is Clio plus everything else. A Reddit user makes essentially the opposite point: its strength is being unusually complete for **criminal/public-defense workflows**. ([Reddit][2])

So the product thesis could become:

> **A public-defender case system that understands the evidence, not merely where the evidence file is stored.**

That is substantially stronger than the current "evidence intake utility" position while still avoiding generic practice-management bloat.

### Where I think the project stands

Relative to a DefenderData replacement, you probably have only perhaps **25–35% of the total product surface** today. But you've built much of the 25–35% that would be hardest for another CMS to replicate: source provenance, multimodal discovery intake, evidentiary relationships, human review semantics, charge-element analysis, search and the groundwork for proper collation.

Most of the remaining work is much less research-heavy but broad: identity, clients, scheduling, notes, office collaboration, security, reporting, networking and deployment.

That's a good position to be in.

The part I would change from the current AKR most strongly is therefore **not** "DefenderData was the wrong comparison; keep this purely discovery-oriented." The evidence project has now matured enough that a thin, well-designed PD-specific CMS wrapped around it makes sense. The evidence kernel should remain opinionated and narrow internally, while the application above it becomes the thing an attorney can actually live in all day.

If you make that change, the strongest competitor is not "free DefenderData." It is **DefenderData rebuilt around the evidence itself**, with the specific UX failures its current users are already telling you how to avoid.

[1]: https://www.justiceworks.com/defender-data "defenderDATA™ | Public Defender Case Management Software"
[2]: https://www.reddit.com/r/publicdefenders/comments/1u7ip43/how_do_you_use_defenderdata/ "How do *you* use DefenderData? : r/publicdefenders"
[3]: https://www.justiceworks.com/legal-data?utm_source=chatgpt.com "legalDATA™ | Flexible Legal Case Management Software"
[4]: https://www.justiceworks.com/reporting-business-intelligence?utm_source=chatgpt.com "Reporting & Business Intelligence"
[5]: https://www.justiceworks.com/?utm_source=chatgpt.com "Justice Works | Case Management Software for Public Defenders, Prosecutors & Legal Teams"
[6]: https://www.justiceworks.com/advanced-security-data-protection?utm_source=chatgpt.com "Advanced Security & Data Protection"
