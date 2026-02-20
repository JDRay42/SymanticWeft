//! Assembles the Axum [`Router`] from all handler modules.

use std::sync::Arc;
use std::time::Duration;

use axum::{
    routing::{delete, get, post},
    Router,
};
use ed25519_dalek::SigningKey;

use crate::{
    config::NodeConfig,
    handlers::{agents, follows, node, peers, units, AppState},
    middleware::rate_limit::{rate_limit_middleware, RateLimiter},
    storage::Storage,
};

/// Build the complete application router with shared state.
pub fn build_router(
    storage: Arc<dyn Storage>,
    config: NodeConfig,
    signing_key: Arc<SigningKey>,
) -> Router {
    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("failed to build HTTP client for handler state");

    // Build the per-IP rate limiter from config (0 = disabled).
    let rate_limiter = Arc::new(RateLimiter::new(config.rate_limit_per_minute));

    let state = AppState {
        storage,
        config,
        http_client,
        signing_key,
    };

    Router::new()
        // Node discovery
        .route("/.well-known/semanticweft", get(node::well_known))
        // Units
        .route("/v1/units", post(units::submit).get(units::list))
        .route("/v1/units/{id}", get(units::get_by_id))
        .route("/v1/units/{id}/subgraph", get(units::subgraph))
        // Sync (node-to-node federation pull)
        .route("/v1/sync", get(units::sync))
        // Peers
        .route("/v1/peers", get(peers::list).post(peers::add))
        // Agents
        .route(
            "/v1/agents/{did}",
            post(agents::register).get(agents::get_agent),
        )
        .route(
            "/v1/agents/{did}/inbox",
            get(agents::inbox).post(agents::inbox_deliver),
        )
        // Follows
        .route(
            "/v1/agents/{did}/following",
            post(follows::follow).get(follows::list_following),
        )
        .route("/v1/agents/{did}/followers", get(follows::list_followers))
        .route(
            "/v1/agents/{did}/following/{target}",
            delete(follows::unfollow),
        )
        .with_state(state)
        // Rate limiting layer applied after routing so it can see the full request.
        .layer(axum::middleware::from_fn(move |req, next| {
            rate_limit_middleware(Arc::clone(&rate_limiter), req, next)
        }))
}
