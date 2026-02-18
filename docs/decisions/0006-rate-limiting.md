# ADR-0006: Rate Limiting and Spam Prevention

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

SemanticWeft nodes are open HTTP servers. Without any protection mechanism, a
node is vulnerable to:

- **Submission flooding** — an agent submitting units at a rate that exhausts
  the node's storage or CPU.
- **Sybil spam** — many pseudonymous agents each submitting at a low rate,
  collectively overwhelming the node.
- **Relay amplification** — a node faithfully relaying flooded units to its
  peers, spreading the attack across the network.

The network has no central authority to enforce global rate limits or to
maintain an identity registry. Any rate-limiting scheme must work in a
decentralised context.

At the same time, overly aggressive rate limiting would balkanise the network:
nodes that refuse to accept units from unknown agents undermine the open
federation model.

---

## Design constraints

1. **No central authority.** The protocol cannot mandate that all nodes consult
   a shared blocklist or rate-limit counter.
2. **Signing is optional in Phase 1–2.** Until Phase 3 signing is adopted,
   many agents will submit unsigned units.
3. **Node operators have different threat models.** A private research node
   and a public open relay have very different risk tolerances.
4. **The protocol must define how limits are communicated** even if it cannot
   mandate what those limits are.

---

## Options Considered

### Option A — Protocol-mandated rate limits

The spec mandates specific limits: e.g., 100 submissions per minute per IP,
500 per day per author string.

**Pros:**
- Consistent client experience across all nodes.

**Cons:**
- Arbitrary numbers that will be wrong for some operators.
- Impossible to enforce without a shared counter, which requires coordination.
- IP-based limits are trivially bypassed with proxies or IPv6 rotation.
- Counterproductive for high-throughput legitimate use cases
  (e.g., a batch pipeline submitting thousands of inference units).

### Option B — Node operator policy with standardised HTTP signalling

The spec defines how nodes communicate rate limits (HTTP status codes and
headers) but leaves the algorithm and thresholds to the node operator.

**Pros:**
- Consistent client behaviour (clients know how to back off) without
  constraining operator policy.
- Operators can tune limits to their infrastructure and trust model.
- Signing-aware nodes can apply different limits to signed vs. unsigned units.

**Cons:**
- Clients cannot predict in advance whether a given submission will be
  accepted. (This is acceptable: retry logic handles 429 gracefully.)

### Option C — Mandatory cryptographic signing (no unsigned units)

The spec requires all submitted units to carry a valid cryptographic signature.
Unsigned units are rejected with 401.

**Pros:**
- Eliminates sybil spam: each signing key is a real identity commitment.
- Makes attribution irrefutable (within the trust assumptions of the key).

**Cons:**
- Phase 3 signing is not yet implemented. Requiring it in Phase 4 would block
  all agents that predate Phase 3.
- Nodes that want to be open relays (accepting units from arbitrary agents,
  including new or anonymous ones) would be excluded.
- Key generation is cheap; a motivated attacker simply generates many keys.

### Option D — Proof of work (computational cost per submission)

Clients must include a proof-of-work token (e.g., a nonce such that
`SHA-256(unit_id + nonce)` has N leading zero bits) in a request header.
Nodes that want flood protection can require this; nodes that don't can ignore
it.

**Pros:**
- Raises the cost of bulk submission without requiring identity.
- Works without a central authority.

**Cons:**
- Disadvantages low-power clients (embedded devices, phones) relative to
  well-resourced spammers.
- The difficulty parameter is hard to calibrate: too low and it is no barrier;
  too high and it becomes a denial-of-service against legitimate agents.
- Adds complexity to every client implementation.

---

## Decision

**Option B is the primary approach. Option C (signing) is RECOMMENDED as
the preferred defence for nodes that choose to enforce it. Option D (PoW)
is available as an optional extension.**

### Normative rules

#### HTTP signalling (MUST)

1. When a node rejects a submission due to rate limiting, it MUST respond
   with HTTP **429 Too Many Requests**.
2. The 429 response MUST include a `Retry-After` header whose value is
   either the number of seconds to wait or an HTTP-date after which the
   client MAY retry.
3. The 429 response body SHOULD be a standard error object:
   `{ "error": "rate limit exceeded", "code": "rate_limit_exceeded" }`.

#### Operator policy (MAY)

4. Nodes MAY apply any rate-limiting policy they choose, including:
   - Per-IP token bucket
   - Per-author token bucket (keyed on the `author` field of submitted units)
   - Global submission cap
   - Differential limits for signed vs. unsigned units

#### Signing preference (SHOULD)

5. Nodes SHOULD apply stricter rate limits or require signing for units
   submitted by unknown authors.
6. A node that requires signing MUST respond with HTTP **401 Unauthorized**
   (not 403) when an unsigned unit is rejected on that basis, and the
   response body SHOULD explain the requirement.

#### Proof-of-work extension (MAY)

7. A node MAY advertise proof-of-work requirements in its discovery document
   (`/.well-known/semanticweft`) using the `pow_required` field:
   ```json
   { "pow_required": { "algorithm": "sha256", "difficulty": 20 } }
   ```
8. When PoW is required, a client MUST include an `X-Sweft-PoW` request
   header whose value is `<nonce>:<hash>`, where the hash satisfies the
   advertised difficulty against the input `<unit_id>:<nonce>`.
9. A node that rejects a submission for missing or invalid PoW MUST respond
   with HTTP **428 Precondition Required** and include the `pow_required`
   object in the response body.

#### Relay behaviour

10. Nodes that replicate units from peers via the sync protocol MUST NOT
    re-apply submission-facing rate limits to synced units. Rate limits
    protect the submission interface from agents; they do not govern
    node-to-node replication.

---

## Consequences

- Client libraries MUST implement retry logic on 429 responses, honouring
  the `Retry-After` header.
- The node discovery document (`/.well-known/semanticweft`) carries
  `pow_required` and `signing_required` fields so clients can adapt before
  their first submission.
- The conformance test suite (Phase 6) will verify that nodes return 429
  with a `Retry-After` header when they advertise a rate limit. The specific
  limit thresholds are not tested.
- A future ADR may address cross-node blocklist sharing (e.g., a gossip
  protocol for known-bad author DIDs) once Phase 3 signing is deployed.
