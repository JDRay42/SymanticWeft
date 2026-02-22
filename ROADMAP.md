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

## Phase 1: Schema Specification ✓

*Define the Semantic Unit formally enough that any two independent implementations can exchange units and agree on their meaning.*

- [x] **JSON Schema for Semantic Units** — `spec/schema/unit.schema.json` (JSON Schema 2020-12)
- [x] **Field semantics document** — `spec/semantic-unit.md` normative specification
- [x] **Unit type definitions** — five types (`assertion`, `question`, `inference`, `challenge`, `constraint`) with formal reference rules; five relationship types (`supports`, `rebuts`, `derives-from`, `questions`, `refines`)
- [x] **Versioning and extension model** — `x-` prefix namespacing for extensions (ADR-004)
- [x] **Formal specification document** — `spec/semantic-unit.md` is the normative reference

**Decisions made:** `confidence` is an optional 0–1 float; `references` are typed with explicit `rel` fields; extensions use `x-<namespace>-<field>` namespacing.

---

## Phase 2: Reference Implementation ✓

*A library that makes it easy to build compliant agents, and validates that the schema is implementable.*

- [x] **Core library** — `packages/core` (`semanticweft` crate): types, validation, serialization, JCS canonicalisation
- [x] **Graph traversal** — in-memory unit graph with ancestor/descendant traversal and type filtering
- [x] **Human-readable rendering** — `packages/core/src/render.rs` generates natural language summaries from unit graphs
- [x] **Validation CLI** — `sweft validate` checks units against the spec; `sweft render` produces human-readable output

**Language: Rust.** Distributed as `sweft` (CLI) and the `semanticweft` crate. WebAssembly bindings in `packages/wasm`.

---

## Phase 3: Identity and Trust ✓

*Make unit authorship verifiable. An assertion is only as useful as the trust placed in its source.*

- [x] **Agent identity model** — `did:key` (Ed25519) for agents and nodes; self-contained, no central registry required (ADR-0001)
- [x] **Unit signing** — Ed25519 signatures over JCS-canonicalised unit JSON; `proof` field in unit schema
- [x] **Verification** — public key embedded in `did:key`; verification is local and requires no network lookup
- [x] **Trust levels** — peer reputation system tracks node reliability scores; community-gated voting with statistical threshold and weighted EigenTrust-style updates implemented (ADR-0008)
- [x] **Revocation** — units are immutable; a `challenge` unit with `rebuts` reference is the protocol-native retraction mechanism

**Decisions made:** `did:key` chosen for self-containment (ADR-0001); Ed25519 chosen for performance and widespread support (ADR-0002); trust expressed via peer reputation scores, not in-band unit fields (ADR-0003).

---

## Phase 4: Transport and Federation ✓

*Define how units move between agents and nodes, and how the graph grows across a network.*

- [x] **Node API specification** — `spec/node-api.md`: HTTP/REST endpoints with pagination and filtering (ADR-0004)
- [x] **Unit submission protocol** — Ed25519 HTTP Signatures for authenticated agent requests
- [x] **Graph sync** — pull-based federation with SSE streaming for live updates; eventual consistency model (ADR-0005, ADR-0009)
- [x] **Node discovery** — bootstrap peer list with automatic peer table propagation (ADR-0010)

**Decisions made:** REST/HTTP with SSE for streaming (ADR-0004); eventual consistency (ADR-0005); rate limiting per client IP without central authority (ADR-0006); pull-based federation with push for `network` visibility fan-out (ADR-0009).

---

## Phase 5: Node Hosting ✓

*Make it practical to run a node, so the network can actually exist.*

- [x] **Storage specification** — SQLite with bundled driver; defined storage trait allows alternative backends
- [x] **Reference node implementation** — `sweft-node` binary passes the conformance suite; persistent and in-memory modes
- [x] **Docker infrastructure** — multi-stage `Dockerfile` and `docker-compose.yml` for one-command deployment
- [x] **Self-hosting guide** — [`docs/guides/node-operator.md`](docs/guides/node-operator.md) covers installation, configuration, federation, security, and troubleshooting
- [x] **Operational requirements** — documented in the operator guide (hardware minimums, uptime expectations, data retention guidance)

---

## Phase 6: Ecosystem and Governance

*Ensure the protocol can evolve without fragmentation, and that no single party controls it.*

- [ ] **Conformance test suite** — a machine-readable set of tests that any implementation can run to verify compliance; refactor `packages/conformance` into a standalone binary that tests any HTTP endpoint (removing its current dependency on `semanticweft-node`)
- [ ] **Protocol versioning policy** — how breaking vs. non-breaking changes are classified and communicated
- [ ] **Extension registry** — a public registry of namespaced extensions so the community can coordinate without requiring core spec changes
- [ ] **Governance model** — how decisions about the protocol are made; who has standing; how forks are handled
- [ ] **Trademark policy** — define what may be called "SemanticWeft-conformant"; CC BY-SA 4.0 protects the spec text from proprietary forks, but trademark is the mechanism that controls use of the name and conformance badge

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

Phases 0–5 are complete. The network is runnable: anyone can stand up a node with `docker compose up`, register agents, submit units, and federate with peers.

The immediate priority is **Phase 6: Ecosystem and Governance** — the infrastructure needed for the protocol to evolve safely without fragmenting. Key open items:

- A machine-readable conformance suite that third-party implementations can run to certify compliance.
- A protocol versioning policy that distinguishes breaking from non-breaking changes.
- An extension registry so the community can coordinate on namespace additions without requiring core spec changes.
- A governance model that keeps control distributed and prevents any single operator from steering the protocol.
- A trademark policy for the "SemanticWeft" name and conformance badge, complementing the CC BY-SA protection on the spec (see ADR-0012).
