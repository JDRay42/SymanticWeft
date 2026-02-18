# ADR-004: Extension Namespacing

**Date:** 2026-02-18
**Status:** Accepted

---

## Context

The core Semantic Unit schema cannot anticipate every use case. Implementers will need to attach domain-specific metadata to units — processing hints, application-level tags, proprietary fields — without breaking interoperability with implementations that don't know about those fields.

The extension mechanism must:

1. Allow implementers to add fields without conflicting with the core schema or with each other
2. Allow receivers to safely ignore unknown extensions
3. Not require a central registry to operate

Options considered:

1. **Reverse-domain prefix (`x-com.example.myfield`)** — a naming convention familiar from MIME types and other protocols; no registry needed
2. **Top-level `extensions` object** — all custom fields live under a single reserved key, namespaced within it
3. **Registry-assigned short names** — extensions get short names from a central registry (deferred to Phase 6)
4. **No extensions** — the schema is closed until a formal extension process is established

## Decision

**Extension fields use a reverse-domain prefix.** Specifically:

- Extension field names MUST begin with `x-` followed by a reverse-domain namespace (e.g. `x-com.example.myfield`).
- Extension field names MUST be lowercase.
- The domain used in the namespace SHOULD be a domain the implementer controls.
- Receivers that do not recognize an extension field MUST ignore it and MUST NOT reject the unit on that basis.
- Implementations MUST preserve extension fields when storing or forwarding units.

## Rationale

**Reverse-domain over top-level `extensions` object:** A dedicated `extensions` key is cleaner to read but has a subtle problem: the keys within `extensions` still need namespacing to avoid collisions between different implementers. Two implementers could both add `extensions.priority` with different semantics. The reverse-domain approach pushes the namespace into the field name itself, making the owner immediately visible.

**No registry required now:** A registry is appropriate once the protocol is stable and a community of implementers exists. Requiring a registry at Phase 1 would create a bureaucratic bottleneck for experimentation. The reverse-domain convention provides effective collision avoidance without coordination overhead.

**`x-` prefix:** The `x-` prefix signals "this is an extension" to any reader. It prevents extensions from shadowing future core fields (which will never start with `x-`). This follows MIME type conventions (e.g., `x-custom-content-type`).

**Lowercase requirement:** Consistent casing prevents `x-com.Example.Field` and `x-com.example.field` from being treated as different keys by case-sensitive implementations.

**Extension field values are unconstrained by this specification.** The value of an extension field may be any valid JSON value. The extension's author is responsible for documenting its schema.

## Consequences

- Implementers can add fields immediately without coordination.
- Collision-free namespacing depends on implementers using domains they control; there is no enforcement mechanism at the protocol level.
- The `x-` prefix is reserved; the core schema will never add a field beginning with `x-`.
- A future extension registry (Phase 6) may assign short canonical names for widely-adopted extensions, but the reverse-domain mechanism will remain valid.
- JSON Schema validation allows extension fields via `patternProperties` on `^x-[a-z0-9]+(\.[a-z0-9]+)+` combined with `additionalProperties: false` for all other unrecognized keys.
