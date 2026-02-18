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
mod error;
mod handlers;
mod router;
mod storage;

use std::sync::Arc;

use config::NodeConfig;
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

    let config = NodeConfig::from_env();

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

    let app = router::build_router(storage, config.clone());

    tracing::info!("listening on {}", config.bind_addr);
    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {}: {e}", config.bind_addr));

    axum::serve(listener, app)
        .await
        .expect("server error");
}
