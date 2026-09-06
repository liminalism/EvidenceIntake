# Domain and epistemic rules

These rules are invariants for the kernel and future UI.

1. Originals are immutable source records. Derived artifacts receive their own
   source identity and point back to exact original segments.
2. A source containing a statement, an officer reporting that statement, and
   an attorney proposition based on it are distinct nodes.
3. Propositions remain contested. Supporting and contradicting material can
   coexist; no confidence-weighted truth value is calculated.
4. Every extracted content item identifies an exact source segment. Human and
   machine-derived entries also record extractor and review state.
5. Machine suggestions begin as `suggested`. Only a person can mark an item
   `reviewed`, `verified`, or `rejected`.
6. Raw time is never replaced. Normalized intervals and clock offsets are
   reviewable hypotheses with an explicit basis.
7. A person mention is not a person. Resolution may remain absent, and
   `possibly_same_person` does not merge records.
8. Negative observations are bounded to what a source shows. Absence from a
   recording and absence from a production are not proof that an event or
   object did not exist.
9. Admissibility is not a global property. Potential objections, purposes,
   foundation needs, rulings, and attorney status belong in advocacy work
   product.
10. Advocacy items and decision briefs are privileged by default and are
    excluded from routine source/discovery exports.
11. Timeline lanes remain separate. Recorded events, witness accounts, police
    narrative, client account, and attorney hypotheses never collapse into one
    authoritative sequence.
12. Every factual sentence in a future export must resolve to a proposition or
    evidentiary-content node and from there to an exact original locator.
13. Source creation time and alleged-event time are distinct. A police report
    written at 23:30 can assert that a collision happened at 21:07; neither
    value substitutes for the other.
14. Original source modality is explicit. OCR text points to a document,
    timestamped ASR points to original audio, and a scene observation points to
    original video frames.
15. Machine confidence describes extraction reliability only. It is never
    proposition confidence and cannot establish an offense element.
16. Every review decision names the person accountable for it and is appended
    to an immutable trail. Review state on a record is a cache of the latest
    decision; the trail is the artifact.
17. `verified` asserts that a person opened the original. Where a record has one
    exact locator, the decision must cite that locator and it must match.
    Relationships, propositions, and events span sources and have no single
    original, so verifying one requires a written basis instead.
18. Rejection requires a written reason. Removing evidence from view without
    one leaves nothing for a later reader to weigh.
19. Review is continuous, not terminal. A verification may be withdrawn and a
    rejection reinstated as later material arrives; neither erases the earlier
    decision. Nothing may return to `unreviewed` or `suggested`, because those
    states record what import produced, not what a person concluded.
20. Authoring is not review. A proposition or relationship a person writes down
    enters `unreviewed` and waits like anything else; writing it is not evidence
    that anyone checked it. An authored proposition is always `contested`.
21. An authored relationship carries a written rationale. It spans sources and
    has no original of its own, so the rationale is the only thing a reviewer or
    later reader can weigh. The same claim is asserted once: repeating it is
    refused rather than written twice.
22. An element assessment is a direction, not a weight. `supports`, `opposes`,
    `uncertain`, and `excluded` are never aggregated, and `uncertain` is a
    first-class answer rather than an unfinished one. One proposition bears on
    one element in one direction; filing it under two is contradictory, not
    richer.
23. A charge is written with its elements or not at all, in statutory order. A
    charge with no elements cannot be reasoned about, because every question a
    defender asks of a charge is element-by-element.
24. Case boundaries hold in the views as well as the writes. `element_links`
    carries no case column, so both the mutation and the queries that read it
    check the case explicitly: no case's workspace may surface another's
    material.
25. Work product is versioned by superseding, never by overwriting. An earlier
    reading is what an attorney thought when they made a decision, so it stays
    readable. Only the current version may be revised; views report the current
    version, and a superseded record is not a second record.
26. Work product carries no review state. Review asks whether an extraction
    faithfully represents an original; an attorney's own analysis is not an
    extraction, and there is nothing to check it against.
27. An issue's follow-up tasks are the ones linked to that issue. A workspace
    that listed every open task in the case would send a defender to chase work
    belonging to an unrelated question.
28. A disclosable export never reads the privileged tables at all. Excluding
    work product structurally, rather than by filtering a `privileged` flag,
    means one wrong write cannot disclose it.
29. An export names what it left out and what it left unchecked. Rejected
    material is counted, not silently dropped, and evidence no person has
    reviewed is counted, not silently attached.
30. An analyzer proposes and never concludes. It may not review, verify, merge,
    score, or alter a record a person wrote; its whole authority is to point at
    a pair and state a reason a defender can reconstruct.
31. A reviewer is not asked twice. A claim the case already holds — asserted,
    proposed, or rejected — is skipped on later runs, and the check is
    direction-blind because an analyzer points at a pair, not an orientation.
32. A proposal is a claim and a finding is a gap. Proposals are written and
    reviewed; findings are derived on every run, stored nowhere, and dismissed
    only by closing the gap they report.
33. Two entities may share a name. Refusing the second write would merge them by
    default; `duplicate-entity` raises the question and a person answers it.
34. No tolerance windows. How much clock disagreement matters is the defender's
    judgment, so the tool reports that two sources differ and never how much
    difference is acceptable.
35. Structure may be reported; a verdict may not. The kernel will say that an
    element's support all traces to one source, that nobody has opened it, and
    that a gap in the record touches a charged element — each a checkable fact
    about how the material is connected. It will not say that an element is
    weak, score a charge, rank elements by strength, or estimate an outcome.
    Ordering obeys the same line: elements appear in statutory order and gaps
    are ordered by whether they touch a charge, which is a fact, rather than by
    how serious they are, which is the defender's call.
36. Search reaches the evidentiary record and nothing else. Only extracted
    content is indexed; privileged work product is left out structurally rather
    than filtered afterwards. Hits are ordered by how well a passage matches the
    words asked for, which ranks matches and not evidence — the relevance value
    is never reported, because a number printed beside an excerpt is read as a
    measurement of the excerpt. The index lives in the same file and the same
    transaction as the writes, so it can never hold a passage the record does
    not.
37. Cases do not share records. A speaker, attributed person, parent statement,
    production, superseded original, element mapping, or relationship may not
    name a row that belongs to another case. The store refuses the write; the
    schema refuses it too, so a hand-edited database cannot smuggle one
    matter's evidence into another. Identifiers are unique in the file, not
    per case, which is how that refusal stays unambiguous.
38. A visual finder points; it does not assert. Keyframe embeddings are a
    case-local index against derived stills, not content and not an edge.
    Internal similarity selects the reviewer's bounded candidate pool, then
    the pool is ordered chronologically; the number is never reported.
    Retrieval writes no observation; only what a reviewer confirms is
    authored. The index is excluded from every export.
39. The kernel stays clean. The office layer — clients, matters, calendar,
    notes, assignments, office search — is built beside the evidence kernel and
    never inside it. No office concern enters the evidence schema, and every
    invariant above holds unchanged with the office layer present.
40. Two databases, no foreign key. `office.sqlite` sits beside
    `evidence.sqlite`; they never share a transaction. A matter carries an
    `evidence_case_id` as a bare identifier that the office layer stores and
    never resolves. Exactly one place in the workspace holds both open, and it
    reports a matter naming a case the kernel does not hold as the broken link
    it is — distinct both from a matter with no case and from a case with
    nothing outstanding.
41. Privileged material is unreachable from the office, not filtered out of it.
    `office-core` has no dependency on the kernel and no way to open an evidence
    database, so a docket row is built by code that cannot read `advocacy_items`,
    `annotations`, or `decision_briefs`. This is rule 36's structural exclusion
    applied one level up, and it is discharged by the dependency graph.
42. Identity across the boundary is a person's decision. A client is tied to a
    kernel entity only when a named person resolves a prompt; candidates are
    computed live from name and contact overlap and are never stored. Only
    `linked` and `dismissed` are written, and `dismissed` exists so a declined
    prompt is not offered again. Nothing is ever merged — rule 7 extended
    across the two databases.
43. A note is append-only and its authorship is immutable. Notes cannot be
    updated or deleted in place; the schema refuses both. An edit is a new
    version superseding the old one, every view filters superseded rows, and
    the author and creation time of a version can never change. Rule 24's
    versioning applied to office writing, and made structural because "no user
    can silently alter another author's note" is a stronger claim than "a
    reader can see that it changed".
44. Operational time is not evidentiary time. A court setting and a deadline
    are scheduling, dated in the local civil calendar the courthouse keeps; a
    kernel event is a competing account of what happened, and rule 6 governs
    it. Audit timestamps on both sides are UTC, so an append-only trail stays
    comparable. The two clocks never meet.
