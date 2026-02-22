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
/// | `SWEFT_API_BASE` | derived from SWEFT_BIND | Public host URL advertised in the discovery doc |
/// | `SWEFT_NODE_ID` | generated `did:key` | Stable DID for this node |
/// | `SWEFT_NAME` | (absent) | Human-readable node name |
/// | `SWEFT_CONTACT` | (absent) | Operator contact email or URL |
/// | `SWEFT_DB` | (absent = in-memory) | Path to the SQLite database file |
/// | `SWEFT_SYNC_INTERVAL_SECS` | `60` | Seconds between federation sync rounds |
/// | `SWEFT_BOOTSTRAP_PEERS` | (absent) | Comma-separated peer API base URLs |
/// | `SWEFT_MAX_PEERS` | `100` | Maximum number of peers to track |
/// | `SWEFT_RATE_LIMIT` | `60` | Max requests per minute per client IP (0 = unlimited) |
/// | `SWEFT_REPUTATION_VOTE_SIGMA_FACTOR` | `1.0` | Standard deviations below mean at which a peer loses voting rights |
#[derive(Debug, Clone)]
pub struct NodeConfig {
    /// Stable DID identifier for this node. Set to a generated `did:key` in
    /// `main` after identity initialisation if `SWEFT_NODE_ID` is not set.
    pub node_id: String,

    /// Human-readable name, shown in the discovery document.
    pub name: Option<String>,

    /// Public host URL of this node, advertised to peers.
    /// Example: `"https://node.example.com"`.
    pub api_base: String,

    /// Operator contact info (email or URL), shown in the discovery document.
    pub contact: Option<String>,

    /// Socket address the server binds to.
    pub bind_addr: SocketAddr,

    /// Path to the SQLite database file.
    /// `None` means use an in-memory store (data is lost on restart).
    pub db_path: Option<String>,

    /// How many seconds to wait between federation sync rounds.
    pub sync_interval_secs: u64,

    /// Bootstrap peer host URLs used on startup for peer discovery.
    /// Set via `SWEFT_BOOTSTRAP_PEERS` as a comma-separated list.
    pub bootstrap_peers: Vec<String>,

    /// Maximum number of peers to track locally.
    /// When full, the lowest-reputation peer is evicted for a new one.
    pub max_peers: usize,

    /// This node's Ed25519 public key, multibase-encoded (`z`-prefixed base58btc).
    /// `None` here; set in `main` after identity initialisation.
    pub public_key: Option<String>,

    /// Maximum number of requests per minute per client IP.
    /// `0` disables rate limiting entirely.
    /// Set via `SWEFT_RATE_LIMIT`. Default: 60.
    pub rate_limit_per_minute: u32,

    /// Number of standard deviations below the community mean reputation at which
    /// a peer loses the right to vote on reputation updates.
    ///
    /// The voting threshold is `max(0.0, mean − sigma_factor × stddev)`. A peer
    /// whose reputation is below this threshold has `PATCH /v1/peers/{id}` calls
    /// rejected with `403 Forbidden`.
    ///
    /// When every peer has the same reputation (stddev = 0) the threshold equals
    /// the mean, so all peers at that score can still vote — the correct behaviour
    /// for new or homogeneous communities.
    ///
    /// Set via `SWEFT_REPUTATION_VOTE_SIGMA_FACTOR`. Default: `1.0`.
    pub reputation_vote_sigma_factor: f32,
}

impl NodeConfig {
    /// Populate config from environment variables, applying defaults where absent.
    ///
    /// `node_id` is set to the sentinel `"__generate__"` when `SWEFT_NODE_ID`
    /// is absent; `main` replaces it with the generated `did:key` after
    /// calling [`identity::NodeIdentity::load_or_generate`].
    pub fn from_env() -> Self {
        let bind_addr: SocketAddr = std::env::var("SWEFT_BIND")
            .unwrap_or_else(|_| "0.0.0.0:3000".into())
            .parse()
            .expect("SWEFT_BIND must be a valid socket address (e.g. 0.0.0.0:3000)");

        let api_base = std::env::var("SWEFT_API_BASE")
            .unwrap_or_else(|_| format!("http://{bind_addr}"));

        let bootstrap_peers: Vec<String> = std::env::var("SWEFT_BOOTSTRAP_PEERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();

        let max_peers = std::env::var("SWEFT_MAX_PEERS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100);

        let sync_interval_secs = std::env::var("SWEFT_SYNC_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(60);

        let rate_limit_per_minute = std::env::var("SWEFT_RATE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(60);

        let reputation_vote_sigma_factor = std::env::var("SWEFT_REPUTATION_VOTE_SIGMA_FACTOR")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0);

        // Sentinel replaced in main after identity init.
        let node_id = std::env::var("SWEFT_NODE_ID")
            .unwrap_or_else(|_| "__generate__".into());

        Self {
            node_id,
            name: std::env::var("SWEFT_NAME").ok(),
            api_base,
            contact: std::env::var("SWEFT_CONTACT").ok(),
            bind_addr,
            db_path: std::env::var("SWEFT_DB").ok(),
            sync_interval_secs,
            bootstrap_peers,
            max_peers,
            public_key: None,
            rate_limit_per_minute,
            reputation_vote_sigma_factor,
        }
    }

    /// Returns `true` when `node_id` should be replaced by a generated DID.
    pub fn node_id_needs_generation(&self) -> bool {
        self.node_id == "__generate__"
    }
}
