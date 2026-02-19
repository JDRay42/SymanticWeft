//! Agent addressing — `did:key:z6Mk…@hostname` format.
//!
//! A [`AgentAddress`] encodes both *who* (the agent's DID) and *where*
//! (the node's public hostname) in a single, self-contained string.  HTTPS
//! is assumed; the `https://` scheme is an implicit constant.  The `/v1`
//! API path is intentionally omitted — it is discovered at delivery time
//! via `/.well-known/semanticweft`, so that addresses remain valid across
//! API version changes.
//!
//! # Format
//!
//! ```text
//! did:key:z6MkAgent…@sweft.mydomain.com
//! └────────────────┘ └────────────────┘
//!    agent DID            node hostname
//! ```
//!
//! # Delivery flow (host responsibility)
//!
//! 1. Call [`AgentAddress::well_known_url`] to get the discovery URL.
//! 2. `GET` that URL and pass the response body to [`NodeSession::from_discovery`].
//! 3. Call [`AgentAddress::inbox_url`] with the `api_base` from the session to
//!    get the final delivery endpoint.

use thiserror::Error;
use urlencoding::encode;

/// Errors that can occur when parsing an agent address string.
#[derive(Debug, Error, PartialEq)]
pub enum AddressError {
    #[error("missing '@' separator in agent address: '{0}'")]
    MissingAt(String),

    #[error("empty DID in agent address: '{0}'")]
    EmptyDid(String),

    #[error("empty hostname in agent address: '{0}'")]
    EmptyHostname(String),
}

/// A parsed agent address: `did:key:z6Mk…@hostname`.
///
/// The hostname is the node's public hostname with no scheme, no path, and
/// no port (unless non-standard).  HTTPS is always assumed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAddress {
    /// The agent's DID, e.g. `did:key:z6MkFoo…`.
    pub did: String,
    /// The node's hostname, e.g. `sweft.mydomain.com`.
    pub hostname: String,
}

impl AgentAddress {
    /// Construct from pre-validated parts.
    pub fn new(did: impl Into<String>, hostname: impl Into<String>) -> Self {
        Self {
            did: did.into(),
            hostname: hostname.into(),
        }
    }

    /// Parse a `did@hostname` string.
    ///
    /// Splits on the **last** `@` so that any DID method that theoretically
    /// contains `@` is handled safely.
    pub fn parse(s: &str) -> Result<Self, AddressError> {
        let at = s
            .rfind('@')
            .ok_or_else(|| AddressError::MissingAt(s.to_string()))?;

        let did = s[..at].trim();
        let hostname = s[at + 1..].trim();

        if did.is_empty() {
            return Err(AddressError::EmptyDid(s.to_string()));
        }
        if hostname.is_empty() {
            return Err(AddressError::EmptyHostname(s.to_string()));
        }

        Ok(Self {
            did: did.to_string(),
            hostname: hostname.to_string(),
        })
    }

    /// `https://hostname` — the node's base URL.
    pub fn node_url(&self) -> String {
        format!("https://{}", self.hostname)
    }

    /// `https://hostname/.well-known/semanticweft`
    ///
    /// **Fetch this URL** to discover the node's current `api_base`, then
    /// pass the response body to [`NodeSession::from_discovery`].
    pub fn well_known_url(&self) -> String {
        format!("https://{}/.well-known/semanticweft", self.hostname)
    }

    /// The agent's inbox URL, given the `api_base` from the discovery document.
    ///
    /// The DID is percent-encoded for safe use in the URL path.
    pub fn inbox_url(&self, api_base: &str) -> String {
        format!(
            "{}/agents/{}/inbox",
            api_base.trim_end_matches('/'),
            encode(&self.did)
        )
    }

    /// The agent's registration URL, given the `api_base` from the discovery
    /// document.
    pub fn register_url(&self, api_base: &str) -> String {
        format!(
            "{}/agents/{}",
            api_base.trim_end_matches('/'),
            encode(&self.did)
        )
    }
}

impl std::fmt::Display for AgentAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.did, self.hostname)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid() {
        let a = AgentAddress::parse("did:key:z6MkFoo@sweft.example.com").unwrap();
        assert_eq!(a.did, "did:key:z6MkFoo");
        assert_eq!(a.hostname, "sweft.example.com");
    }

    #[test]
    fn parse_missing_at() {
        assert_eq!(
            AgentAddress::parse("did:key:z6MkFoo"),
            Err(AddressError::MissingAt("did:key:z6MkFoo".into()))
        );
    }

    #[test]
    fn parse_empty_did() {
        assert!(matches!(
            AgentAddress::parse("@sweft.example.com"),
            Err(AddressError::EmptyDid(_))
        ));
    }

    #[test]
    fn parse_empty_hostname() {
        assert!(matches!(
            AgentAddress::parse("did:key:z6MkFoo@"),
            Err(AddressError::EmptyHostname(_))
        ));
    }

    #[test]
    fn well_known_url() {
        let a = AgentAddress::new("did:key:z6MkFoo", "sweft.example.com");
        assert_eq!(
            a.well_known_url(),
            "https://sweft.example.com/.well-known/semanticweft"
        );
    }

    #[test]
    fn inbox_url_encodes_did() {
        let a = AgentAddress::new("did:key:z6MkFoo", "sweft.example.com");
        assert_eq!(
            a.inbox_url("https://sweft.example.com/v1"),
            "https://sweft.example.com/v1/agents/did%3Akey%3Az6MkFoo/inbox"
        );
    }

    #[test]
    fn inbox_url_strips_trailing_slash() {
        let a = AgentAddress::new("did:key:z6MkFoo", "sweft.example.com");
        let with_slash = a.inbox_url("https://sweft.example.com/v1/");
        let without = a.inbox_url("https://sweft.example.com/v1");
        assert_eq!(with_slash, without);
    }

    #[test]
    fn display_roundtrip() {
        let original = "did:key:z6MkFoo@sweft.example.com";
        let a = AgentAddress::parse(original).unwrap();
        assert_eq!(a.to_string(), original);
    }
}
