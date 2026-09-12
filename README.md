# Evidence Intake

Evidence Intake is a program for public defenders.
The program keeps a case file on your computer.
It does not send case data to a network.

The program helps you put evidence in order.
It keeps different accounts of the same event.
It does not decide which account is true.

A person does the legal work.
The program records the work and shows how the material is connected.

## What the program does

The program stores originals and the text that came from them.
Each piece of text has an exact locator in the original.
A locator is a page, a paragraph, or a time in a recording.

You can:

- Open a case and record each production of discovery
- Keep statements, observations, document text, and gaps in recordings
- Keep people, objects, and places as separate records
- Write a proposition that stays contested
- Link evidence to a proposition and give a written reason
- Record a charge with its statutory elements
- Map a proposition to an element as support, opposition, uncertain, or excluded
- Search the extracted text and get the locator of each hit
- Export a source-linked case file
- Run analyzers that propose a relationship or report a gap
- Review machine output and human writing in one queue.

The program also keeps a separate office file.
The office file holds clients, matters, courts, settings, deadlines, and notes.

On Windows, a graphical program shows the same case file.
The Court pane shows the day in court.
The Office pane shows the evidence work for a matter.

The program includes two demonstration cases:

- A vehicle stop
- A hit-and-run.

These cases contain clock disagreement, a later report that replaces an earlier one, and witness accounts that do not agree.
They also contain an interrupted recording, missing evidence that a source refers to, and a charged offense next to a lesser offense.

## What the program does not do

The program does not conclude the case.
It does not tell you the answer.

| The program does not | Why |
| --- | --- |
| Calculate a truth score or a confidence score for a proposition | Supporting evidence and evidence that contradicts it can exist together. A number hides that fact. |
| Mark evidence as admissible or not for the whole case | Admissibility depends on purpose, foundation, and a ruling. That work belongs to the defender. |
| Merge two person records because the names look the same | Two people can share a name. A named person must decide. |
| Replace a device time or a spoken time with a normalized time | Raw time is the original. A normalized time is a hypothesis that a person can review. |
| Collapse timeline lanes into one official sequence | A recording, a witness, a police report, and a client account can disagree. The program keeps each lane. |
| Let a machine mark a record as reviewed or verified | Only a named person can do that. Machine output starts as `suggested`. |
| Treat a proposition that you write as already checked | Authoring is not review. Written work enters `unreviewed` and waits in the same queue. |
| Show privileged work in a disclosable export | Attorney analysis is not discovery. The export does not read those tables. |
| Do OCR, speech recognition, or video analysis in the evidence store | Those steps are separate adapter programs. The store receives their output. |
| Connect to Axon, a prosecutor portal, or another discovery platform | The program reads files that you already have. It does not log in to a platform. |
| Do billing, e-filing, legal research, or AI drafting | Those tasks are out of scope. |

The standing view reports structure, not a verdict.
It can show that one source carries an element, that nobody opened the original, or that a gap touches a charge.
It does not say that an element is weak, and it does not estimate an outcome.

## Parts of the program

The repository has these parts:

- `evidence` — the command program for the evidence file and the office file
- `evidence-gui` — the Windows graphical program
- `office-core` — the office library (clients, matters, calendar, notes)
- `evidence-document`, `evidence-audio`, `evidence-video` — optional adapters
- `evidence-trt-broker` — optional local inference broker for the adapters.

The evidence file is `evidence.sqlite` by default.
The office file is `office.sqlite` beside it.
The two files do not share a transaction.
The office library cannot open the evidence file.
This boundary keeps attorney work product out of the office views.

## What you need

You need:

- [Rust](https://rustup.rs) (the project file selects nightly)
- A C compiler, because the build compiles SQLite
- Linux or Windows for the command program
- Windows for the graphical program
- Visual Studio Build Tools with C++ on Windows.

You do not need a system SQLite package.
You do not need a GPU to build or operate the command program.
A GPU and a TensorRT package are necessary only when you run the adapters that do OCR, speech, or video analysis.

## How to build the command program

Do these steps:

1. Install `rustup` from <https://rustup.rs>.
2. Open a terminal in the project directory.
3. Build the daily test binary.

```sh
cargo build --profile debug-release --bin evidence
```

The binary is `target/debug-release/evidence`.
On Windows the name is `evidence.exe`.

4. Build the release binary when you want the smaller program.

```sh
cargo build --profile release-final --bin evidence
```

The binary is `target/release-final/evidence`.

`debug-release` is the daily build.
It uses release speed and keeps debug data.
`release-final` is the ship build.
It uses link-time optimization and strips debug data.
The compile time is longer.

You can also start a command without a build first:

```sh
cargo run --bin evidence -- init
```

Always name the binary with `--bin evidence`.
The repository has more than one binary.

## How to build the Windows program

Do this build on Windows:

```powershell
cargo build --profile debug-release --features gui-winsafe --bin evidence-gui
```

The binary is `target/debug-release/evidence-gui.exe`.

For the ship build:

```powershell
cargo build --profile release-final --features gui-winsafe --bin evidence-gui
```

Start the program with the evidence file as the first argument:

```powershell
.\target\debug-release\evidence-gui.exe evidence.sqlite
```

If you omit the argument, the program opens `evidence.db`.
The command program opens `evidence.sqlite` by default.
Give the same path to both programs.

This feature compiles on Linux, but the Linux binary only prints a message.
There is no Linux graphical program.

## How to build the adapters (optional)

The adapters are separate programs.
You do not need them to open a demo case or to write review decisions.

```sh
cargo build --profile debug-release -p evidence-document --bin evidence-document
cargo build --profile debug-release -p evidence-audio --bin evidence-audio
cargo build --profile debug-release -p evidence-video --bin evidence-video
cargo build --profile debug-release -p evidence-trt --bin evidence-trt-broker
```

On Windows, a packaged TensorRT runtime and model packs are also necessary.
See `runtime/tensorrt/README.md` and `scripts/package_windows_tensorrt.ps1`.

An adapter writes a `NormalizedBatch` JSON document.
The command program imports that document.
The evidence store does not run the model.

## How to start with a demo case

The command program prints JSON.

1. Create or migrate the evidence file.
2. Load a demonstration case.
3. Open the standing view.

```sh
./target/debug-release/evidence init
./target/debug-release/evidence seed hit-and-run
./target/debug-release/evidence view case-hit-run-001 standing
```

For the vehicle-stop case:

```sh
./target/debug-release/evidence seed vehicle-stop
./target/debug-release/evidence view case-vehicle-stop-001 standing
```

`seed` is safe to run more than one time.
Open `standing` first.
That view shows what each charge element rests on.

To use a different evidence file, put `--database PATH` before the subcommand:

```sh
./target/debug-release/evidence --database /path/to/case.sqlite init
```

Git ignores SQLite files.

## Daily commands

Show all commands:

```sh
./target/debug-release/evidence --help
./target/debug-release/evidence view --help
```

### Cases and productions

```sh
./target/debug-release/evidence new-case --name "State v. Hall" --reference PD-2026-0900
./target/debug-release/evidence new-production CASE_ID --label "Brady disk 1" --from Prosecution
./target/debug-release/evidence cases
./target/debug-release/evidence productions CASE_ID
```

### Views

```sh
./target/debug-release/evidence view CASE_ID overview
./target/debug-release/evidence view CASE_ID standing
./target/debug-release/evidence view CASE_ID discovery
./target/debug-release/evidence view CASE_ID elements
./target/debug-release/evidence view CASE_ID timeline
./target/debug-release/evidence view CASE_ID collation
./target/debug-release/evidence view CASE_ID issues
./target/debug-release/evidence view CASE_ID offenses
```

Other views take an extra identifier (`witness`, `proposition`, `brief`, `notes`, `work-history`).
See [`docs/standing.md`](docs/standing.md).

### Search, suggestions, review, and export

```sh
./target/debug-release/evidence search CASE_ID 'hatchback'
./target/debug-release/evidence suggest CASE_ID
./target/debug-release/evidence review CASE_ID queue
./target/debug-release/evidence review CASE_ID apply --target content --id CONTENT_ID --state verified --actor "A. Reyes" --locator "LOCATOR"
./target/debug-release/evidence export CASE_ID
./target/debug-release/evidence export CASE_ID --audience work-file
```

`export` without `--audience` is the disclosable file.
That file never reads privileged tables.
`--audience work-file` is the defense team's complete file.
See [`docs/export.md`](docs/export.md), [`docs/search.md`](docs/search.md), [`docs/review-workflow.md`](docs/review-workflow.md), and [`docs/suggestions.md`](docs/suggestions.md).

### Authoring

```sh
./target/debug-release/evidence author CASE_ID proposition --text "Morgan did not perceive the impact." --author "A. Reyes"
./target/debug-release/evidence author CASE_ID link \
  --from-kind content --from CONTENT_ID --relation supports \
  --to-kind proposition --to PROPOSITION_ID \
  --rationale "Morgan disputes awareness of any impact." \
  --author "A. Reyes"
```

Every link needs a written rationale.
See [`docs/authoring.md`](docs/authoring.md).

### Import

```sh
./target/debug-release/evidence ingest CASE_ID batch.json
```

The case identifier in the command must match the case identifier in the batch.
See [`docs/normalized-input.md`](docs/normalized-input.md).

## The office

The office commands operate on `office.sqlite`.
They do not open the evidence file, except `docket` and `matter show`.

```sh
./target/debug-release/evidence office init
./target/debug-release/evidence office seed
./target/debug-release/evidence office docket
./target/debug-release/evidence office --help
```

Use `--office-database PATH` to select a different office file.

A note in the office is append-only.
An edit writes a new version.
The earlier version stays readable.
A named person must set the acting user before office writes.

## The Windows program

The window has two panes.

- **Court** — one row for each court setting on the selected day
- **Office** — the evidence views for the selected matter.

`Ctrl+Shift+K` opens Court.
`Ctrl+Shift+O` opens Office.

In Office, choose a case.
Then open a view: standing, discovery, elements, timeline, issues, review, search, collation, packet, digest, or export.
The lower panel is for named review, intake of a normalized batch, and authoring.
`Alt` plus the underlined letter starts the matching command.
Each view also has a `Ctrl` shortcut.

The safe export is disclosable.
The work-file export is a separate action.

Enrichment Sweep (`Alt+X` or `Ctrl+E`) is where a person classifies a passage.
An adapter cannot say if a sentence is an assertion, a quotation, or a report of another person's words.
A person does that work.
See [`docs/enrichment.md`](docs/enrichment.md).

## Tests and checks

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

On Windows, include the graphical program:

```powershell
cargo test --all-features --features gui-winsafe
```

## License

The license is AGPL-3.0-or-later.

## More reading

- [`docs/domain-rules.md`](docs/domain-rules.md) — the rules that constrain the program
- [`docs/standing.md`](docs/standing.md) — what each charge element rests on
- [`docs/review-workflow.md`](docs/review-workflow.md) — review states and what each decision costs
- [`docs/authoring.md`](docs/authoring.md) — how a person writes into the case
- [`docs/export.md`](docs/export.md) — disclosable export and work-file export
- [`docs/search.md`](docs/search.md) — full-text search
- [`docs/suggestions.md`](docs/suggestions.md) — the analyzers
- [`docs/normalized-input.md`](docs/normalized-input.md) — the adapter input contract
- [`docs/enrichment.md`](docs/enrichment.md) — the enrichment sweep
- [`collation-first-plan.md`](collation-first-plan.md) — the design thesis
