# ADR-0005: Consistency Model for Federated Graphs

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

A SemanticWeft network consists of multiple independent nodes. Each node
accepts units from agents, stores them locally, and replicates them to and
from peer nodes. Because there is no central coordinator, different nodes will
hold different subsets of units at any given moment.

The protocol must define what consistency guarantees it makes about the
federated graph, and what that means for agents and nodes in practice.

Key structural properties of Semantic Units relevant to consistency:

1. **Immutability.** A unit's `id`, `type`, `content`, and all other fields
   are fixed at creation. A unit with a given `id` has identical content on
   every node that holds it.
2. **UUIDv7 IDs.** The time component of a UUIDv7 provides a globally
   consistent creation order that does not require coordination between nodes.
3. **Forward references.** A unit MAY reference a unit whose `id` is not
   yet in a given node's local graph. The spec explicitly requires receivers
   not to reject such units.

---

## Options Considered

### Option A — Strong consistency (linearisability)

All nodes agree on a total order of writes. A unit is "committed" only once a
quorum of nodes has acknowledged it. Reads always reflect the latest state.

**Pros:**
- Clients always see a consistent view of the graph.

**Cons:**
- Requires a consensus protocol (Raft, Paxos, or similar). This implies
  a designated cluster of nodes with coordinated leadership.
- Conflicts with the goal of a decentralised network where any party can
  run a node independently.
- A network partition makes some writes unavailable until the partition heals.
- Unit immutability eliminates the write conflicts that strong consistency
  is primarily designed to prevent — the cost is borne without the benefit.

### Option B — Causal consistency

Nodes guarantee that if unit B references unit A, any client that sees B
also sees A.

**Pros:**
- Preserves the graph's referential integrity from a client's perspective.
- Stronger than eventual consistency without requiring global coordination.

**Cons:**
- Implementing causal consistency requires nodes to track causal dependencies
  and delay delivery of units whose antecedents have not yet arrived.
- The spec already allows forward references to be missing in a local graph
  (Section 4.4 of the unit spec). Enforcing causal delivery would require
  reversing this design choice.
- Causal tracking introduces per-unit metadata overhead and complicates
  sync considerably.

### Option C — Eventual consistency

All nodes independently accept and store units. Given sufficient time and
replication, all nodes converge to holding the same set of units. No ordering
guarantee is made for the moment of observation; only convergence is guaranteed
asymptotically.

**Pros:**
- No coordination required between nodes.
- Any node can accept any unit from any agent independently.
- A network partition does not block writes — nodes that can still reach
  agents accept units and sync to peers once the partition heals.
- Unit immutability means there are no merge conflicts. Two nodes that both
  hold a unit with id `X` always agree on its content.
- UUIDv7 provides a globally consistent, coordination-free total order
  over all units ever created, enabling deterministic pagination and sync.

**Cons:**
- A client querying two different nodes simultaneously may see different
  subsets of the graph.
- A unit submitted to one node may not be visible on another node for some
  time.

---

## Decision

**Option C (eventual consistency) is adopted.**

The SemanticWeft protocol guarantees eventual consistency and nothing stronger.
Nodes MAY lag behind their peers; this is an expected operating condition, not
an error.

### Normative rules

1. **Write acceptance.** A node MUST accept any valid unit submitted to it,
   regardless of whether the units referenced by that unit are present in
   its local graph.

2. **Immutability preservation.** A node that already holds a unit with id `X`
   MUST NOT replace it with a unit of the same id but different content.
   If a node receives a unit whose id it already holds, and the content is
   identical, it MUST treat the submission as idempotent (return success
   without error).

3. **No phantom deletes.** Units are never deleted from the federated graph.
   Retraction is expressed by creating new units (challenge or constraint)
   that reference the retracted unit. This preserves history and ensures
   that all nodes can eventually converge without needing a delete
   propagation protocol.

4. **Sync primitive.** The sync primitive is cursor-based pull. A node asks
   a peer for units with IDs after a given cursor. Because UUIDv7 IDs embed
   a millisecond timestamp, "after cursor X" is equivalent to "created after
   the moment encoded in X", without requiring wall-clock coordination.

5. **No global epoch.** There is no concept of a "current state" of the
   federated graph. Each node has a local view. Agents that need a more
   complete view can query multiple nodes and union the results.

### Implications for agents

- Agents SHOULD NOT assume that a unit submitted to node A is immediately
  visible on node B.
- Agents that need to reason about a related set of units SHOULD query a
  single node, or accept that their view may be incomplete.
- Agents MUST NOT treat the absence of a referenced unit as evidence that
  the referenced unit does not exist in the global graph.

### Sync convergence

A pair of nodes A and B converge when A holds every unit that B holds and
vice versa. Convergence is achieved by each node periodically pulling from
its known peers. The expected convergence time is bounded by the product of
the longest replication cycle and the network diameter (the maximum number
of hops between any two nodes in the peer graph).

---

## Consequences

- The `GET /v1/sync` endpoint is the primary mechanism for node-to-node
  replication. Its cursor-based design (`?after=<uuid>`) encodes the
  eventual-consistency model directly: a node always knows where it left off.
- Conformance tests (Phase 6) will verify that nodes accept forward-referenced
  units, treat duplicate-id submissions idempotently, and expose a compliant
  sync endpoint.
- A future ADR may address optimistic delivery (nodes pushing to known peers
  immediately on receipt) as a latency optimisation, without changing the
  consistency guarantees.
