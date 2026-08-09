# Where the case stands

`view <case> standing` is the view to open first. Everything else in this tool
answers *what the case contains*. This one answers what its charges rest on.

A defender with forty open files does not need an inventory. They need to know,
in the two minutes before a hearing, which element the state has barely covered,
which one rests entirely on a report that might not survive a foundation
objection, and which of the gaps in the record actually touch a charge.

## What it reports, element by element

**Directions, counted separately.** How many propositions a person filed as
supporting the element, opposing it, uncertain, or excluded. These are never
added together and never netted against each other. Three supporting and three
opposing is not zero; it is six things a person has to think about.

**Distinct sources behind the support.** Three propositions quoting the same
report are not three sources. An element can look well covered in the matrix and
still rest on a single page, and only the count of *originals* shows that.

**The sole source, named.** When every supporting proposition traces back to one
source, that source is named — and it appears again under
`load_bearing_sources` with every element it alone carries. This is the single
most actionable thing here: it is where a suppression motion, a foundation
objection, or a chain-of-custody question is worth the hours, because the
element has no second leg to stand on.

**Support nobody has checked.** Supporting propositions where no person has
reviewed or verified any of the evidence underneath them. An element resting on
unopened material is standing on an assumption about what the original says.

**Unbacked mappings.** Propositions filed under an element that nothing
source-grounded reaches. Somebody wrote down a claim and connected it to a
charge without connecting it to evidence.

## Live disputes

Propositions carrying source-grounded evidence in both directions, reported with
the elements they bear on.

These are not defects to be resolved. A proposition with evidence pulling both
ways is the contested ground the case is actually fought on, and the kernel's
entire posture is that it stays contested until a person decides otherwise. What
this adds is *where* the fight lands: knowing that the disputed proposition
carries element 2 of the felony is a different morning than knowing it carries
nothing.

## Gaps, placed against the charges

The analyzers already report unresolved references, unmapped propositions,
unsupported propositions, and clock disagreements. `standing` re-reports them
sorted by whether they touch a charged element, and names which.

That reordering is most of the value. A reference nobody resolved is a to-do. A
reference nobody resolved *under element 3 of the felony* is the afternoon's
work. The tool should not make a defender work out which is which by hand.

## What it will not say

No score. No strength. No likelihood, ranking, or recommendation. There is a
test — `nothing_in_the_standing_view_scores_the_case` — that serializes the
whole view and fails if any of those words appear in it.

The line is between structure and verdict. "This element has one supporting
proposition resting on one source, and nobody has opened it" is a description of
the record; every part of it can be checked and argued with. "This element is
weak" is an argument, and it belongs to the person who signs the motion. A tool
that makes it for them is worse than one that stays quiet, because a defender
reading a strength number stops looking — and the number was never a measurement
of anything, only a rearrangement of counts they could already see.

Ordering follows the same rule. Elements appear in statutory order, not sorted
by how thin they are. Gaps are ordered by whether they touch a charge, which is
a fact, not by how bad they are, which is a judgment.

## Commands

```sh
cargo run -- view case-hit-run-001 standing
cargo run -- view case-vehicle-stop-001 standing
```
