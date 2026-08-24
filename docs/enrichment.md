# The enrichment sweep

The adapters produce passages. They do not produce meaning. Nothing in an OCR
page, an ASR utterance or a video segment says whether a sentence is the
author's own assertion, a quotation, a report of what somebody else said, a
measurement, or a legal characterization — and no current local model decides
that reliably enough to be allowed to. So the semantic layer is entered by a
person, and the only question the design can answer is how few keystrokes that
takes.

The sweep is that answer. It is a Windows-only WinSafe workspace over the
platform-neutral [`Workspace`](../src/gui/mod.rs) API, reached from the main
window with `Alt+X` or `Ctrl+E`.

## Three principles

**Decide once, inherit everywhere.** A source profile is one screen per file:
its role, author, creation date and default form, stance and perception basis
are inherited by every passage in the source. A passage's current reading is
`explicit interpretation ?? inherited profile default`, and the inheritance is
visible — the grid's provenance column reads `inherited:<profile id>` rather
than pretending a person entered it. In a police report the exceptions are
exactly the passages that matter.

**Sweep one field across many passages.** Filling a form per passage is the
slow shape. The fast shape is a column sweep: choose a field, walk down with
`j`/`k`, press one key per row. Attention stays on one question and the rhythm
is one keystroke per passage.

**A candidate costs one key to accept and one to refuse.** The deterministic
rules in [`suggestions.md`](suggestions.md) pre-fill `suggested`
interpretations. `Enter` accepts the one under the cursor, `x` refuses it with
the rule's name recorded as the basis, and anything else overrides it. A
candidate nobody has answered renders as `MACHINE SUGGESTION` and never reaches
a packet or a digest.

## The window

```
Source  [Officer Chen report.pdf [document] 3 passages · 3 awaiting a person · 1 candidates]
Role [police_report]  Author [Ofc. Chen]  Created [2024-03-02]  Defaults [authored_assertion]
                                            Sweep field [F2 Form]   3 passages · 1 needs this field
┌────┬────────────────────┬──────────────────────────────┬──────────────────┬──────────────────────┐
│ #  │ Locator            │ Passage                      │ Value            │ Provenance           │
│ 1  │ page 3, paragraph 4│ Rivera gave verbal consent…  │ reported_statem… │ entered              │
│ 2  │ page 4, paragraph 2│ Backup officer body-camera…  │ evidence_refere… │ suggest:evidence-re… │
└────┴────────────────────┴──────────────────────────────┴──────────────────┴──────────────────────┘
The original under the cursor — check derived text against it before verifying
```

The panel under the grid always names the original the selected row cites, its
page or interval, and whether the file still matches the hash recorded at
intake. `Open Original` and `Open Context` hand the file to the system viewer;
when neither is available the locator is still shown, because the locator is
the citation whether or not this machine can open the file. Rule 17 is
unchanged: a `verified` decision needs the original itself, and the panel says
so when it cannot offer one.

## Keystroke grammar

| Key | Meaning |
| --- | --- |
| `j` / `k`, `↓` / `↑` | next / previous passage |
| `J` / `K` | next / previous passage with a candidate or a missing value |
| `F2`…`F8` | form, speaker, attributed person, temporal stance, time, location, perception basis |
| one letter | set this field's value for this passage and advance |
| `Enter` | accept the candidate and advance |
| `x` | refuse the candidate, recording the rule as the basis |
| `.` | repeat the last entered value |
| `Shift`+letter | set the value and extend it to the end of the page |
| `v` then a letter | visual range: mark here, move, apply to the range |
| `s` | split: mark the sub-span the reading is really about |
| `g` | group this passage with the next into one unit |
| `p` | reporting parent = the nearest preceding passage by the same author |
| `@` | entity autocomplete, with create-new and `possibly the same person` |
| `t` | time entry, with a basis that sticks for the rest of the sweep |
| `n` | boilerplate: leave this passage out of later sweeps |
| `Esc` then `u` | undo by superseding the current version with its predecessor |

Every key that changes a value writes one `content_interpretations` row under
the named reviewer. Nothing is overwritten: a correction supersedes, and the
earlier version stays readable.

`@` never merges. Creating a name the case does not hold writes a separate
record, and `Create, Possibly Same as Selected` adds an unreviewed
`possibly_same_person` edge that joins the review queue — the question is
recorded, not resolved.

## Throughput budget

`tests/enrichment.rs` drives the same `Workspace` API the window drives, with
no mouse path available to it, and ratchets two numbers over the fixture:

- at most **1.6 keystrokes per material passage** for form, speaker and stance
  (an inherited value costs zero; a candidate accepted with `Enter` costs one);
- at most **4 keystrokes** per reported statement for its attributed person and
  its reporting parent.

A change that raises either number fails the test and needs a recorded reason.
Pointing at a row with the mouse costs one keystroke in the same accounting —
reaching a passage is work whether the reviewer walked to it or clicked it, and
the budget would flatter itself if pointing were free.

## What the window does not do

It does not render the page image or the waveform inside itself; it names the
retained context and opens it in the system viewer. It does not offer a clock
offset on the profile screen — a source clock correction is entered through the
interpretation batch, where its written basis is enforced. Neither is a
limitation of the kernel: both are frontend work left for a later pass.

## Reaching the views

The Alt namespace is full — twenty-five of the twenty-six letters are claimed —
so every workspace view also carries a `Ctrl` accelerator: `Ctrl+1`…`Ctrl+9`
and `Ctrl+0` for the first ten, `Ctrl+Shift+1` and `Ctrl+Shift+2` for
proposition packets and the case digest, which have no letter left to take.
`every_command_has_its_own_accelerator` keeps both namespaces unambiguous, and
`every_workspace_view_is_on_the_rail` keeps a read model from existing with no
way to reach it.
