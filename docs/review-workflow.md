# Human review workflow

The kernel's central promise is that a machine cannot confer human verification.
Import produces `unreviewed` and `suggested` records; only a named person moves
anything past that line, and every move is recorded.

## States

| State | Produced by | Meaning |
| --- | --- | --- |
| `unreviewed` | import | Entered the case; nobody has looked at it. |
| `suggested` | import from an adapter | A machine proposed it. |
| `reviewed` | a person | Someone read it and let it stand. |
| `verified` | a person | Someone opened the original and confirmed it. |
| `rejected` | a person | Someone found it wrong, and said why. |

`unreviewed` and `suggested` describe what import produced. No decision may
return a record to either: a suggestion that was examined and found wanting is
`rejected`, not un-suggested.

Every other move is permitted, including withdrawing a verification and
reinstating a rejected item. Investigation is continuous, and later discovery
routinely undoes an earlier reading. What a reviewer may not do is repeat a
decision the record already holds; that is refused rather than written twice.

## What each decision costs

`reviewed` asks only for a named reviewer.

`verified` is a claim that a person opened an original, so it must say which
one. Content and sources have a single exact locator, and the decision must
cite it verbatim — `Officer Chen report.pdf @ page 3, paragraph 4`. A cited
locator that does not match the record's own is refused, not recorded with a
warning. Relationships, propositions, and events are attorney judgments drawn
across several sources; they have no one original to open, so verifying one
requires a written basis describing what was compared.

`rejected` always requires a written reason. Rejection removes material from
the working view, and a later reader needs to know on what ground.

## The trail

`review_events` is append-only, enforced by SQLite triggers rather than
convention. The state column on each record is a cache of the latest decision;
the trail is the authoritative artifact, and it is read in insertion order
rather than by timestamp so that two decisions in the same millisecond — or a
corrected system clock — cannot reorder what a reviewer actually did.

A case holding review decisions cannot be deleted by cascade. The audit trail
outlives convenience.

## Commands

```sh
cargo run -- review case-hit-run-001 queue
cargo run -- review case-hit-run-001 apply \
  --target content --id hr-content-911-injury --state verified \
  --actor "A. Reyes" \
  --locator "911 call.wav @ 00:00:08.200–00:00:31.600"
cargo run -- review case-hit-run-001 history --id hr-content-911-injury
```

`queue` lists everything still awaiting a person, machine suggestions first,
each with the exact locator to open. `view … overview` reports the same backlog
as `pending_review`.
