//! `sweft-node` — SemanticWeft reference node implementation.
//!
//! # Quick start
//!
//! ```sh
//! # In-memory node on the default port:
//! sweft-node
//!
//! # Persistent SQLite node:
//! SWEFT_DB=./data.db sweft-node
//!
//! # Custom bind address and node identity:
//! SWEFT_BIND=0.0.0.0:8080 SWEFT_NODE_ID=did:key:z6Mk... sweft-node
//! ```
//!
//! # Environment variables
//!
//! See [`config::NodeConfig::from_env`] for the full list.

mod config;
mod middleware;
mod error;
mod federation;
mod handlers;
mod identity;
mod peer_discovery;
mod router;
mod storage;

use std::sync::Arc;
use std::time::Duration;

use config::NodeConfig;
use federation::FederationSync;
use identity::NodeIdentity;
use peer_discovery::PeerDiscovery;
use semanticweft_node_api::PeerInfo;
use storage::{memory::MemoryStorage, sqlite::SqliteStorage, Storage};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "semanticweft_node=info,tower_http=debug".into()
            }),
        )
        .init();

    let mut config = NodeConfig::from_env();

    let storage: Arc<dyn Storage> = match &config.db_path {
        Some(path) => {
            tracing::info!("storage: SQLite at {path}");
            Arc::new(
                SqliteStorage::open(path)
                    .unwrap_or_else(|e| panic!("failed to open SQLite database at {path}: {e}")),
            )
        }
        None => {
            tracing::info!("storage: in-memory (data will not survive restart)");
            Arc::new(MemoryStorage::new())
        }
    };

    // Initialise node identity (load or generate Ed25519 keypair).
    let identity = NodeIdentity::load_or_generate(&storage)
        .await
        .unwrap_or_else(|e| panic!("failed to initialise node identity: {e}"));

    if config.node_id_needs_generation() {
        config.node_id = identity.did();
        tracing::info!("identity: generated node DID {}", config.node_id);
    } else {
        tracing::info!("identity: using configured node DID {}", config.node_id);
    }
    config.public_key = Some(identity.public_key_multibase());

    // Warn when api_base looks non-routable (helps operators catch misconfiguration).
    if config.api_base.contains("0.0.0.0") || config.api_base.contains("127.0.0.1") {
        tracing::warn!(
            "api_base '{}' may not be routable from other nodes; \
             set SWEFT_API_BASE to your public URL",
            config.api_base
        );
    }

    // Spawn the background federation sync loop.
    {
        let sync_storage = Arc::clone(&storage);
        let interval = Duration::from_secs(config.sync_interval_secs);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client for federation sync");

        tracing::info!(
            "federation: sync loop starting (interval = {}s)",
            config.sync_interval_secs
        );

        tokio::spawn(async move {
            FederationSync::new(client, sync_storage).run(interval).await;
        });
    }

    // Spawn the bootstrap peer discovery sweep.
    {
        let discovery_storage = Arc::clone(&storage);
        let bootstrap_peers = config.bootstrap_peers.clone();
        let max_peers = config.max_peers;
        let own_info = PeerInfo::new(config.node_id.clone(), config.api_base.clone());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client for peer discovery");

        tokio::spawn(async move {
            let discovery = PeerDiscovery::new(client, discovery_storage, own_info, max_peers);
            discovery.bootstrap(&bootstrap_peers).await;
        });
    }

    let (app, _sse_tx) = router::build_router(storage, config.clone(), std::sync::Arc::new(identity.signing_key().clone()));

    tracing::info!("listening on {}", config.bind_addr);
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {}: {e}", config.bind_addr));

    axum::serve(listener, app)
        .await
        .expect("server error");
}
