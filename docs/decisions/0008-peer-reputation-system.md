# ADR-0008: Peer Reputation System

**Status:** Accepted; Partially implemented (Phase 2 community voting gate live; Phase 2 cross-node propagation and Phase 3 pending)
**Date:** 2026-02-19
**Updated:** 2026-02-22

---

## Context

SemanticWeft nodes form a decentralised peer-to-peer network without a central
registry or authority. Peer discovery spreads through gossip: nodes learn about
new peers from peers they already know. This openness means that bad actors can
inject fake peer records, announce unreachable endpoints, or otherwise degrade
the quality of a node's peer list.

Additionally, when peer lists are shared between nodes (pull-on-connect), the
receiving node currently has no basis for deciding which peers to trust, how to
weight incoming reputation claims, or which peers to evict when its peer list
reaches the configured cap (`SWEFT_MAX_PEERS`).

The peer count cap introduced alongside this ADR means that peer slot
allocation is a finite, valuable resource. A principled reputation model is
needed to make eviction fair and to make the peer slot worth something to a
well-behaved node.

### Inspirational prior art

**EigenTrust (Gnutella / BitTorrent):**
Each node stores a local trust score for every other node it has interacted
with. When computing the *global* trust of a node, local scores are weighted
by the reputation of the scorer — creating a self-referential ranking that is
highly resistant to collusion by low-reputation nodes.

**PGP Web of Trust:**
Signatures from highly-trusted keys carry more weight than signatures from
unknown keys. Trust is transitive but attenuates over hops.

**Bitcoin addr relay:**
Peers relay `addr` messages (peer advertisements) but weight their acceptance
of relayed addresses by the reputation of the relaying peer and the freshness
of the address.

---

## Problem statement

When Node A receives a peer list from Node B that includes Node C with a
claimed reputation, A must decide:

1. Should A add C to its peer list at all?
2. If so, what reputation should A assign C locally?
3. If A later hears a conflicting reputation for C (from Node D), how should A
   reconcile the two claims?
4. When A's peer list is full and a new high-reputation peer is discovered, who
   gets evicted?

---

## Decision

### Phase 1 — MVP (implemented)

The data model is established now so the schema never needs to change.
Each `PeerInfo` record carries:

- `reputation: f32` in the range `[0.0, 1.0]`, defaulting to `0.5`.
- `last_seen: Option<String>` — ISO 8601 timestamp of the most recent
  successful contact.

Eviction policy: when the peer list is full and a new peer is discovered, evict
the peer with the lowest reputation, breaking ties by oldest `last_seen`.
New peers are assigned the neutral reputation `0.5`.

### Phase 2a — Community voting gate (implemented)

Reputation adjustments are community-internal. The following rules govern
`PATCH /v1/peers/{node_id}`:

#### Caller identity

The caller must supply an `X-Node-ID` HTTP header containing its own `node_id`
(DID). Requests without this header are rejected with `403 Forbidden`.

This header is self-asserted; cryptographic verification is deferred to Phase 3.
The trust model mirrors existing peer-exchange gossip: nodes trust the node_id
they receive from an HTTP call to the same degree they trust any other unsigned
peer announcement. Phase 3 will upgrade this to verified Ed25519 claims.

#### Community membership gate

The caller's `node_id` must appear in the receiving node's local peer list.
Outsiders — nodes the community does not know — cannot influence reputation
scores. This makes reputation management strictly community-internal.

#### Community voting threshold

Within the community, only peers whose reputation is at or above the
community's own statistical floor may vote:

```
threshold = max(0.0, mean(reputation) − σ_factor × stddev(reputation))
```

where the statistics are computed over all peers in the local list and
`σ_factor` is configurable via `SWEFT_REPUTATION_VOTE_SIGMA_FACTOR`
(default `1.0`).

Key properties:

- When all peers have the same reputation (stddev = 0) — as in a brand-new
  community — the threshold equals the mean, so every peer can vote. This
  is the correct behaviour for new or homogeneous communities: the system
  does not artificially exclude anyone until the community develops enough
  heterogeneity to define an "outlier".
- In a healthy, diverse community (~1σ below mean), roughly the bottom 16%
  of peers lose voting rights. Whether 90% of the population falls above
  the threshold in a self-congratulating community is an emergent property
  of that community's choices — the protocol does not second-guess it.
- The threshold tracks the community's own distribution, not an external
  absolute value.

#### Weighted update

A qualifying vote is not applied as a direct override. Instead, it is blended
with the current value using the caller's reputation as the weight:

```
new_rep(target) =
    current_rep(target) × (1 − caller_rep) +
    proposed_rep        × caller_rep
```

This is the same EigenTrust update rule used for cross-node propagation in
Phase 2b, now applied to direct `PATCH` calls. A caller with reputation 1.0
overrides the current value entirely; a caller at the voting threshold barely
moves the needle.

#### Self-update prohibition

A node cannot adjust its own reputation. Requests where the `PATCH` target DID
matches the receiving node's own `node_id` are rejected with `403 Forbidden`,
regardless of who is calling.

### Phase 2b — Weighted claim reconciliation (future)

When Node A receives a peer list from Node B that includes `(C, reputation_B_of_C)`:

```
local_reputation(C) = weighted_merge(
    local_reputation(C),          // what A already knows about C
    reputation_B_of_C,            // what B says about C
    weight = local_reputation(B)  // how much A trusts B's opinion
)
```

The merge function is a weighted average:

```
new_reputation(C) =
    (local_reputation(C) * (1 - local_reputation(B))) +
    (reputation_B_of_C  * local_reputation(B))
```

This is the EigenTrust update rule adapted for a two-party exchange. Its key
properties are:

- A node with `reputation(B) = 1.0` ("B is infallible to me") will adopt B's
  opinion of C directly.
- A node with `reputation(B) = 0.0` ("B is untrustworthy") will ignore B's
  claim entirely.
- The neutral default `0.5` gives equal weight to existing knowledge and the
  new claim — a safe starting point before any track record is established.

#### Reputation adjustment triggers (examples, not exhaustive)

| Event | Effect |
|-------|--------|
| Successful contact (verify, sync, or peer exchange) | Small positive adjustment (+0.02) |
| Contact failure (timeout, connection refused) | Small negative adjustment (−0.05) |
| Node ID mismatch on `/.well-known/semanticweft` | Large negative adjustment (−0.30) |
| Peer recommended a bad node (its claim was wrong) | Propagate penalty to recommender |
| Node stays consistently reachable over 30 days | Positive adjustment (+0.10) |

The magnitude of adjustments is configurable per deployment and will be
specified in a future sub-ADR.

#### Convergence properties

- Because reputation adjustments are bounded to `[0.0, 1.0]` and applied as
  weighted averages, the system is numerically stable.
- In the absence of any signals, reputation drifts toward the network mean
  (a desirable pull toward the centre from the neutral default).
- A Sybil cluster (many fake nodes all vouching for each other) cannot inflate
  reputation beyond the reputation of whichever legitimate node first endorsed
  any member of the cluster, because each endorsement is weighted by the
  endorser's own reputation.

### Phase 3 — Signed reputation claims (future)

In Phase 2, reputation claims in peer lists are unsigned and could be forged.
Phase 3 introduces optional signing of the `reputation` field:

- Nodes sign the tuple `(target_node_id, reputation, timestamp)` with their
  Ed25519 private key (see ADR-0001, ADR-0002).
- Receivers that support signing can verify the claim and weight it more
  heavily than unsigned claims.
- Unsigned claims are still accepted but given a lower trust multiplier.

This creates a graceful upgrade path: nodes that do not yet implement Phase 3
continue to participate in the network, but their reputation claims carry less
weight with Phase-3-capable peers.

---

## Schema implications

The `PeerInfo` struct and `peers` storage table introduced for Phase 1 are
designed to accommodate all three phases without schema changes:

```rust
pub struct PeerInfo {
    pub node_id:    String,          // stable DID key
    pub api_base:   String,          // current routable URL
    pub reputation: f32,             // [0.0, 1.0]; default 0.5
    pub last_seen:  Option<String>,  // ISO 8601; updated on contact
}
```

Phase 2 adds no new columns (the computation is in application logic).
Phase 3 adds an optional `reputation_sig: Option<String>` field to the
wire format only — existing peers without the field are handled gracefully
via `#[serde(skip_serializing_if)]`.

---

## Consequences

- The reputation field is present but inert in Phase 1, giving operators
  a stable schema to build tooling against.
- Eviction by reputation is live in Phase 1 (lowest reputation is evicted
  first), creating an incentive for well-behaved nodes to maintain good
  standing even before adjustment logic exists.
- Phase 2 can be shipped as a feature flag without protocol changes.
- The weighted-merge formula is stateless per exchange — no distributed
  consensus is required.
- Nodes with restrictive peer caps benefit most from reputation: their
  limited slots converge to the most reliable peers over time.
