# ADR-0003: Trust Model — In-band vs. Out-of-band

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

Cryptographic identity (ADR-0001 and ADR-0002) answers *who* signed a unit.
**Trust** is the orthogonal question: *how much weight should I give to what
this agent says?*

Trust is domain-specific, transitive, and subjective. Agent A may trust agent
B completely on climate science but not at all on financial regulation. Agent
C may trust A's trust-in-B as a reason to also trust B. These relationships
are themselves knowledge claims — they deserve the same first-class treatment
as any other claim in the graph.

The question to decide is: **where do trust relationships live?**

---

## Options Considered

### Option A — Out-of-band trust (local configuration only)

Each agent or node maintains a private trust store — a configuration file or
database table that maps DIDs to trust levels. Trust is never expressed as
units; it is an implementation detail of each participant.

**Pros:**
- Simple to implement: just a key-value store.
- No schema change required.
- Trust data cannot leak into the public graph if that is undesirable.

**Cons:**
- Makes trust opaque. If agent X reaches a conclusion partly because it trusted
  agent Y, there is no way for a downstream consumer to see or challenge that
  trust relationship.
- Trust reasoning cannot be inspected, audited, or traversed with the same
  tools used for knowledge reasoning.
- Breaks the core design principle that all reasoning in a SemanticWeft graph
  should be transparent and traceable.
- Different nodes may silently apply different trust configurations to the same
  graph, producing different conclusions with no record of the divergence.

### Option B — In-band trust as a new `trust` unit type

Add `trust` to the `UnitType` enum. A trust unit expresses "I, the author,
vouch for this other agent to degree X in domain Y."

**Pros:**
- Trust is first-class in the type system.
- Validators can enforce trust-specific field requirements (subject DID,
  scope, level are required for a trust unit; content is the human
  explanation).
- Clean semantics: unit type communicates intent without needing to inspect
  extension fields.

**Cons:**
- Breaking change to the `UnitType` enum and the JSON Schema.
- Premature: the trust use cases are not yet well-enough understood to
  nail down the schema for a dedicated type.
- A `trust` unit cannot meaningfully have `references` in the current
  reference model (trust relationships are between agents, not units).

### Option C — In-band trust using existing types + namespaced extension fields

Express trust relationships as `assertion` or `constraint` units (both are
semantically appropriate — an assertion of trust, or a constraint stating the
boundaries within which another agent's claims hold). Trust-specific semantics
are carried in `x-sw-trust-*` extension fields alongside a human-readable
`content` field.

**Pros:**
- No type-system change. Works with the current schema.
- Trust is in the graph: it can be traversed, challenged with a `challenge`
  unit, revoked with a `challenge`, and reasoned about with the same API as
  any other unit.
- Explicitly provisional: the `x-sw-trust-*` namespace signals "this is an
  extension that will migrate to a first-class field when the semantics are
  mature."
- `content` remains required, so trust units remain human-readable.

**Cons:**
- Trust units look identical to knowledge units at the type level; consumers
  must inspect extension fields to identify them.
- Extension field validation is weaker than type-level validation.

---

## Decision

**Option C is adopted** as the Phase 3 trust model, with Option B flagged as
the intended upgrade path once trust semantics are fully defined.

### Trust unit structure

A trust relationship is expressed as an `assertion` or `constraint` unit with
the following `x-sw-trust-*` extension fields:

```json
{
  "id": "019c6f50-4f89-7a93-88c8-bee0335b9920",
  "type": "assertion",
  "content": "I trust did:key:z6Mk... for claims about climate-science at confidence 0.9.",
  "created_at": "2026-02-18T06:00:00Z",
  "author": "did:key:z6MkAuthor...",
  "confidence": 0.9,
  "x-sw-trust-subject": "did:key:z6MkTrusted...",
  "x-sw-trust-scope":   "climate-science",
  "x-sw-trust-level":   0.9,
  "proof": { ... }
}
```

| Extension field | Type | Required | Description |
|-----------------|------|----------|-------------|
| `x-sw-trust-subject` | string (DID) | Yes (if trust unit) | The agent being trusted |
| `x-sw-trust-scope` | string | No | Domain or topic; omit to mean "all topics" |
| `x-sw-trust-level` | float 0–1 | Yes (if trust unit) | Degree of trust; mirrors `confidence` semantics |

The `x-sw-` prefix is reserved for the SemanticWeft core specification.
Application extensions should use a different prefix (e.g. `x-myapp-`).

### Using `constraint` for trust boundaries

`constraint` is appropriate when the trust claim is normative — "agents
processing this graph MUST treat the following agent as authoritative for the
following domain":

```json
{
  "type": "constraint",
  "content": "did:key:z6MkAuditor... is the authoritative source for SOC 2 compliance claims in this graph.",
  "author": "did:key:z6MkOperator...",
  "x-sw-trust-subject": "did:key:z6MkAuditor...",
  "x-sw-trust-scope":   "soc2-compliance",
  "x-sw-trust-level":   1.0
}
```

### Trust revocation

Revoke a trust relationship by issuing a `challenge` unit that:
1. References the original trust unit (rel: `contradicts`).
2. Sets `content` to explain why the trust is withdrawn.
3. Is signed by the same author as the original trust unit (same DID).

Verifiers that see a trust unit with a signed `challenge` referencing it from
the same DID SHOULD treat the trust as revoked as of the challenge's
`created_at` timestamp.

### Trust inference (transitivity)

The protocol does not mandate any specific transitive trust algorithm.
Implementations MAY compute transitive trust by traversing trust units in the
graph; if they do, they SHOULD apply a decay factor per hop (e.g. Levien's
Advogato algorithm). This is left to implementer discretion because:
- Transitive trust is context-dependent (A trusts B; B trusts C does not
  automatically mean A trusts C).
- Mandating an algorithm too early would constrain use cases we haven't yet
  encountered.

---

## Consequences

- No change to `UnitType`, `Reference`, or the core validation logic in
  Phase 3. Trust units validate as regular units.
- The extension field name check in `validate_unit` must recognise
  `x-sw-trust-*` as valid (currently all `x-` names pass; this is already
  satisfied).
- The spec must document the `x-sw-trust-*` fields and their semantics.
- A future ADR should formalize the `trust` unit type once the schema is
  stable. When that happens, existing `assertion`/`constraint` trust units
  can be migrated by changing their `type` field — no other field changes
  are needed because the `x-sw-trust-*` fields carry forward.
- Node implementations that want to support trust-aware graph queries will
  need to index `x-sw-trust-subject` and `x-sw-trust-scope`; this is
  implementation advice, not a spec requirement at this stage.
