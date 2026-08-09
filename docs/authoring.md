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

`--id` is available on both and is generated when omitted. `--relation` accepts
any `EdgeKind`; `--from-kind` and `--to-kind` accept any node the `edges` table
can point at — `content`, `source`, `proposition`, `event`, `edge`, `entity`, or
`advocacy`. Charges and elements are deliberately absent: elements reach
propositions through `element_links`, not through `edges`.

Afterwards, `review … queue` shows both new records waiting, and
`view … proposition <id>` resolves the link back to the exact locator in the
original it rests on.
