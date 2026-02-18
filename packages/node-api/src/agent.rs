//! Agent registration types — `POST/GET/DELETE /v1/agents/{did}` (spec §8.1, ADR-0007).
//!
//! Agents are the identity layer of SemanticWeft. An agent registers its DID on
//! a node so that other nodes can discover its inbox endpoint via WebFinger and
//! deliver fan-out units to it.

use serde::{Deserialize, Serialize};

/// Persistent profile for a registered agent.
///
/// Returned by `GET /v1/agents/{did}` and `POST /v1/agents/{did}` on success.
///
/// # Example
///
/// ```json
/// {
///   "did": "did:key:z6MkHaXXX",
///   "inbox_url": "https://node.example.com/v1/agents/did%3Akey%3Az6MkHaXXX/inbox",
///   "display_name": "Researcher-7"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentProfile {
    /// The agent's decentralised identifier. Acts as the primary key.
    pub did: String,

    /// Absolute URL of this agent's inbox. Nodes POST fan-out units here.
    pub inbox_url: String,

    /// Human-readable label for the agent. OPTIONAL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Public key material for verifying HTTP Signatures from this agent.
    /// OPTIONAL; required only when the node enforces signed delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Request body for `POST /v1/agents/{did}` — register or update an agent.
///
/// The `did` path parameter and the `did` body field MUST match; servers
/// return `400` if they differ.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegisterRequest {
    /// The agent's decentralised identifier.
    pub did: String,

    /// The inbox URL the agent wants to advertise. The registering agent MUST
    /// control the resource at this URL.
    pub inbox_url: String,

    /// Optional human-readable display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Optional public key material (e.g. a JWK or multibase-encoded key) used
    /// to verify HTTP Signatures produced by this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

/// Response body for `GET /v1/agents/{did}/inbox`.
///
/// A page of units delivered to the agent's inbox that it has not yet
/// acknowledged. Items are ordered oldest-first.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InboxResponse {
    /// Units waiting in the inbox.
    pub items: Vec<semanticweft::SemanticUnit>,

    /// Cursor for fetching the next page; `null` when the inbox is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl InboxResponse {
    /// Construct an empty inbox response (no pending units).
    pub fn empty() -> Self {
        Self {
            items: vec![],
            next_cursor: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_roundtrip() {
        let req = RegisterRequest {
            did: "did:key:z6MkHaXXX".into(),
            inbox_url: "https://node.example.com/v1/agents/did%3Akey%3Az6MkHaXXX/inbox".into(),
            display_name: Some("Researcher-7".into()),
            public_key: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: RegisterRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
        // public_key absent in JSON when None
        assert!(!json.contains("public_key"));
    }

    #[test]
    fn agent_profile_roundtrip() {
        let profile = AgentProfile {
            did: "did:key:z6MkHaXXX".into(),
            inbox_url: "https://node.example.com/v1/agents/did%3Akey%3Az6MkHaXXX/inbox".into(),
            display_name: None,
            public_key: None,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn inbox_empty() {
        let resp = InboxResponse::empty();
        assert!(resp.items.is_empty());
        let json = serde_json::to_string(&resp).unwrap();
        // next_cursor should be omitted
        assert!(!json.contains("next_cursor"));
    }
}
