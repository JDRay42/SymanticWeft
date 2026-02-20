//! HTTP request handlers for all SemanticWeft node endpoints.
//!
//! Each submodule covers a logical group of endpoints as defined in
//! `spec/node-api.md`. Handlers are pure async functions that receive
//! Axum extractors and return `Result<impl IntoResponse, AppError>`.
//!
//! All authorization logic (visibility filtering) lives here, not in storage.

pub mod agents;
pub mod follows;
pub mod node;
pub mod peers;
pub mod units;

use std::sync::Arc;

use crate::{config::NodeConfig, storage::Storage};

/// Shared application state threaded through all Axum handlers via [`axum::extract::State`].
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn Storage>,
    pub config: NodeConfig,
    /// HTTP client for outbound requests (remote fan-out, etc.).
    /// `reqwest::Client` is cheaply clonable — it wraps an `Arc` internally.
    pub http_client: reqwest::Client,
    /// The node's Ed25519 signing key, used to authenticate outbound S2S requests.
    pub signing_key: Arc<ed25519_dalek::SigningKey>,
}
