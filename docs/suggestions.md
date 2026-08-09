# Assisted collation

Analyzers propose. They do not conclude.

An analyzer reads the graph a person already built and points at pairs worth a
second look. Each proposal enters as `suggested`, joins the review queue behind
the same rule as everything else, and stays visibly unconfirmed until a named
person acts on it. `review queue` shows them first and names which analyzer
proposed each, the way extracted content names its extractor.

## No model, no score

These are rules over data the case already holds: interval arithmetic and joins.
There is no model here and no confidence value, and that is deliberate twice
over. A suggestion a defender cannot reconstruct is one they cannot argue with,
so every proposal carries a rationale stating the reason in full. And a number
attached to a proposal would be read as a probability that it is *true*, which is
exactly the judgment this kernel refuses to make.

## What an analyzer may never do

Mark anything reviewed or verified. Merge two records. Alter a record a person
wrote. Or re-propose something a reviewer already rejected — someone who has said
no does not get asked again on the next run.

Running the analyzers twice proposes nothing new. A claim the case already holds
is counted and skipped, whether a person asserted it, an earlier run proposed it,
or a reviewer rejected it. That check is **direction-blind**: an analyzer points
at a *pair*, so a person who wrote `b impeaches a` has already answered the
question and the mirror image is not a second thing to review.

## The analyzers

`temporal-overlap` proposes `temporally_overlaps` between events in **different
lanes** whose normalized intervals overlap. Lanes never collapse; this says only
that two accounts describe overlapping time, never that either is the right one.
Intervals are half-open — an event beginning exactly as another ends does not
overlap it — and an event with no recorded end is a point in time, not an
infinity.

`contradiction-candidate` proposes `contradicts` between two excerpts bearing on
the same proposition in opposite directions. It is a tension to examine, not a
finding that either excerpt is wrong.

`conflicting-attribution` proposes `impeaches` between two accounts attributed to
the **same** witness that bear opposite ways on one proposition. One witness
changing their account is a question about the witness; two different witnesses
disagreeing is a question about the facts, which is the analyzer above. The two
never claim the same pair.

## Commands

```sh
cargo run -- suggest case-vehicle-stop-001
cargo run -- suggest case-vehicle-stop-001 --analyzer temporal-overlap
cargo run -- review case-vehicle-stop-001 queue
```

With no `--analyzer`, every analyzer runs. The output reports what each proposed
and how much it found but did not write because the case already held it.
