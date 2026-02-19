//! Node session — URL helpers for a resolved agent↔node connection.
//!
//! A [`NodeSession`] is constructed once the host has fetched the node's
//! discovery document and extracted the `api_base`.  It provides URL
//! computation for every node API endpoint the agent might call.  There is
//! no I/O here — the host supplies the discovery JSON, the session returns
//! strings, and the host makes the HTTP calls.
//!
//! # Typical host lifecycle
//!
//! ```text
//! // 1. Resolve the node address.
//! let addr = AgentAddress::parse("did:key:z6Mk…@sweft.example.com")?;
//!
//! // 2. Host fetches the discovery document (I/O happens here).
//! let discovery_json = http_get(addr.well_known_url()).await?;
//!
//! // 3. Build the session — pure computation from here on.
//! let session = NodeSession::from_discovery(&discovery_json, &identity.did())?;
//!
//! // 4. Use URL helpers for all subsequent calls.
//! let body = http_post(session.register_url(), register_payload).await?;
//! let inbox = http_get(session.inbox_url()).await?;
//! ```

use thiserror::Error;
use urlencoding::encode;

/// Errors that can occur when building a [`NodeSession`].
#[derive(Debug, Error, PartialEq)]
pub enum SessionError {
    #[error("failed to parse discovery document: {0}")]
    ParseError(String),

    #[error("discovery document is missing 'api_base' field")]
    MissingApiBase,
}

/// A resolved agent↔node session.
///
/// Constructed from the node's discovery document and the agent's own DID.
/// All methods return URL strings; the host makes the actual HTTP requests.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSession {
    /// The node's current API base URL, e.g. `https://sweft.example.com/v1`.
    /// Sourced from the `api_base` field in the discovery document.
    pub api_base: String,

    /// The acting agent's DID.
    pub own_did: String,
}

impl NodeSession {
    /// Construct directly from known parts (useful in tests or when
    /// `api_base` was obtained through another channel).
    pub fn new(api_base: impl Into<String>, own_did: impl Into<String>) -> Self {
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            own_did: own_did.into(),
        }
    }

    /// Parse the node's `/.well-known/semanticweft` response JSON to
    /// extract `api_base`, then build a session.
    pub fn from_discovery(discovery_json: &str, own_did: &str) -> Result<Self, SessionError> {
        let doc: serde_json::Value = serde_json::from_str(discovery_json)
            .map_err(|e| SessionError::ParseError(e.to_string()))?;

        let api_base = doc
            .get("api_base")
            .and_then(|v| v.as_str())
            .ok_or(SessionError::MissingApiBase)?;

        Ok(Self::new(api_base, own_did))
    }

    // ── Agent endpoints ───────────────────────────────────────────────────────

    /// `{api_base}/agents/{own_did_encoded}/inbox`
    pub fn inbox_url(&self) -> String {
        format!("{}/agents/{}/inbox", self.api_base, encode(&self.own_did))
    }

    /// `{api_base}/agents/{own_did_encoded}`
    pub fn register_url(&self) -> String {
        format!("{}/agents/{}", self.api_base, encode(&self.own_did))
    }

    // ── Unit endpoints ────────────────────────────────────────────────────────

    /// `{api_base}/units` — submit a new unit via `POST`.
    pub fn units_url(&self) -> String {
        format!("{}/units", self.api_base)
    }

    // ── Sync endpoint ─────────────────────────────────────────────────────────

    /// `{api_base}/sync?limit={limit}[&after={cursor}]`
    pub fn sync_url(&self, after: Option<&str>, limit: usize) -> String {
        match after {
            Some(cursor) => format!(
                "{}/sync?after={}&limit={}",
                self.api_base,
                encode(cursor),
                limit
            ),
            None => format!("{}/sync?limit={}", self.api_base, limit),
        }
    }

    // ── Peer endpoints ────────────────────────────────────────────────────────

    /// `{api_base}/peers`
    pub fn peers_url(&self) -> String {
        format!("{}/peers", self.api_base)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> NodeSession {
        NodeSession::new("https://sweft.example.com/v1", "did:key:z6MkFoo")
    }

    #[test]
    fn inbox_url() {
        assert_eq!(
            session().inbox_url(),
            "https://sweft.example.com/v1/agents/did%3Akey%3Az6MkFoo/inbox"
        );
    }

    #[test]
    fn register_url() {
        assert_eq!(
            session().register_url(),
            "https://sweft.example.com/v1/agents/did%3Akey%3Az6MkFoo"
        );
    }

    #[test]
    fn units_url() {
        assert_eq!(session().units_url(), "https://sweft.example.com/v1/units");
    }

    #[test]
    fn peers_url() {
        assert_eq!(session().peers_url(), "https://sweft.example.com/v1/peers");
    }

    #[test]
    fn sync_url_no_cursor() {
        let url = session().sync_url(None, 100);
        assert_eq!(url, "https://sweft.example.com/v1/sync?limit=100");
    }

    #[test]
    fn sync_url_with_cursor() {
        let url = session().sync_url(Some("abc-123"), 50);
        assert_eq!(url, "https://sweft.example.com/v1/sync?after=abc-123&limit=50");
    }

    #[test]
    fn from_discovery_json() {
        let json = r#"{
            "node_id": "did:key:z6MkNode",
            "api_base": "https://sweft.example.com/v1",
            "protocol_version": "1.0",
            "capabilities": ["sync"]
        }"#;
        let s = NodeSession::from_discovery(json, "did:key:z6MkFoo").unwrap();
        assert_eq!(s.api_base, "https://sweft.example.com/v1");
        assert_eq!(s.own_did, "did:key:z6MkFoo");
    }

    #[test]
    fn from_discovery_strips_trailing_slash() {
        let json = r#"{"api_base":"https://sweft.example.com/v1/","node_id":"x","protocol_version":"1.0","capabilities":[]}"#;
        let s = NodeSession::from_discovery(json, "did:key:z6MkFoo").unwrap();
        assert_eq!(s.api_base, "https://sweft.example.com/v1");
    }

    #[test]
    fn from_discovery_missing_api_base() {
        let json = r#"{"node_id":"did:key:z6MkNode"}"#;
        assert_eq!(
            NodeSession::from_discovery(json, "did:key:z6MkFoo"),
            Err(SessionError::MissingApiBase)
        );
    }
}
