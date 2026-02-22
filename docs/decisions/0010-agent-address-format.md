# ADR-0010: Agent Address Format

**Status:** Accepted
**Date:** 2026-02-19

---

## Context

For push delivery (ADR-0009) to work, a `SemanticUnit`'s `audience` entries
must encode enough information to:

1. **Identify the recipient** — which specific agent should receive the unit.
2. **Route the delivery** — which node to send it to.

Two dimensions of the problem were resolved independently:

**Agent identity.**  SemanticWeft is designed for agent-to-agent communication;
human-readable handles are explicitly out of scope.  Agents already have
`did:key` identifiers (ADR-0001) that are globally unique, cryptographically
verifiable, and portable across nodes.  The `did:key` is therefore the natural
agent identifier — no separate username registry is needed.

**Node address.**  The system runs over HTTPS.  Including the scheme in stored
addresses is redundant.  More importantly, including the API version path
(e.g. `/v1`) would bake a versioning artifact into `SemanticUnit` records that
may persist indefinitely.  Audience entries should remain valid across API
revisions.

**Self-contained routing.**  The audience entry should carry both the agent
identity *and* the routing destination inline, without requiring a lookup in
the local peer list.  This keeps unit data independent of each node's
operational state — a unit is interpretable and deliverable on any node that
can read it.

---

## Decision

Agent addresses use the format:

```
did:key:z6Mk<agent-public-key>@<node-hostname>
```

### Rules

| Component | Rule |
|---|---|
| Left of `@` | The agent's full `did:key` DID |
| Right of `@` | The node's public hostname only — no scheme, no path, no port (unless non-standard) |
| Delimiter | Split on the **last** `@` to future-proof against DID methods that may contain `@` |
| Scheme | HTTPS is assumed and implicit |
| Path | Omitted; discovered at delivery time via `/.well-known/semanticweft` |

### Examples

```
did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuias8siQmsDNyZCeT@sweft.example.com
```

A local audience entry (no `@`) references an agent on the same node and
requires no outbound push.

### Delivery resolution

Because the API path is omitted, the sending node performs a one-step
discovery before the first push to any hostname:

```
GET https://<hostname>/.well-known/semanticweft
→ { "api_base": "https://<hostname>", … }
→ POST https://<hostname>/v1/agents/<encoded-did>/inbox
```

The `/.well-known/semanticweft` endpoint is the single stable surface that
never changes across API versions.  This indirection decouples the address
from the API implementation.

Discovery results **should be cached** by the sending node to avoid a lookup
on every push to a known hostname.

---

## Consequences

### Positive

- Audience entries are self-contained and version-stable.  A unit created
  today can be delivered correctly after an API upgrade.
- No central registry; routing is derived from the address itself.
- Consistent with how email (`user@host`) and Mastodon (`@user@host`) work,
  making the format intuitive to developers.
- `did:key` identifiers never contain `@`, so splitting on the last `@`
  is unambiguous for the current DID method.

### Negative / deferred

- **Node migration**: if an agent moves to a new hostname, existing audience
  entries pointing to the old hostname cannot follow.  This is accepted as
  correct behaviour — equivalent to changing an email address.  A forwarding
  mechanism is deferred.
- **Discovery cache invalidation**: cached `api_base` values may become stale
  if a node changes its URL structure.  A TTL and re-fetch strategy are
  deferred.
- **Multi-node presence**: the same `did:key` can be registered on multiple
  nodes.  Senders must address a specific `did@hostname`; they cannot send
  to "this agent wherever it lives".  Agent-side inbox aggregation is a
  client concern (see ADR-0011).

---

## Alternatives Considered

### `did:key@api_base` (path included)

Including `/v1` in the address (e.g. `did:key:z6Mk…@sweft.example.com/v1`)
would allow direct inbox URL construction without a discovery lookup.
Rejected because it bakes an implementation detail into persisted records.

### Structured audience entry `{ "agent_id": "…", "node": "…" }`

A JSON object in the `audience` array is unambiguous and extensible.
Rejected to avoid a schema change — `audience: Vec<String>` already exists
and the `@` separator is unambiguous for `did:key` DIDs.  The structured
form can be adopted in a future schema revision if needed.

### Agent DID with service endpoint

A DID document for the agent could advertise its home node as a service
endpoint, making `audience` entries just bare DIDs.  Rejected because it
requires DID resolution infrastructure that does not exist and introduces
a latency-critical external dependency into the delivery path.
