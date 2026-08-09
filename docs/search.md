# Finding a passage

```sh
cargo run -- search case-hit-run-001 'hatchback'
cargo run -- search case-hit-run-001 '"paint transfer"'
cargo run -- search case-hit-run-001 'sedan NOT silver' --limit 10
cargo run -- search case-hit-run-001 'streetlight OR crossing'
```

Queries use SQLite FTS5 syntax: bare words, `"exact phrases"`, `AND`, `OR`,
`NOT`, and `prefix*`. A query FTS5 cannot parse is reported back with what was
typed, because a stray quote is a typo rather than a broken database.

## What a hit carries

The matched passage with the matching words bracketed, the full text it came
from, the **exact locator in the original** — a page, a timestamp, a line — and
the source's logical name. A hit is a place to look, and the point of the
locator is that looking takes one action rather than a search through a PDF.

It also carries `bears_on`: the propositions the case has already tied that
passage to, with the relationship. An empty `bears_on` is information, not
absence — it means nobody has done anything with this passage yet.

## What it searches, and what it does not

**Extracted content only.** The words in originals: statements, document
assertions, observations, transcript lines.

**Never privileged work product.** Advocacy items, annotations, and decision
briefs are not indexed at all, the same way `export_case(_, Disclosable)` never
queries those tables. Excluding them structurally rather than filtering them
afterwards means one wrong query cannot surface attorney analysis somewhere that
does not know it is privileged. `search_never_reaches_privileged_work_product`
seeds a canary phrase into two privileged tables and fails if a search finds it.

**Not propositions or elements.** Those are deliberate: a case has a few dozen
propositions and you read all of them in `standing` or `export`. Search exists
for the haystack, and the haystack is content — a serious case's discovery will
run to tens of thousands of transcript lines. Searching for a word that appears
only in a proposition returns nothing, and that is the intended boundary rather
than a gap.

## On ranking

Results are ordered by BM25 relevance: how well a passage matches the words that
were asked for.

That is a rank of *matches*, not of evidence, which is why it is allowed here
when nothing else in this kernel is ranked. Nothing about the order claims a
passage is true, admissible, corroborated, or important — only that it contains
more of what was typed. The relevance number itself is not reported, because a
number printed next to an excerpt gets read as a measurement of the excerpt.

## Why it lives inside SQLite

The index is an FTS5 external-content table: it stores the terms, not a second
copy of the text, and reads the words back out of `content` itself. Triggers
maintain it inside the same transaction as the write, so a committed excerpt is
always findable and a rolled-back one never is.

A separate search engine beside the database would be a second store of an
evidentiary record, able to drift from the append-only trail — holding a passage
that was rolled back, or missing one that committed. For a tool where the file
*is* the artifact a defender relies on, that is the wrong trade at any speed.
