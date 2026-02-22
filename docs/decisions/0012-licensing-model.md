# ADR-0012: Licensing Model — Specification, Implementation, and Conformance Suite

**Status:** Accepted
**Date:** 2026-02-22

---

## Context

SemanticWeft consists of three distinct artifacts that serve different audiences
and have different downstream use patterns:

1. **The protocol specification** (`spec/`) — normative documents that define
   the Semantic Unit format and the Node API. The audience is *anyone* who
   wants to implement the protocol: researchers, companies, AI providers, and
   independent developers in any language.

2. **The reference implementation** (`packages/` — core, cli, agent-core, wasm,
   node-api, node) — the canonical Rust implementation of the protocol. This is
   production software intended to be run as a network server.

3. **The conformance suite** (`packages/conformance/`) — a test harness that
   verifies whether a node implementation conforms to the specification. The
   audience is implementers who want to validate their own nodes.

All three previously declared `license = "MIT"` in Cargo metadata, while the
root `LICENSE` file was AGPL-3.0. This was inconsistent: a downstream user
reading `Cargo.toml` had a reasonable expectation of MIT while the actual
license file said AGPL.

The root design principle of SemanticWeft is **neutral ground**: no single AI
provider or organisation controls the protocol. For the ecosystem goal to be
realised, the spec must be implementable by anyone without legal friction, while
the reference implementation retains copyleft to ensure modifications are
shared.

---

## Options Considered

### Specification licensing

| Option | Effect |
|--------|--------|
| AGPL-3.0 (previous de-facto) | Wrong category — AGPL is a software license; applying it to a document creates implementer uncertainty about whether their code must be AGPL |
| MIT / Apache 2.0 | Permissive; anyone may fork and proprietary-license a modified spec, risking ecosystem fragmentation |
| CC BY 4.0 | Standard for open specs (W3C); permissive; attribution required; forks may be any license |
| **CC BY-SA 4.0** | Copyleft for documents; forks of the spec must also be CC BY-SA; implementations are unrestricted |
| Community Specification License | Purpose-built for multi-vendor specs; overkill at this stage |

### Implementation licensing

| Option | Effect |
|--------|--------|
| MIT | Permissive; modified node implementations need not share source |
| Apache 2.0 | Permissive with patent grant; same concern as MIT |
| **AGPL-3.0-only** | Copyleft for network services; operators of modified nodes must publish source |

### Conformance suite licensing

| Option | Effect |
|--------|--------|
| AGPL-3.0 | Technically correct given node dependency, but signals "implementers must be AGPL" — chilling effect on adoption |
| MIT | Permissive; no patent grant |
| **Apache 2.0** | Permissive with explicit patent grant; industry-standard for protocol conformance tooling |

---

## Decision

### Specification: CC BY-SA 4.0

The `spec/` directory is licensed under Creative Commons Attribution-ShareAlike
4.0 International. See `spec/LICENSE`.

This means:
- Anyone may implement the protocol in any language under any license without
  restriction. The CC license governs only the spec *text*, not implementations.
- Anyone may share, translate, and annotate the spec freely with attribution.
- Modified or extended versions of the spec itself must be released under CC BY-SA.
  This prevents a vendor from capturing the spec and releasing an incompatible
  proprietary variant.

### Reference implementation: AGPL-3.0-only

All packages under `packages/` (except the conformance suite) are licensed
under AGPL-3.0-only, consistent with the root `LICENSE` file. The Cargo
metadata in each package now reflects this correctly.

AGPL ensures that any operator who modifies and runs the reference node server
must publish their modifications — the "network use is distribution" clause
applies.

### Conformance suite: Apache-2.0

`packages/conformance/` is licensed under the Apache License, Version 2.0.
See `packages/conformance/LICENSE`.

This signals clearly that anyone may run, fork, embed, and redistribute the
conformance suite without copyleft obligation. The explicit patent grant in
Apache 2.0 is meaningful here: contributors to the conformance tests grant
patent rights to passing implementers, providing confidence that a "conformant"
badge does not carry hidden patent risk.

**Architectural note:** The conformance suite currently links against
`semanticweft-node` (AGPL). As an internal `publish = false` test harness, this
causes no distribution concern. Phase 6 should refactor the suite into a
standalone binary that accepts a target URL and tests any node over HTTP. At
that point, the Apache 2.0 license will apply cleanly with no AGPL linkage.

---

## Summary

| Artifact | License | Rationale |
|----------|---------|-----------|
| `spec/` | CC BY-SA 4.0 | Standard for open protocol specs; prevents proprietary spec forks; implementations unrestricted |
| `packages/` (core, cli, agent-core, wasm, node-api, node) | AGPL-3.0-only | Copyleft for network service; operators must share modifications |
| `packages/conformance/` | Apache-2.0 | Maximally permissive with patent grant; standard for conformance tooling |

---

## Consequences

- The Cargo `license` fields in all packages now match the actual intent,
  eliminating the MIT/AGPL ambiguity.
- Third-party implementers in any language can implement the protocol
  without legal concern about the spec license.
- The conformance suite can be bundled, embedded, or hosted by third-party
  certification services without license friction.
- Phase 6 governance work should formalise trademark policy for the
  "SemanticWeft" name and conformance marking, complementing the CC BY-SA
  protection on the spec.
