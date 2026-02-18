# ADR-002: Confidence Representation

**Date:** 2026-02-18
**Status:** Accepted

---

## Context

Assertions and inferences carry epistemic weight. Representing how certain the author is about a unit's content allows downstream reasoners to weight claims appropriately, flag low-confidence inferences, and detect when confident claims are later rebutted.

The options considered were:

1. **Float 0.0–1.0, optional** — standard probabilistic range, fine-grained, familiar from ML and Bayesian reasoning
2. **Float 0.0–1.0, required on assertions and inferences** — same representation but enforced for the most common unit types
3. **Enum: `low` / `medium` / `high`** — simpler, less precise, easier to read
4. **Omit confidence entirely** — defer to a later phase

## Decision

**`confidence` is an optional float in the range [0.0, 1.0].**

It is global (applicable to any unit type) but optional everywhere. No unit type requires it.

## Rationale

**Float over enum:** The float range is the standard representation in probabilistic systems and allows downstream reasoning that aggregates or compares confidence values. An enum collapses distinctions that agents may care about (0.51 vs. 0.99 are both "high") and is harder to evolve without breaking changes.

**Optional over required:** Requiring confidence on assertions and inferences would force authors to quantify certainty they may not have a principled basis for expressing. A fabricated `0.7` is worse than no value — it introduces false precision that downstream systems may over-rely on. Authors who have grounded confidence estimates SHOULD express them; those who don't SHOULD omit.

**Not deferred:** The field is simple enough to include now, and deferring it would mean existing units have no way to express confidence, requiring a breaking schema change later.

**Semantics:**
- `1.0` — the author is certain (use sparingly; genuine certainty is rare)
- `0.7–0.99` — the author believes this with high confidence
- `0.4–0.69` — moderate confidence; the author has significant uncertainty
- `0.0–0.39` — low confidence; the unit captures a possibility more than a belief
- absent — the author has not quantified; MUST NOT be treated as `0.0`

## Consequences

- Receivers MUST treat absent `confidence` as "unquantified", not as zero.
- Tooling SHOULD display or surface confidence values to users when present.
- Aggregation across units (e.g., computing a graph-level confidence) is left to the reasoning layer and is out of scope for the protocol.
- Future phases may add mechanisms for confidence to be updated by inference engines, but the base field is immutable with the rest of the unit.
