# ADR-0002: Signature Scheme

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

Phase 3 requires that a Semantic Unit can carry a cryptographic proof of
authorship. The proof must be:

- **Verifiable** by any party in possession of the unit and the author's DID
- **Binding** — the signature must cover every field that gives the unit its
  meaning, so that tampering is detectable
- **Compact** — units may be embedded in larger documents or transmitted over
  bandwidth-constrained links
- **Forward-compatible** — the scheme should not preclude adding new fields
  or a second proof (e.g. a notary co-signature) in a later version

Three interlocking decisions are recorded here:

1. The **signature algorithm**
2. The **canonicalization method** (what exact bytes are signed)
3. The **proof envelope** (where the signature lives in the unit JSON)

---

## Options Considered

### Signature algorithm

| Algorithm | Key size | Sig size | Library support | Notes |
|-----------|----------|----------|-----------------|-------|
| Ed25519 / EdDSA | 32 B | 64 B | Excellent | Fast, compact, constant-time |
| ECDSA P-256 / ES256 | 64 B | 64 B | Excellent | Required by many PKI standards |
| ECDSA P-384 / ES384 | 96 B | 96 B | Good | More conservative; overkill here |
| RSA-PSS | 256–512 B | 256–512 B | Excellent | Too large; legacy |

Ed25519 is the right choice: it produces 64-byte signatures, is constant-time
(no side-channel leakage), has first-class support in the Rust ecosystem
(`ed25519-dalek`, `ring`), and maps directly to the `did:key` multicodec
prefix chosen in ADR-0001. P-256 is a reasonable fallback for integrations
with existing PKI, but it is not the primary scheme.

### Canonicalization method

A JSON document can be serialized in many equivalent ways (field order, spacing,
Unicode escaping). To sign a unit, both the signer and the verifier must produce
the same byte sequence from the same logical document.

| Method | Standard | Notes |
|--------|----------|-------|
| JSON Canonicalization Scheme (JCS) | RFC 8785 | Lexicographic key ordering; deterministic; widely implemented |
| JSON-LD Normalisation (URDNA2015) | W3C | Requires JSON-LD processing; complex |
| Serialize-as-received | None | Non-portable; breaks round-trips |

JCS (RFC 8785) is adopted: lexicographic key ordering, no insignificant
whitespace, Unicode escaped as-is. It requires no semantic knowledge of the
document — any conforming implementation can canonicalize a unit.

Before canonicalizing, the `proof` field is **removed** from the object if
present. This ensures the signature is stable regardless of whether a proof
has already been attached.

### Proof envelope

The signature and its metadata must live somewhere in the unit. Options:

**Option A — Top-level `proof` object (spec-defined field)**

```json
{
  "id": "...",
  "type": "assertion",
  "content": "...",
  ...
  "proof": {
    "method": "did:key:z6Mk...#z6Mk...",
    "created": "2026-02-18T06:00:00Z",
    "value": "z..."
  }
}
```

- Clean, human-readable.
- `proof` is a reserved field name in the spec; validation rejects any unit
  that includes `proof` without the correct shape.
- A verifier does not need to scan for an `x-*` field name it might not know.

**Option B — Extension field `x-sw-proof`**

- Works without a schema change (extension fields are already allowed).
- Splits the cryptographic proof into the "extension" namespace alongside
  application-specific data; semantically odd.
- Any implementation that strips unknown `x-*` fields would silently remove
  the proof.

**Option C — Detached JWS (separate document)**

- The unit and its proof travel as two separate objects.
- Breaks the "unit is self-contained" property that makes SemanticWeft
  convenient to use and store.

---

## Decision

### Algorithm

**Ed25519 / EdDSA** is the required signing algorithm. ECDSA P-256 is a
MAY-support for interoperability with existing certificate infrastructure.

### Canonicalization

Units are signed over the **JCS (RFC 8785) canonical serialization** of the
unit object **with the `proof` field removed**. In pseudocode:

```
payload = jcs_serialize(unit_without_proof)
signature = ed25519_sign(private_key, payload)
```

Verification:

```
payload = jcs_serialize(received_unit_without_proof)
ok = ed25519_verify(public_key, payload, proof.value)
```

### Proof envelope

`proof` is a **spec-defined optional top-level field**. Its shape:

```json
"proof": {
  "method":  "did:key:z6MkpTHR...#z6MkpTHR...",
  "created": "2026-02-18T06:00:00Z",
  "value":   "z2vf3jkY..."
}
```

| Field | Type | Description |
|-------|------|-------------|
| `method` | string (DID URL) | Verification method: the DID plus fragment identifying the specific key used |
| `created` | string (ISO 8601) | When the proof was generated; MAY differ from `created_at` if re-signed |
| `value` | string | base58btc-encoded 64-byte Ed25519 signature |

The `value` encoding uses base58btc with the multibase prefix `z` (the same
convention used by `did:key` and W3C Data Integrity).

### Validation rule

`sweft validate` (and the core library's `validate_unit`) will:
- Accept a unit with no `proof` field (signature is optional in Phase 3).
- Accept a unit with a well-formed `proof` field (correct shape, non-empty
  strings; cryptographic verification is opt-in, not the default).
- Reject a unit with a `proof` field that does not match the specified shape.
- Reject any extension field named `proof` (the name is reserved).

Cryptographic verification (actually checking the signature against the key)
is a separate function: `verify_proof(&unit) -> Result<(), ProofError>`. It
is not called by `validate_unit` because it requires key resolution, which
may involve I/O.

---

## Consequences

- `proof` is added to the JSON Schema for Semantic Units as an optional object
  with required sub-fields `method`, `created`, and `value`.
- The `validate_unit` function in `packages/core` gains a new error variant
  for malformed proof objects.
- A new function `verify_proof` (or equivalent) is needed in Phase 3; it
  is deliberately not part of this ADR's implementation scope.
- A helper crate or feature flag (`semanticweft/signing`) will pull in
  `ed25519-dalek` and `jcs` without making them mandatory dependencies
  for consumers that only need validation.
- The spec must document the signing and verification procedures clearly
  enough that a non-Rust implementation can interoperate.
- A future ADR should address **multi-signatures** (e.g. a notary co-signing
  a unit) — the current `proof` object is singular; an array `proofs` field
  is the natural extension.
