# Architecture Decision Records

This directory captures significant design decisions made during the development
of the SemanticWeft protocol. Each record documents the context that made the
decision necessary, the options that were available, and the reasoning behind
the choice made.

ADRs are numbered sequentially. A decision is never deleted; if it is superseded
by a later decision, its status is updated to reflect that.

## Index

| # | Title | Status |
|---|-------|--------|
| [0001](./0001-did-method-selection.md) | DID Method Selection | Accepted |
| [0002](./0002-signature-scheme.md) | Signature Scheme | Accepted |
| [0003](./0003-trust-model.md) | Trust Model: In-band vs. Out-of-band | Accepted |

## Format

Each ADR uses the following sections:

- **Status** — Proposed / Accepted / Superseded by ADR-NNNN / Deprecated
- **Date** — ISO 8601 date the decision was recorded
- **Context** — The situation or problem that made a decision necessary
- **Options Considered** — The alternatives evaluated, with brief pros/cons
- **Decision** — The choice made and the primary reasons for it
- **Consequences** — What changes as a result; follow-on work required
