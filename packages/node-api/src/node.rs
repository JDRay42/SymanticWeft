//! Node discovery types — `GET /.well-known/semanticweft` (spec §6).

use serde::{Deserialize, Serialize};

/// The response body for `GET /.well-known/semanticweft`.
///
/// Describes this node's identity, capabilities, and API location.
/// Agents and peers use this document to bootstrap interaction.
///
/// # Example
///
/// ```json
/// {
///   "node_id": "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuias8siQmsDNyZCeT",
///   "name": "Example Research Node",
///   "protocol_version": "1.0",
///   "api_base": "https://node.example.com",
///   "capabilities": ["sync", "sse", "subgraph", "peers"],
///   "signing_required": false,
///   "pow_required": null,
///   "contact": "admin@example.com"
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeInfo {
    /// Stable identifier for this node. SHOULD be a DID (see ADR-0001).
    pub node_id: String,

    /// Human-readable name for this node. OPTIONAL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Highest protocol version supported. MUST be `"1.0"` for conformant nodes.
    pub protocol_version: String,

    /// Public host URL of this node.
    /// Example: `"https://node.example.com"`.
    pub api_base: String,

    /// Optional features this node supports. See [`Capability`].
    pub capabilities: Vec<Capability>,

    /// If `true`, the node rejects unsigned units. Default: `false`.
    #[serde(default)]
    pub signing_required: bool,

    /// Proof-of-work parameters, if required. `null` or absent means not required.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pow_required: Option<PowParams>,

    /// Operator contact information (email or URL). OPTIONAL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,

    /// The node's Ed25519 public key, multibase-encoded (base58btc, `z` prefix).
    ///
    /// This is a convenience field — the same key is also embedded in `node_id`
    /// when `node_id` is a `did:key` DID. Peers use this field to verify node
    /// identity without implementing full DID resolution. Absent on nodes that
    /// have not yet generated a keypair.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
}

impl NodeInfo {
    /// The protocol version string for the 1.0 spec.
    pub const PROTOCOL_VERSION: &'static str = "1.0";

    /// Construct a minimal, conformant [`NodeInfo`].
    pub fn new(node_id: impl Into<String>, api_base: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            name: None,
            protocol_version: Self::PROTOCOL_VERSION.to_string(),
            api_base: api_base.into(),
            capabilities: vec![Capability::Sync],
            signing_required: false,
            pow_required: None,
            contact: None,
            public_key: None,
        }
    }
}

/// Optional features a node may advertise in its discovery document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    /// The `/v1/sync` endpoint is available (all conformant nodes; listed for
    /// discoverability).
    Sync,

    /// `/v1/sync` supports Server-Sent Events streaming (spec §5.5.2).
    Sse,

    /// The `/v1/units/{id}/subgraph` endpoint is available (spec §5.4).
    Subgraph,

    /// The `/v1/peers` endpoints are available (spec §7).
    Peers,

    /// Agent registration and WebFinger discovery are available (spec §8, ADR-0007).
    Agents,

    /// Follow-graph management endpoints are available (spec §8.3, ADR-0007).
    Follows,
}

/// Proof-of-work parameters advertised in the discovery document (ADR-0006).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PowParams {
    /// Hash algorithm used for the PoW computation. Currently `"sha256"`.
    pub algorithm: String,

    /// Required number of leading zero bits in the hash.
    pub difficulty: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_node_info_roundtrip() {
        let info = NodeInfo::new(
            "did:key:z6MkpTHR8VNsBxYAAWHut2Geadd9jSwuias8siQmsDNyZCeT",
            "https://node.example.com",
        );
        let json = serde_json::to_string(&info).unwrap();
        let back: NodeInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back, info);
        assert_eq!(back.protocol_version, "1.0");
        assert!(!back.signing_required);
        assert!(back.pow_required.is_none());
    }

    #[test]
    fn full_node_info_roundtrip() {
        let json = r#"{
            "node_id": "did:key:z6Mk",
            "name": "Test Node",
            "protocol_version": "1.0",
            "api_base": "https://example.com",
            "capabilities": ["sync", "sse", "subgraph", "peers"],
            "signing_required": true,
            "pow_required": { "algorithm": "sha256", "difficulty": 20 },
            "contact": "admin@example.com"
        }"#;
        let info: NodeInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.capabilities.len(), 4);
        assert!(info.signing_required);
        assert_eq!(info.pow_required.as_ref().unwrap().difficulty, 20);
    }

    #[test]
    fn capability_serialises_lowercase() {
        let json = serde_json::to_string(&Capability::Sse).unwrap();
        assert_eq!(json, r#""sse""#);
        let json = serde_json::to_string(&Capability::Subgraph).unwrap();
        assert_eq!(json, r#""subgraph""#);
    }
}
