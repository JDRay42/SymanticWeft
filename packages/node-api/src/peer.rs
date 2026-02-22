//! Peer management types — `GET /v1/peers` and `POST /v1/peers` (spec §7).

use serde::{Deserialize, Serialize};

fn default_reputation() -> f32 {
    0.5
}

/// A peer node entry: stable identifier, API base URL, and local reputation.
///
/// Used in [`PeersResponse`] and as the body for `POST /v1/peers`.
///
/// # Reputation
///
/// `reputation` is this node's *local assessment* of the peer, in `[0.0, 1.0]`.
/// It defaults to `0.5` (neutral) and is managed by the reputation system
/// described in ADR-0008. Senders SHOULD include their reputation score for
/// the peer when sharing peer lists; receivers weight the incoming score by
/// their own trust in the sender.
///
/// # Example
///
/// ```json
/// {
///   "node_id":    "did:key:z6MkiTBz1ymuepAQ4HEHYSF1H8quG5GLVVQR3djdX3mDooW",
///   "api_base":   "https://peer.example.com/v1",
///   "reputation": 0.85,
///   "last_seen":  "2026-02-19T14:32:00Z"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeerInfo {
    /// Stable identifier for the peer. SHOULD be a `did:key` DID whose embedded
    /// public key is used for identity verification (ADR-0001, ADR-0008).
    pub node_id: String,

    /// API base URL of the peer (e.g., `"https://peer.example.com/v1"`).
    /// This URL may change if the node moves servers; the `node_id` is the
    /// stable identity anchor.
    pub api_base: String,

    /// This node's local reputation score for the peer, in `[0.0, 1.0]`.
    ///
    /// Defaults to `0.5` (neutral). Adjusted over time by the reputation
    /// system (ADR-0008, Phase 2). Included when sharing peer lists so that
    /// receiving nodes can apply weighted-merge reconciliation.
    #[serde(default = "default_reputation")]
    pub reputation: f32,

    /// ISO 8601 timestamp of the most recent successful contact with this peer.
    ///
    /// Updated whenever the peer is successfully reached (sync, verify, or
    /// peer-exchange). Used as a tiebreaker in eviction: when the peer list
    /// is full, the lowest-reputation peer is evicted; ties are broken by
    /// oldest `last_seen`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<String>,
}

impl PeerInfo {
    /// Construct a new `PeerInfo` with neutral reputation and no `last_seen`.
    pub fn new(node_id: impl Into<String>, api_base: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            api_base: api_base.into(),
            reputation: 0.5,
            last_seen: None,
        }
    }
}

/// Response body for `GET /v1/peers`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeersResponse {
    /// Known peer nodes.
    pub peers: Vec<PeerInfo>,
}

/// Request body for `PATCH /v1/peers/{node_id}` — update a peer's reputation.
///
/// `reputation` must be in `[0.0, 1.0]`. Values outside this range are clamped
/// by the server. Returns 404 if the peer is not known to this node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReputationUpdate {
    /// New reputation score for the peer, in `[0.0, 1.0]`.
    pub reputation: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_response_roundtrip() {
        let resp = PeersResponse {
            peers: vec![
                PeerInfo::new("did:key:z6MkA", "https://a.example.com/v1"),
                PeerInfo::new("did:key:z6MkB", "https://b.example.com/v1"),
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: PeersResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn reputation_defaults_to_half() {
        let json = r#"{"node_id":"did:key:z6MkA","api_base":"https://a.example.com/v1"}"#;
        let p: PeerInfo = serde_json::from_str(json).unwrap();
        assert!((p.reputation - 0.5).abs() < f32::EPSILON);
        assert!(p.last_seen.is_none());
    }

    #[test]
    fn reputation_roundtrip() {
        let mut p = PeerInfo::new("did:key:z6MkA", "https://a.example.com/v1");
        p.reputation = 0.85;
        p.last_seen = Some("2026-02-19T14:32:00Z".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: PeerInfo = serde_json::from_str(&json).unwrap();
        assert!((back.reputation - 0.85).abs() < 1e-5);
        assert_eq!(back.last_seen.as_deref(), Some("2026-02-19T14:32:00Z"));
    }

    #[test]
    fn empty_peers_response() {
        let json = r#"{"peers":[]}"#;
        let resp: PeersResponse = serde_json::from_str(json).unwrap();
        assert!(resp.peers.is_empty());
    }
}
