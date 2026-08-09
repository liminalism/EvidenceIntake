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

