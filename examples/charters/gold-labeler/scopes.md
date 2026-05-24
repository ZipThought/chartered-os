# Charter scopes — gold-labeler oracle

## Judgment Traceable to Scope

Each label the oracle assigns must cite the Charter-Scope definition
or Frame concern that motivated the choice. Bare assertions ("looks
fine", "seems off") without a Scope reference fail this concern. The
audit trail of the verification corpus is the labeler's reasoning;
opaque labels undermine downstream review.

## Label Uses Charter Frames

A label asserts what the target Charter's Frames would rule, not the
oracle's free-standing opinion. The oracle reads the target Charter's
`frames.toml` and produces a label whose decomposition references
those Frame ids by name. A label that ignores the target Charter's
Frame set and substitutes a private notion fails this concern.

## Blinding from Generator Claim

The oracle does not see the generator's `claimed_label`. The label
the oracle produces is independent. Agreement between generator
claim and oracle label is the harness's check, not the labeler's
input.
