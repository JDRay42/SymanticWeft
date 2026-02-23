//! Agent registration types — `POST/GET/DELETE /v1/agents/{did}` (spec §8.1, ADR-0007).
//!
//! Agents are the identity layer of SemanticWeft. An agent registers its DID on
//! a node so that other nodes can discover its inbox endpoint via WebFinger and
//! deliver fan-out units to it.

use serde::{Deserialize, Serialize};

fn default_agent_reputation() -> f32 {
    0.5
}

/// Membership status of an agent registered on this node (ADR-0013).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Fully admitted member. May sponsor new applicants and participate
    /// without restriction.
    #[default]
    Full,
    /// Admitted on probation via the self-service apply endpoint. May
    /// participate normally but cannot sponsor other applicants until their
    /// contribution count reaches the node's configured threshold.
    Probationary,
}

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
///   "display_name": "Researcher-7",
///   "status": "full",
///   "contribution_count": 0,
///   "reputation": 0.5
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

    /// Membership status on this node. Defaults to `Full` for backward
    /// compatibility with records that pre-date ADR-0013.
    #[serde(default)]
    pub status: AgentStatus,

    /// Number of community contributions recorded for this agent.
    /// A probationary agent graduates to `Full` when this reaches the node's
    /// configured `probation_threshold`. Defaults to `0`.
    #[serde(default)]
    pub contribution_count: u32,

    /// This node's local reputation score for the agent, in `[0.0, 1.0]`.
    ///
    /// Defaults to `0.5` (neutral). Adjusted over time by community voting
    /// via `PATCH /v1/agents/{did}/reputation`. Included in sync responses
    /// as `author_reputations` so that receiving nodes can compute a
    /// credibility score for federated units.
    #[serde(default = "default_agent_reputation")]
    pub reputation: f32,
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

/// Request body for `POST /v1/agents/{did}/apply` — self-service application
/// for node membership (ADR-0013).
///
/// Applicants begin as `Probationary` regardless of whether a sponsor is
/// provided. Full membership is granted by the operator directly (via
/// `POST /v1/agents/{did}`) or automatically when the agent's contribution
/// count reaches the node's configured threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApplyRequest {
    /// The agent's decentralised identifier.
    pub did: String,

    /// The inbox URL the agent wants to advertise.
    pub inbox_url: String,

    /// Optional human-readable display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Optional public key material.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,

    /// DID of an existing `Full`-status member willing to vouch for this
    /// applicant. An invalid or absent sponsor does not block admission; it
    /// is recorded in the operator webhook notification only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sponsor_did: Option<String>,
}

/// Request body for `PATCH /v1/agents/{did}/reputation` — update an agent's
/// reputation score.
///
/// `reputation` must be in `[0.0, 1.0]`. The update is community-gated and
/// weighted: only registered agents above the community voting threshold may
/// vote, and the vote is blended with the current value using the caller's
/// own reputation as weight (EigenTrust update rule).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentReputationUpdate {
    /// Proposed reputation score for the agent, in `[0.0, 1.0]`.
    pub reputation: f32,
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
            status: AgentStatus::Full,
            contribution_count: 0,
            reputation: 0.5,
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: AgentProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back, profile);
    }

    #[test]
    fn agent_profile_status_serialises() {
        let profile = AgentProfile {
            did: "did:key:z6Mk".into(),
            inbox_url: "https://example.com/inbox".into(),
            display_name: None,
            public_key: None,
            status: AgentStatus::Probationary,
            contribution_count: 3,
            reputation: 0.75,
        };
        let json = serde_json::to_string(&profile).unwrap();
        assert!(json.contains("\"status\":\"probationary\""));
        assert!(json.contains("\"contribution_count\":3"));
        assert!(json.contains("\"reputation\":0.75"));
    }

    #[test]
    fn agent_profile_defaults_when_fields_absent() {
        // Pre-existing JSON without status, contribution_count, or reputation
        // fields should deserialise with defaults.
        let json = r#"{"did":"did:key:z6Mk","inbox_url":"https://example.com/inbox"}"#;
        let profile: AgentProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.status, AgentStatus::Full);
        assert_eq!(profile.contribution_count, 0);
        assert!((profile.reputation - 0.5).abs() < f32::EPSILON);
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
