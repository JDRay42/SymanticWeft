# SemanticWeft

A semantic communication protocol for AI-to-AI interaction.

---

## The Problem

When AI agents communicate today, they use natural language — the medium designed for humans. That works, but it carries costs that compound at scale:

- **Ambiguity**: Meaning is implicit. What an agent intends and what another infers can diverge silently, with no mechanism to detect or resolve it.
- **No epistemics**: Natural language doesn't express confidence, assumptions, or provenance. An assertion and a guess are syntactically identical.
- **Opaque reasoning structure**: When agents chain through natural language, the structure of reasoning is lost — which premises support which conclusions, where agents agree, where they don't.
- **Forced linearity**: Conversation is a poor model for thought. Meaning is a graph. Transcripts are not.

This isn't a complaint about current AI systems. It's a problem with using a human-facing format as the substrate for machine-to-machine reasoning.

---

## The Concept

SemanticWeft defines a **Semantic Unit** as the primitive of AI communication — not a message, but a typed, attributed, linkable node of meaning.

```json
{
  "id": "019191e4-c9f0-7000-b5f3-3c15d6a8f9b2",
  "type": "assertion",
  "content": "The payment processing requirement implies PCI DSS scope.",
  "confidence": 0.85,
  "assumptions": [
    "user operates commercially",
    "payments include card data"
  ],
  "source": "agent://analyst-1",
  "references": ["019191e4-ab12-7000-a3f1-9d84c2e107f5"]
}
```

Unit IDs are [UUIDv7](https://www.rfc-editor.org/rfc/rfc9562#name-uuid-version-7) (RFC 9562). UUIDv7 is lexicographically sortable by creation time, requires no coordination to generate, and is an IETF standard — making it a natural fit for a decentralized, open protocol. The embedded millisecond timestamp reflects *creation time*, not submission time.

Units have types:

| Type         | Meaning                                             |
|--------------|-----------------------------------------------------|
| `assertion`  | A claim the agent holds to be true                 |
| `question`   | An open issue requiring resolution                 |
| `inference`  | A conclusion derived from referenced units         |
| `challenge`  | A rebuttal or counter to a referenced unit         |
| `constraint` | A boundary condition scoping a problem             |

Units reference each other, forming a **directed graph of meaning** — not a transcript. An agent asserts something. Another challenges it. A third derives an inference from both. All of that structure is preserved, addressable, and traversable. Nothing is lost in the retelling.

---

## Design Principles

**More auditable, not less.**
SemanticWeft is not a back-channel. A typed assertion with explicit confidence and stated assumptions is *more* inspectable than a natural language sentence. The graph is readable by humans and machines alike.

**Meaning and expression are separate.**
Natural language is an optional rendering layer, not the source of truth. Agents exchange structured units. Human-readable summaries are generated from them.

**Neutral ground.**
No frontier AI provider controls this protocol. If any major lab defines the standard, the others won't adopt it — and the ecosystem fragments. An independent, open-source protocol is the only realistic path to cross-provider adoption.

**Messages are stateless. The fabric is not.**
Each unit is self-contained — it carries its context with it. But the graph units form is persistent and accumulative. This is closer to a shared knowledge base than a chat session.

**Designed to outlast current models.**
The protocol should be meaningful between a model built today and one that doesn't exist yet. Extensibility and backward compatibility are first-class concerns, not afterthoughts.

---

## Status

Early-stage protocol design. Current state:

- [x] Problem statement and design principles
- [ ] Formal schema specification
- [ ] Reference implementation
- [ ] Identity and trust layer
- [ ] Node hosting specification

---

## Get Involved

A protocol is only as useful as its second implementer. If you're building AI agents and find yourself working around the limitations of natural language as a coordination medium, this project wants to hear from you.

Looking for:
- Feedback on the semantic unit model
- Collaborators on the schema specification
- Early implementers willing to run a node
