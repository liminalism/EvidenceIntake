# Export

An export is where collation leaves the tool: a chronology for a client meeting,
an attachment to a discovery letter, the factual predicate of a motion. Two rules
govern it, and neither is left to whoever is producing it.

## Every factual line resolves to an original

Each exported evidence item carries the source and the exact locator it came
from, so any sentence a reader doubts can be opened in one action. A proposition
that resolves to no source-grounded evidence is **not** exported as a bare
assertion; it is listed under `unsupported` with the reason. The check is made
while the export is assembled, not assumed: an evidence item that somehow reached
the export with an empty locator fails the whole export rather than travelling as
a fact nobody can check.

Note what "source-grounded" means here. Only content resolves to an original
locator. A proposition supported solely by another proposition, or by an event,
has nothing a reader can open, and is reported as unsupported.

## Privileged analysis does not leave in a disclosable export

`--audience disclosable` is for anything that may go outside the defense team.
It **never reads** the advocacy, annotation, or brief tables. That exclusion is
structural rather than a filter on the `privileged` flag, because a flag is only
as good as every future writer that sets it, and the cost of one wrong write is
disclosing work product.

`--audience work-file` is the team's own complete file, privileged analysis
included, and says so in `includes_privileged`. Never produce one in response to a
discovery obligation.

Both audiences carry the production ledger. An exhibit list without the
completeness record hides what is missing, which is usually the thing worth
knowing.

## Nothing leaves silently

Two counts sit in the header rather than making a reader tally the body:

- `rejected_evidence_omitted` — material a reviewer rejected, left out as it is
  everywhere else, but never dropped without saying so.
- `unreviewed_evidence_included` — lines resting on an extraction or a
  relationship no person has reviewed. Every line already states its own review
  state; the count means a defender deciding whether to attach this to a filing
  does not have to count them.

Machine suggestions are exported rather than hidden, with their state attached.
Hiding them would misrepresent the file; exporting them silently would be worse.

## Commands

```sh
cargo run -- export case-hit-run-001
cargo run -- export case-hit-run-001 --audience work-file
```

The default audience is `disclosable`: the safe one is the one you get by
forgetting to choose.
