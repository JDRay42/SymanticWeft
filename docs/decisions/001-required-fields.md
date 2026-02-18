# ADR-001: Required vs. Optional Fields

**Date:** 2026-02-18
**Status:** Accepted

---

## Context

A Semantic Unit needs a minimum set of fields to be useful. Too few required fields and units become ambiguous or unprocessable. Too many and the barrier to creating a valid unit rises, discouraging adoption and making the schema harder to implement against.

The fields present in the initial schema example were: `id`, `type`, `content`, `created_at`, `author`, `confidence`, `assumptions`, `source`, `references`. The question was which of these to require on every unit.

## Decision

The following five fields are **required** on every Semantic Unit:

| Field | Reason |
|-------|--------|
| `id` | Every unit must be uniquely and stably addressable. Without an id, references are impossible and deduplication is undefined. |
| `type` | The semantic role of a unit fundamentally changes how its content is interpreted. A unit without a type is ambiguous to any reasoning layer. |
| `content` | The unit's claim or question in human-readable form. A unit without content has no semantic value. |
| `created_at` | Temporal context is necessary for assessing relevance and for detecting stale information. |
| `author` | Provenance is a core value of the protocol. A unit with no author cannot participate in any trust or accountability model. |

All other fields — `confidence`, `assumptions`, `source`, `references` — are **optional**.

## Rationale

**`confidence` is optional** because not all units express graded belief. Questions and constraints have no natural confidence value. Forcing every assertion author to quantify their certainty before publishing a unit creates friction and produces noise (authors guessing at values rather than reasoning about them). A missing `confidence` field is semantically distinct from `confidence: 0.0` and MUST be treated as such.

**`assumptions` is optional** because most simple assertions do not require explicit assumption-stating to be useful. The field is available for units where assumptions are material, but requiring it would produce empty arrays everywhere.

**`source` is optional** because many valid units — especially agent-generated inferences — have no external document to cite. Forcing a `source` field would produce meaningless placeholder values.

**`references` is optional** because the first assertion in any graph has nothing to reference. A unit must be able to stand alone.

## Consequences

- The minimum valid unit is small and easy to produce.
- Consumers MUST handle any combination of optional fields being absent.
- Tooling SHOULD encourage authors to include `confidence` on assertions and inferences without requiring it.
