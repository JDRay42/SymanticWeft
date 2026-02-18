//! Node configuration, populated from environment variables.

use std::net::SocketAddr;

/// Runtime configuration for a SemanticWeft node.
///
/// All fields are populated from environment variables with sensible
/// defaults, so a node can be started with zero configuration.
///
/// | Variable | Default | Description |
/// |----------|---------|-------------|
/// | `SWEFT_BIND` | `0.0.0.0:3000` | TCP socket address to listen on |
/// | `SWEFT_API_BASE` | derived from SWEFT_BIND | Base URL advertised in the discovery doc |
/// | `SWEFT_NODE_ID` | placeholder DID | Stable DID for this node |
/// | `SWEFT_NAME` | (absent) | Human-readable node name |
/// | `SWEFT_CONTACT` | (absent) | Operator contact email or URL |
/// | `SWEFT_DB` | (absent = in-memory) | Path to the SQLite database file |
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Stable DID identifier for this node.
    pub node_id: String,

    /// Human-readable name, shown in the discovery document.
    pub name: Option<String>,

    /// Base URL of the `/v1/` API, advertised to peers.
    /// Example: `"https://node.example.com/v1"`.
    pub api_base: String,

    /// Operator contact info (email or URL), shown in the discovery document.
    pub contact: Option<String>,

    /// Socket address the server binds to.
    pub bind_addr: SocketAddr,

    /// Path to the SQLite database file.
    /// `None` means use an in-memory store (data is lost on restart).
    pub db_path: Option<String>,
}

impl NodeConfig {
    /// Populate config from environment variables, applying defaults where absent.
    pub fn from_env() -> Self {
        let bind_addr: SocketAddr = std::env::var("SWEFT_BIND")
            .unwrap_or_else(|_| "0.0.0.0:3000".into())
            .parse()
            .expect("SWEFT_BIND must be a valid socket address (e.g. 0.0.0.0:3000)");

        let api_base = std::env::var("SWEFT_API_BASE")
            .unwrap_or_else(|_| format!("http://{bind_addr}/v1"));

        Self {
            node_id: std::env::var("SWEFT_NODE_ID")
                .unwrap_or_else(|_| "did:key:z6MkDefaultNodeId".into()),
            name: std::env::var("SWEFT_NAME").ok(),
            api_base,
            contact: std::env::var("SWEFT_CONTACT").ok(),
            bind_addr,
            db_path: std::env::var("SWEFT_DB").ok(),
        }
    }
}
