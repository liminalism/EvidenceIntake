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

