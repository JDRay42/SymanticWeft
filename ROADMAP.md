# SemanticWeft Roadmap

This document maps the full development arc of the SemanticWeft protocol — from its current early-design stage to a federated, interoperable ecosystem. It is a living document. Phases may shift as decisions are made and collaborators weigh in.

---

## Phase 0: Foundation ✓

*Establish what the project is and why it exists.*

- [x] Problem statement
- [x] Design principles
- [x] Semantic Unit concept and type taxonomy
- [x] ID format: UUIDv7 (RFC 9562)

---

## Phase 1: Schema Specification

*Define the Semantic Unit formally enough that any two independent implementations can exchange units and agree on their meaning.*

- [ ] **JSON Schema for Semantic Units** — required fields, types, constraints
- [ ] **Field semantics document** — what each field means, what values are valid, edge cases
- [ ] **Unit type definitions** — formal semantics for `assertion`, `question`, `inference`, `challenge`, `constraint`; rules for valid references between types
- [ ] **Versioning and extension model** — how the schema evolves without breaking existing units; how implementers add fields without conflicting
- [ ] **Formal specification document** — a human-readable spec (Markdown or RFC-style) that is the normative reference for all implementations

**Open decisions:**
- Which fields are required vs. optional? (`confidence`, `assumptions`, `source` are present in the example but their optionality is unspecified)
- What is the valid range and interpretation of `confidence`? (0–1 float? Enum? Required?)
- Are `references` typed? (Does a unit declare *why* it references another — support, rebuttal, derivation?)
- How are extensions namespaced to avoid collisions?

---

## Phase 2: Reference Implementation

*A library that makes it easy to build compliant agents, and validates that the schema is implementable.*

- [ ] **Core library** — create, validate, serialize, and deserialize Semantic Units
- [ ] **Graph traversal** — navigate a local collection of units: ancestors, descendants, subgraphs by type
- [ ] **Human-readable rendering** — generate natural language summaries from unit graphs (the "expression layer")
- [ ] **Validation CLI** — check a unit or a graph of units against the spec

**Language: Rust.** The reference implementation is written in Rust. Reasons: native binaries with no runtime dependency, first-class WebAssembly compilation (one crate, every ecosystem), and a type system that makes protocol correctness hard to undermine accidentally. Distributed as `sweft` (CLI) and the `semanticweft` crate.

**Open decisions:**
- Packaging and distribution strategy (crates.io for the library; GitHub Releases for pre-built CLI binaries; WASM build for browser/embedding use)

---

## Phase 3: Identity and Trust

*Make unit authorship verifiable. An assertion is only as useful as the trust placed in its source.*

- [ ] **Agent identity model** — how agents are identified; likely [Decentralized Identifiers (DIDs)](https://www.w3.org/TR/did-core/) to avoid dependence on any central registry
- [ ] **Unit signing** — cryptographic signature attached to units at creation; schema extension for signature fields
- [ ] **Verification** — how a receiving agent verifies a unit's signature and resolves the signing key
- [ ] **Trust levels** — how agents express and reason about varying degrees of trust in sources
- [ ] **Revocation** — how an agent can retract or supersede a previously issued unit

**Open decisions:**
- DID method(s) to support (did:web is simple; did:key is self-contained; others add complexity)
- Signature scheme (Ed25519 is the practical default)
- Whether trust is expressed in-band (in unit fields) or out-of-band (separate trust assertions)

---

## Phase 4: Transport and Federation

*Define how units move between agents and nodes, and how the graph grows across a network.*

- [x] **Node API specification** — HTTP endpoints for submitting and retrieving units; pagination; filtering by type, source, reference
- [x] **Unit submission protocol** — how an agent publishes a unit to a node
- [x] **Graph sync** — how nodes replicate units from each other; push vs. pull; conflict model
- [x] **Node discovery** — how agents and nodes find each other; bootstrapping

**Open decisions:**
- Transport mechanism (REST/HTTP is the baseline; WebSockets or SSE for streaming; ActivityPub-style federation is worth evaluating)
- Consistency model for federated graphs (eventual consistency is the practical answer, but the implications need spelling out)
- Rate limiting and spam prevention without a central authority

---

## Phase 5: Node Hosting

*Make it practical to run a node, so the network can actually exist.*

- [ ] **Storage specification** — what a node must be able to store and index; no prescribed database, but defined requirements
- [ ] **Reference node implementation** — a runnable node that passes the conformance suite
- [ ] **Self-hosting guide** — documentation for running a node without deep protocol knowledge
- [ ] **Operational requirements** — minimum hardware, uptime expectations, data retention policies

---

## Phase 6: Ecosystem and Governance

*Ensure the protocol can evolve without fragmentation, and that no single party controls it.*

- [ ] **Conformance test suite** — a machine-readable set of tests that any implementation can run to verify compliance
- [ ] **Protocol versioning policy** — how breaking vs. non-breaking changes are classified and communicated
- [ ] **Extension registry** — a public registry of namespaced extensions so the community can coordinate without requiring core spec changes
- [ ] **Governance model** — how decisions about the protocol are made; who has standing; how forks are handled

---

## Repository Structure (target)

```
/
├── ROADMAP.md           # This document
├── README.md            # Project overview
├── spec/
│   ├── semantic-unit.md # Normative specification
│   └── schema/
│       └── unit.schema.json
├── docs/
│   ├── decisions/       # Recorded design decisions (ADRs)
│   └── guides/
├── packages/
│   └── core/            # Reference implementation library
├── node/                # Reference node implementation
└── conformance/         # Conformance test suite
```

---

## What's Next

The immediate priority is **Phase 1**. The schema must be nailed down before anything else can build on it — the reference implementation, the trust layer, and the transport protocol all depend on a stable unit structure.

Start with the field semantics: enumerate every field, decide what's required, and make the hard calls on `confidence`, typed references, and extension namespacing. The spec document follows from that.
