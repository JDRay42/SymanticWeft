//! Peer management types — `GET /v1/peers` and `POST /v1/peers` (spec §7).

use serde::{Deserialize, Serialize};

/// A peer node entry: a stable identifier and the base URL of its API.
///
/// Used in [`PeersResponse`] and as the body for `POST /v1/peers`.
///
/// # Example
///
/// ```json
/// {
///   "node_id": "did:key:z6MkiTBz1ymuepAQ4HEHYSF1H8quG5GLVVQR3djdX3mDooW",
///   "api_base": "https://peer.example.com/v1"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerInfo {
    /// Stable identifier for the peer. SHOULD be a DID.
    pub node_id: String,

    /// API base URL of the peer (e.g., `"https://peer.example.com/v1"`).
    pub api_base: String,
}

/// Response body for `GET /v1/peers`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeersResponse {
    /// Known peer nodes.
    pub peers: Vec<PeerInfo>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peers_response_roundtrip() {
        let resp = PeersResponse {
            peers: vec![
                PeerInfo {
                    node_id: "did:key:z6MkA".into(),
                    api_base: "https://a.example.com/v1".into(),
                },
                PeerInfo {
                    node_id: "did:key:z6MkB".into(),
                    api_base: "https://b.example.com/v1".into(),
                },
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: PeersResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
    }

    #[test]
    fn empty_peers_response() {
        let json = r#"{"peers":[]}"#;
        let resp: PeersResponse = serde_json::from_str(json).unwrap();
        assert!(resp.peers.is_empty());
    }
}
