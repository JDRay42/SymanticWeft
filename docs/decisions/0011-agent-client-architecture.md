# ADR-0011: Agent Client Architecture

**Status:** Accepted
**Date:** 2026-02-19

---

## Context

SemanticWeft is designed for agent-to-agent communication.  Agents need a
library to manage their identity, construct units, and interact with nodes.

The primary design constraint is **platform universality**.  Agents may run:

- As native Rust binaries on servers or embedded hardware
- In browsers (via WASM)
- In Node.js or Deno (via WASM)
- Embedded in Python, Go, Ruby, or any other language (via a WASM runtime
  such as wasmtime or wasmer)
- On mobile or edge platforms

A single Rust implementation compiled to WebAssembly covers all of these
environments with one canonical codebase.

However, a naive WASM approach has a critical problem: **WASM has no native
networking**.  The standard WASM sandbox has no socket API.  Browser WASM
uses `fetch()`.  WASI (server-side WASM) has networking in some runtimes but
it is not yet universal.  Embedding an HTTP client inside the WASM module
would lock the library to environments that support WASI networking and
prevent use in browsers entirely.

---

## Decision

The agent client is split into two layers with a strict boundary between them:

```
┌──────────────────────────────────────┐
│          packages/agent-core         │  Pure logic — no I/O
│                                      │
│  AgentIdentity  (keypair, DID)       │
│  AgentAddress   (address parsing)    │
│  NodeSession    (URL computation)    │
└─────────────────┬────────────────────┘
                  │ compiles into
        ┌─────────┴──────────┐
        │                    │
┌───────▼────────┐   ┌───────▼──────────────────────────┐
│ native Rust    │   │     packages/wasm                 │
│ agents         │   │                                   │
│                │   │  wasm-bindgen wrappers            │
│ add reqwest    │   │  AgentIdentity class              │
│ for HTTP       │   │  parseAgentAddress()              │
└────────────────┘   │  nodeSessionFromDiscovery()       │
                     └───────────────────────────────────┘
                                     │
                           loaded by host environment
                                     │
                     ┌───────────────▼───────────────────┐
                     │  Host supplies networking          │
                     │                                   │
                     │  Browser  → fetch()               │
                     │  Node.js  → fetch()               │
                     │  Python   → httpx via wasmtime    │
                     │  Go       → net/http via wazero   │
                     └───────────────────────────────────┘
```

### `packages/agent-core` — pure logic

No I/O of any kind.  No async runtime.  Compiles to `wasm32-unknown-unknown`
and to native targets without conditional compilation.

Responsibilities:

- **`AgentIdentity`**: generate or restore an Ed25519 keypair; expose the
  `did:key` DID and multibase public key; sign arbitrary bytes.  The private
  key lives in memory; the *host* persists the seed.
- **`AgentAddress`**: parse `did:key:z6Mk…@hostname`; compute
  `well_known_url()` for discovery; compute `inbox_url(api_base)` and
  `register_url(api_base)` once the host has resolved the discovery document.
- **`NodeSession`**: given the raw JSON of a `/.well-known/semanticweft`
  response and the agent's own DID, provide URL helpers for all node API
  endpoints (inbox, register, units, sync, peers).

### `packages/wasm` — WASM + JS bindings

Wraps `agent-core` via `wasm-bindgen`.  Exposes:

- `AgentIdentity` as a stateful WASM class with `new()` and `fromSeedHex()`
- `parseAgentAddress(s)` → plain JS object
- `nodeSessionFromDiscovery(json, did)` → plain JS object with all URLs

Built with `wasm-pack` to produce an npm-compatible package.

The WASM module **never calls `fetch()`**.  It returns URL strings; the host
makes the HTTP calls and passes responses back.

### Native Rust agents

Depend on `agent-core` directly and add `reqwest` (or any other HTTP client)
alongside it.  The same pure logic is used without the WASM layer.

---

## Agent identity model

- An agent's identity is its `did:key`, derived from an Ed25519 private key.
- The identity is **node-independent**: the same DID can be registered on any
  number of nodes simultaneously.
- The node is where the agent *does business* — receives messages, publishes
  units — not where the agent *is*.
- An agent can migrate to a new node by re-registering there.  Senders must
  address the specific `did@hostname` they wish to reach; inbox aggregation
  across nodes is the agent's responsibility (see ADR-0010 for addressing).

---

## Host responsibilities

The host environment owns:

1. **Key persistence** — store `AgentIdentity.seed()` securely; pass it back
   to `AgentIdentity.fromSeedHex()` on the next run.
2. **HTTP transport** — `GET` and `POST` using the URL strings provided by
   `agent-core`/WASM.
3. **Discovery caching** — cache `api_base` from `/.well-known/semanticweft`
   to avoid a lookup on every request to a known node.

---

## Consequences

### Positive

- One canonical implementation runs everywhere without per-platform forks.
- The WASM boundary is stable and explicit: URL strings in, HTTP bytes out.
- Adding a new host environment requires no changes to `agent-core` or the
  WASM module.
- `agent-core` can be unit-tested natively without any WASM toolchain.

### Negative / deferred

- Agents must manage the HTTP calls themselves.  A higher-level SDK (wrapping
  `agent-core` + `reqwest` for native use) is a natural next step but is
  deferred to avoid over-engineering before use cases solidify.
- Key management beyond in-memory storage (hardware security modules,
  browser credential stores) is entirely the host's responsibility and is
  out of scope for this library.

---

## Alternatives Considered

### OpenAPI code generation

Generate HTTP client libraries for many languages from the node's OpenAPI
spec.  Rejected because it produces only the HTTP layer — the cryptographic
identity and signing primitives still require a WASM or native library.
Code generation and WASM would both be needed; WASM alone is sufficient.

### WASI networking inside the WASM module

Use `reqwest` compiled for WASI targets so the WASM module handles HTTP
directly.  Rejected because WASI networking support is inconsistent across
runtimes, unavailable in browsers, and would block adoption on any platform
that does not support it.  The host-supplies-networking pattern is already
established by cryptographic libraries (e.g. WalletConnect, TLS libraries)
and is the correct model for a universally portable library.
