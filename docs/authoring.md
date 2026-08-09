# Authoring

Import is how an adapter adds a machine's reading of a case. Authoring is how a
person adds their own: a contested proposition, and the typed relationships tying
source-grounded content to it. Both are mutations; neither is review.

## Authoring is not review

An authored proposition enters `unreviewed`, and so does an authored
relationship. Writing something down is not evidence that anyone checked it, so
both join the same queue as anything an adapter produced and wait for the same
named decision. The person who wrote a record may of course be the person who
later reviews it — but that is a second act, recorded separately in the trail.

Nor can authoring settle a proposition. Every authored proposition is
`contested`. `undisputed` is a conclusion about the state of the evidence, and
nothing in this kernel calculates one.

## What each mutation costs

`author_proposition` asks for a named person and something to say. It refuses an
empty proposition and refuses an identifier already in use, rather than replacing
the record that holds it.

`link_evidence` asks for a named person and, always, a **written rationale**. That
requirement is the mirror of the one review imposes: a relationship is drawn
across several sources and has no original of its own, so a reviewer verifying it
must supply a written basis rather than a locator — see
[`review-workflow.md`](review-workflow.md). The rationale is what that reviewer,
and every later reader, actually has to weigh. A relationship asserted without one
would be an assertion nobody can check.

Both endpoints must already exist and belong to the same case. An unknown
identifier is refused rather than quietly creating the node it names, and a link
may not reach across a case boundary in either direction. Nothing may stand in a
relationship to itself.

The same claim is never written twice. Asserting a relationship that already
exists — even with differently worded reasoning — is refused, so the original
keeps its own rationale and its own review history. A reviewer should not face two
rows saying the same thing, and no view should count it twice.

## Charges and element mapping

A charge is written with its statutory elements or not at all. A charge with no
elements cannot be reasoned about — the element matrix, the offense comparison,
and every question a defender asks of a charge are element-by-element — so at
least one is required and they are written in the same transaction. Ordinals come
from the order given rather than from the caller, because a statute's elements
have an order and a gap in it would be a transcription error.

`map_element` records how one proposition bears on one element. The assessment is
a **direction, not a weight**: `supports`, `opposes`, `uncertain`, or `excluded`.
Nothing aggregates them. An element with three supporting and three opposing
propositions is reported as exactly that, and `uncertain` is the honest,
first-class answer for most contested material rather than a placeholder for an
assessment somebody has yet to sharpen.

One proposition bears on one element in one direction. Filing the same
proposition under an element as both `supports` and `opposes` is not a richer
reading but a contradictory one, so the second is refused and names the direction
already recorded. Changing an assessment is a real act that should leave a trace;
until there is a path for it, the existing mapping stands.

Every mapping names the person who made it, and the element matrix reports that
name. Mappings written before authorship was recorded show no name rather than
being backfilled with one nobody actually stood behind.

Elements reach propositions through `element_links`, which carries no case column
of its own. Both the mutation and the views that read it check the case
explicitly, so one case's element matrix can never surface another's material.

## Commands

```sh
cargo run -- author case-hit-run-001 proposition \
  --text "Morgan did not perceive the impact." \
  --author "A. Reyes"

cargo run -- author case-hit-run-001 link \
  --from-kind content --from hr-content-client-driving \
  --relation supports \
  --to-kind proposition --to hr-prop-no-perception \
  --rationale "Morgan expressly disputes awareness of any impact." \
  --author "A. Reyes"
```

```sh
cargo run -- author case-hit-run-001 charge \
  --label "Leaving the scene of an accident" \
  --element "The defendant drove a vehicle." \
  --element "The vehicle was involved in an accident." \
  --element "The defendant left without identifying themselves." \
  --citation "Example Code § 20-166" --grade misdemeanor

cargo run -- author case-hit-run-001 mapping \
  --element <element-id> --proposition hr-prop-property-damage \
  --assessment supports \
  --notes "Damage evidence is stronger than the injury evidence." \
  --author "A. Reyes"
```

`--element` is repeated once per element, in statutory order; the ordinals and
the element identifiers come back in the command's output. `--id` is available on
each of these and is generated when omitted. `--relation` accepts
any `EdgeKind`; `--from-kind` and `--to-kind` accept any node the `edges` table
can point at — `content`, `source`, `proposition`, `event`, `edge`, `entity`, or
`advocacy`. Charges and elements are deliberately absent: elements reach
propositions through `element_links`, not through `edges`.

Afterwards, `review … queue` shows both new records waiting, and
`view … proposition <id>` resolves the link back to the exact locator in the
original it rests on.
