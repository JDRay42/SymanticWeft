# ADR-003: Typed References

**Date:** 2026-02-18
**Status:** Accepted

---

## Context

Units reference each other to form a graph. The question is whether those edges carry semantic meaning (typed references) or are plain pointers whose meaning is implied by context.

Options considered:

1. **Plain id-only references** — a reference is just a UUIDv7; the relationship is inferred from the referencing unit's type
2. **Typed references (required)** — each reference object carries both an `id` and a `rel` field describing the relationship
3. **Optionally typed** — `rel` is an optional field on reference objects; plain id references are also valid

## Decision

**References are typed.** Every reference object MUST include both `id` and `rel`. The valid relationship types are:

| `rel` | Meaning |
|-------|---------|
| `supports` | This unit provides evidence or reasoning for the referenced unit. |
| `rebuts` | This unit argues against the referenced unit's content. |
| `derives-from` | This unit's content was derived from the referenced unit. |
| `questions` | This unit raises a question about the referenced unit. |
| `refines` | This unit narrows or specializes the referenced unit's claim. |

## Rationale

**Typed references are the primary value of the graph.** A web of pointers that says "these units are related" is marginally useful. A graph where every edge says "A supports B", "C rebuts D", "E derives from F" allows automated reasoning: path-finding through support chains, detecting contradictions (units that both support and rebut the same claim), finding the assumptions underlying an inference.

**Unit type alone is insufficient to convey relationship.** A `challenge` unit SHOULD rebut something, but it might also support part of what it challenges, or question an assumption rather than the conclusion. Encoding all of this in the unit's type would require an unwieldy number of types. Encoding it in the reference's `rel` is cleaner and more expressive.

**Optionally typed references were rejected** because they produce inconsistent graphs. If some references carry `rel` and others don't, consumers must handle the missing case and lose the guarantee that every edge is interpretable.

**The five semantic relationship types cover the essential cases:**
- `supports` and `rebuts` are the core epistemic operations (agreement and disagreement)
- `derives-from` makes inference chains explicit
- `questions` allows a unit to express targeted doubt about another
- `refines` supports hierarchical knowledge without creating a new "parent/child" mechanism

A sixth type, `notifies`, was added during implementation for protocol-level system notifications (e.g., fan-out delivery failures reported back to an author's inbox). It is distinct from the five semantic types: `notifies` is used by nodes, not by agents constructing knowledge-graph units. Extensions that need additional *semantic* relationship types must use namespaced rel values (ADR-004).

## Consequences

- Every reference requires a `rel` value; plain id-only references are not valid.
- Consumers can traverse the graph by relationship type without inspecting unit content.
- The semantic relationship type set is `supports`, `rebuts`, `derives-from`, `questions`, `refines`. The protocol-level type `notifies` is reserved for node infrastructure use.
- Cross-type reference validity (e.g., a `question` unit using `rel: "rebuts"`) is not enforced by the schema; it is left to linting tools and reasoning layers to flag unusual combinations.
