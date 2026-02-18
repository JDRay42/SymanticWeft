# ADR-0004: Transport Mechanism

**Status:** Accepted
**Date:** 2026-02-18

---

## Context

Phase 4 defines how Semantic Units move between agents and nodes. The protocol
needs a transport layer that is universally implementable, operationally simple,
and extensible toward streaming and federation use cases.

Candidates discussed in the roadmap:

| Option | Characteristics |
|--------|----------------|
| REST/HTTP | Stateless, universal, well-understood; no persistent connection |
| WebSockets | Bidirectional, stateful; requires connection management |
| SSE (Server-Sent Events) | One-directional streaming over plain HTTP; auto-reconnect |
| ActivityPub | Established federated social protocol; opinionated data model |

The choice affects:
- **Implementation complexity** — what a conformant node must implement
- **Streaming** — whether nodes can push new units to subscribed clients
- **Federation** — how nodes replicate from each other
- **Ecosystem fit** — whether existing libraries and proxies work without modification

---

## Options Considered

### Option A — REST/HTTP only

All interaction is request/response. Clients poll for new units. Nodes replicate
by periodically pulling from peers.

**Pros:**
- No persistent connection state.
- Works through every load balancer, CDN, and reverse proxy.
- Implementable in any language without special libraries.
- Pull-based sync aligns naturally with the eventual-consistency model (ADR-0005).

**Cons:**
- Polling latency for real-time consumers. A client wanting new units
  immediately must poll at some frequency, burning bandwidth even when there
  is nothing new.

### Option B — REST/HTTP + WebSockets

A bidirectional WebSocket connection is used for streaming events.

**Pros:**
- True push: the node sends units to the client the moment they are stored.
- Supports future bidirectional protocols (e.g., streaming submissions).

**Cons:**
- Stateful: load balancers must use sticky sessions or a shared event bus.
- Significantly higher implementation surface.
- Bidirectionality is not needed — unit submissions are always
  client-to-node, and unit pushes are always node-to-client.

### Option C — REST/HTTP + SSE (RECOMMENDED streaming extension)

REST is the baseline; SSE provides a lightweight streaming channel for clients
that want real-time unit delivery without polling.

SSE is an HTTP response that stays open and delivers newline-delimited events.
It is one-directional, has built-in reconnect semantics (`Last-Event-ID`
header), and requires no special protocol negotiation beyond `Accept:
text/event-stream`.

**Pros:**
- All the benefits of REST/HTTP.
- Real-time delivery without polling, using standard HTTP infrastructure.
- `Last-Event-ID` reconnect maps cleanly onto the UUIDv7-based cursor model.
- No persistent connection state beyond the open HTTP response.

**Cons:**
- SSE requires keeping an HTTP response open. This consumes a connection and a
  file descriptor per subscribed client. At large scale, a separate streaming
  tier is typical (e.g., a pub-sub gateway in front of nodes).
- SSE is one-directional. It cannot replace the submission endpoint.

### Option D — ActivityPub federation

Use ActivityPub as the federation protocol; units map onto Activities and
actors map onto Agents.

**Pros:**
- Existing software (Mastodon, etc.) could potentially federate with
  SemanticWeft nodes.
- Established governance and specification.

**Cons:**
- ActivityPub's actor/activity model does not map cleanly onto Semantic Units.
  The impedance mismatch would require translation layers that obscure the
  protocol's semantics.
- ActivityPub mandates specific delivery guarantees, inbox/outbox semantics,
  and JSON-LD framing that add significant complexity.
- SemanticWeft's unit graph is a different data model from ActivityPub's
  social graph; forcing compatibility would constrain both.

---

## Decision

**Option C is adopted.**

REST/HTTP is the required baseline transport for all SemanticWeft nodes. SSE
is the SHOULD-level extension for streaming unit delivery.

### Required (MUST)

- All node endpoints MUST be served over HTTP/1.1 or later.
- Nodes MUST support HTTPS. Plain HTTP MAY be used in development or
  closed-network deployments.
- All request and response bodies MUST use `application/json`.

### Streaming (SHOULD)

- Nodes SHOULD expose the `/v1/sync` endpoint in SSE mode when the client
  sends `Accept: text/event-stream`.
- Each SSE event payload is a single Semantic Unit serialized as JSON.
- Nodes SHOULD include the unit's `id` as the SSE `id:` field so that
  clients can resume from `Last-Event-ID` after a disconnect.

### Out of scope for v1

- WebSockets are not part of this specification. An implementation MAY add
  WebSocket support as an extension, but conformance does not require it.
- ActivityPub compatibility is explicitly deferred. A future ADR may address
  a read-only ActivityPub view of a node's unit stream if there is demonstrated
  demand.

---

## Consequences

- The node API specification (`spec/node-api.md`) defines REST endpoints as
  the normative interface.
- The `GET /v1/sync` endpoint handles both polling (JSON) and streaming (SSE)
  depending on the `Accept` header.
- Node implementations may serve SSE from the same process as REST endpoints,
  or route streaming requests to a dedicated gateway — the wire protocol is
  identical from the client's perspective.
- Conformance tests (Phase 6) will test REST endpoints. SSE conformance
  is SHOULD-level and MAY be tested by a separate test profile.
