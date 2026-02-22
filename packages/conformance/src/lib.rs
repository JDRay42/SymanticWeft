//! Shared helpers for the SemanticWeft conformance test suite.
//!
//! Provides [`spawn_node`] — a function that binds a `TcpListener` on an
//! ephemeral port, wires up an in-process node backed by `MemoryStorage`,
//! and returns both the local URL and a reference to the underlying storage
//! so tests can pre-populate data without going through the HTTP layer.

use std::sync::Arc;

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use semanticweft_node::{build_router, config::NodeConfig, storage::memory::MemoryStorage, Storage};

/// Start an ephemeral in-process node and return `(base_url, storage)`.
///
/// The node runs in a background `tokio` task and is bound to an OS-assigned
/// port on `127.0.0.1`. The returned `String` is the full API base URL, e.g.
/// `http://127.0.0.1:51234`. The returned `Arc<MemoryStorage>` gives tests
/// direct access to the same storage instance the node uses, so they can
/// seed data without bootstrapping circular auth requirements.
///
/// # Panics
///
/// Panics if the TCP listener cannot be bound or the node fails to start.
pub async fn spawn_node() -> (String, Arc<MemoryStorage>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("get local addr");
    let base_url = format!("http://{addr}");

    let mem_storage = Arc::new(MemoryStorage::new());
    let storage: Arc<dyn Storage> = Arc::clone(&mem_storage) as Arc<dyn Storage>;

    let config = NodeConfig {
        node_id: "did:key:zConformanceNode".into(),
        name: Some("conformance-node".into()),
        api_base: base_url.clone(),
        contact: None,
        bind_addr: addr,
        db_path: None,
        sync_interval_secs: 3600,
        bootstrap_peers: vec![],
        max_peers: 10,
        public_key: None,
        rate_limit_per_minute: 0,
        reputation_vote_sigma_factor: 1.0,
    };
    let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
    let (router, _sse_tx) = build_router(storage, config, signing_key);

    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("conformance node error");
    });

    (base_url, mem_storage)
}
