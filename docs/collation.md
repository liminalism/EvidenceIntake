# Time & Place Index

The collation index is a source-grounded navigation view. It arranges active
content without merging records or declaring that two files document one
event.

```sh
cargo run -- view case-vehicle-stop-001 collation
```

The view contains three arrangements of the same passages:

- `by_date` uses only the calendar portion of `normalized_start` and keeps the
  entries in chronological order;
- `by_location` folds case and repeated whitespace in `location_text`, but does
  no address expansion, geocoding, abbreviation matching or semantic inference;
  and
- `shared_anchor_unconfirmed` contains only date-plus-location groups spanning at least
  two distinct immutable sources.

It also contains two integration-work sections:

- `source_coverage` gives raw counts, per original, of passages carrying raw,
  creation, asserted and normalized date anchors and locations; and
- `needs_placement` names every active passage missing a normalized date,
  location or both, while retaining the original locator needed to fix it.

These are counts and work lists, not percentages or assessments. Rejected
content is absent from both.

Every entry retains its source id and name, source kind, exact locator, text,
raw time, asserted time, normalized interval, written time basis, location,
content-creation time, machine attribution and review state. Raw, creation and
asserted times are displayed but never substituted for a missing normalized
date.

Each shared-anchor group states the exact keys it shares and says that the records
remain separate and their relationship is not established. Opening this view writes no relationship and adds nothing to
the review queue. A lawyer may later author a relationship or a named work
collection after opening the originals.

`without_normalized_date` and `without_location` are deliberate work signals.
They show how much discovery cannot yet participate in those arrangements;
they are not quality scores.

This first pass is intentionally conservative. For example, `Oak St` and
`400 block of Oak Street` remain separate location groups. Reconciling them
requires explicit structured location work rather than an invisible fuzzy
match.

## Native workspace

The Windows workspace exposes this index through **Time & Place Index**
(`Alt+G`). Unlike the CLI's machine-readable JSON, the native view renders a
review sheet with a short summary, source-anchor coverage, possible shared-key
groups, placement work, chronological entries and a compact location index.
The navigation rail carries twelve views in a denser row so they remain above
the collation and export actions at the window's minimum height, which
`the_rail_fits_above_the_minimum_height` checks.
