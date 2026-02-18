# ADR-0001: DID Method Selection

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

SemanticWeft units carry an `author` field that identifies the agent that
created the unit. For unit signatures (Phase 3) to be verifiable, `author`
must be a resolvable identifier that yields a public key. Decentralized
Identifiers (W3C DID Core 1.0) are the natural choice: they avoid dependence
on any central registry and map cleanly to cryptographic keys.

There are dozens of DID methods. The choice of method determines:
- **Infrastructure burden** — what the signing agent must operate
- **Resolution complexity** — how a verifying agent recovers the public key
- **Offline / air-gap support** — whether verification requires a network call
- **Key rotation** — whether an agent can change keys without changing identity

The practical candidates for SemanticWeft's initial scope are:

| Method | Resolution | Key rotation | Infrastructure |
|--------|------------|-------------|----------------|
| `did:key` | Algorithmic (no network) | No (new key = new DID) | None |
| `did:web` | HTTPS GET to a well-known URL | Yes | HTTPS + DNS |
| `did:peer` | Bilateral exchange | Yes | None |
| `did:ethr`, `did:ion` | Blockchain query | Yes | Blockchain node / gateway |

---

## Options Considered

### Option A — `did:key` only

The DID encodes the public key directly in the identifier string using
multibase + multicodec. Resolution is a pure local computation: no network
call, no infrastructure, no registry.

**Pros:**
- Zero infrastructure. Any process can generate a valid DID in microseconds.
- Units are self-contained: a verifier needs only the unit itself and the DID
  string to check the signature.
- Works fully offline and in air-gapped environments.
- Simplest possible implementation surface in the core library.

**Cons:**
- No key rotation. If a key is compromised, the agent must move to a new DID.
  All prior units under the old DID remain attributed to it — for better and
  for worse (history is immutable, which is also a property of the protocol).
- Not suitable for long-lived institutional identities where key rotation is
  operationally expected.

### Option B — `did:web` only

The DID resolves to a JSON document hosted at a well-known HTTPS path under
a domain the agent controls.

**Pros:**
- Human-readable DIDs (`did:web:example.com`).
- Key rotation without changing DID.
- Familiar to anyone who manages DNS and HTTPS certificates.

**Cons:**
- Requires HTTPS infrastructure. Agents running locally or in embedded
  contexts cannot easily publish a DID document.
- Introduces a live network dependency into signature verification.
- A domain lapsing or a webserver going down breaks resolution of all past
  signatures issued under that DID — a significant long-term durability risk.

### Option C — `did:key` required, `did:web` recommended for organisations

Mandate `did:key` as the minimum that every implementation must support.
Declare `did:web` as a SHOULD-support extension for implementations that
serve organizational / institutional agents. Other methods are MAY.

---

## Decision

**Option C is adopted.**

`did:key` is the required DID method for SemanticWeft. The `author` field in
a signed unit MUST contain a `did:key` identifier (or a method that resolves
without requiring network access to verify a given unit). `did:web` is a
SHOULD-support for nodes that want to host organisational identifiers.

### Key format

The `did:key` multicodec prefix for Ed25519 is `0xed01`. The resulting DID
looks like:

```
did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuias8siQmsDNyZCeT
```

The fragment identifier (`#z6Mk...`) in a verification method URL is the
same multibase string, following the `did:key` specification.

### Key rotation

Key rotation is handled at the protocol level, not the DID method level:

1. The compromised or superseded agent issues a **`challenge`** unit
   referencing the units whose provenance should be reconsidered, with
   `content` explaining the rotation.
2. The agent issues a **`constraint`** unit declaring "units attributed to
   `did:key:OLD` issued after `<timestamp>` should be treated as if authored
   by `did:key:NEW`" (or discarded, depending on the situation).
3. Verifiers that encounter units from an agent with a documented rotation
   chain can traverse it to determine how much weight to give prior units.

This keeps the trust reasoning inside the graph where it is transparent
and traversable, rather than hidden in resolver metadata.

---

## Consequences

- The `author` field in the spec MUST document that its value SHOULD be a DID
  and that `did:key` is the required method for signed units.
- The core library will need a helper that generates a fresh `did:key` from an
  Ed25519 public key (deferred to Phase 3 implementation work).
- The signature verification path in the core library need not make any network
  calls for `did:key` authors.
- A future ADR (post-Phase 3) should revisit `did:web` support once the signing
  implementation exists and organisational use cases are better understood.
- `did:peer` and ledger-anchored methods are explicitly out of scope until
  there is a demonstrated use case that `did:key` + `did:web` cannot address.
