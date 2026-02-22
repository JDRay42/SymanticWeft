//! Peer management handlers — `GET /v1/peers`, `POST /v1/peers`, and
//! `PATCH /v1/peers/{node_id}` (spec §7, ADR-0008).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use semanticweft_node_api::{PeerInfo, PeersResponse, ReputationUpdate};

use crate::{error::AppError, storage::Storage};

use super::AppState;

/// `GET /v1/peers` — return all peers known to this node.
pub async fn list(State(state): State<AppState>) -> Result<Json<PeersResponse>, AppError> {
    let peers = state.storage.list_peers().await?;
    Ok(Json(PeersResponse { peers }))
}

/// `POST /v1/peers` — register a new peer or update its `api_base`.
///
/// Stores the peer immediately and returns 200, then spawns a background task
/// to verify reachability via `/.well-known/semanticweft` (spec §7.2 SHOULD).
/// A successful verification (with a matching `node_id`) nudges reputation up;
/// an unreachable or mismatched peer nudges reputation down.
pub async fn add(
    State(state): State<AppState>,
    Json(peer): Json<PeerInfo>,
) -> Result<impl IntoResponse, AppError> {
    if peer.node_id.is_empty() || peer.api_base.is_empty() {
        return Err(AppError::BadRequest(
            "node_id and api_base are required".into(),
        ));
    }
    state.storage.add_peer(&peer).await?;

    // §7.2 SHOULD: verify reachability asynchronously so the endpoint stays
    // fast. The check updates last_seen / reputation in the background.
    tokio::spawn(verify_peer_reachability(
        state.http_client.clone(),
        Arc::clone(&state.storage),
        peer.node_id.clone(),
        peer.api_base.clone(),
    ));

    Ok((StatusCode::OK, Json(peer)))
}

/// Fetch `/.well-known/semanticweft` from the peer and update its reputation
/// based on reachability and whether its declared `node_id` matches.
async fn verify_peer_reachability(
    client: reqwest::Client,
    storage: Arc<dyn Storage>,
    node_id: String,
    api_base: String,
) {
    // Derive the well-known URL from api_base (strip path, keep scheme+host).
    let well_known = match reqwest::Url::parse(&api_base) {
        Ok(u) => {
            let scheme = u.scheme();
            let host = match u.host_str() {
                Some(h) => h,
                None => {
                    tracing::warn!("verify_peer: cannot extract host from {api_base}");
                    return;
                }
            };
            let port_str = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            format!("{scheme}://{host}{port_str}/.well-known/semanticweft")
        }
        Err(_) => {
            tracing::warn!("verify_peer: malformed api_base URL: {api_base}");
            return;
        }
    };

    match client.get(&well_known).send().await {
        Ok(resp) if resp.status().is_success() => {
            let id_matches = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("node_id")?.as_str().map(|s| s == node_id))
                .unwrap_or(false);

            if id_matches {
                tracing::debug!("verify_peer: {node_id} reachable, node_id confirmed");
                // Nudge reputation toward 0.55 to reward a successful verification.
                let _ = storage.update_peer_reputation(&node_id, 0.55).await;
            } else {
                tracing::warn!(
                    "verify_peer: {node_id} reachable at {api_base} \
                     but node_id in discovery document does not match"
                );
                // Mismatched identity is a stronger signal of misconfiguration.
                let _ = storage.update_peer_reputation(&node_id, 0.3).await;
            }
        }
        Ok(resp) => {
            // A non-success HTTP status (e.g. 404, 500) is a transient or
            // configuration error — do not penalise reputation, as an explicit
            // PATCH may already have set a considered value.
            tracing::warn!(
                "verify_peer: {node_id} returned HTTP {} for {well_known}",
                resp.status()
            );
        }
        Err(e) => {
            // Network errors are transient; leave reputation unchanged.
            tracing::warn!("verify_peer: {node_id} unreachable at {api_base}: {e}");
        }
    }
}

/// `PATCH /v1/peers/{node_id}` — update a peer's reputation score.
///
/// Accepts a [`ReputationUpdate`] body and applies it to the stored peer
/// record. The `reputation` value is clamped to `[0.0, 1.0]`. Returns 200
/// with the updated [`PeerInfo`] on success, 400 if the body is invalid, or
/// 404 if the peer is not known to this node.
pub async fn update_reputation(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Json(update): Json<ReputationUpdate>,
) -> Result<impl IntoResponse, AppError> {
    if !update.reputation.is_finite() || update.reputation < 0.0 || update.reputation > 1.0 {
        return Err(AppError::BadRequest(
            "reputation must be a finite number in [0.0, 1.0]".into(),
        ));
    }

    if node_id == state.config.node_id {
        return Err(AppError::Forbidden(
            "a node cannot update its own reputation".into(),
        ));
    }

    state
        .storage
        .update_peer_reputation(&node_id, update.reputation)
        .await
        .map_err(|e| match e {
            crate::storage::StorageError::NotFound => {
                AppError::NotFound(format!("peer {node_id} not found"))
            }
            other => AppError::Internal(other.to_string()),
        })?;

    // Return the updated peer record.
    let peers = state.storage.list_peers().await?;
    let peer = peers
        .into_iter()
        .find(|p| p.node_id == node_id)
        .ok_or_else(|| AppError::NotFound(format!("peer {node_id} not found")))?;

    Ok((StatusCode::OK, Json(peer)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{body::Body, http::{Request, StatusCode}};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use semanticweft_node_api::PeerInfo;
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::{
        config::NodeConfig,
        router::build_router,
        storage::{memory::MemoryStorage, Storage},
    };

    fn build_app(storage: Arc<dyn Storage>) -> axum::Router {
        let config = NodeConfig {
            node_id: "did:key:zNode".into(),
            name: None,
            api_base: "http://localhost/v1".into(),
            contact: None,
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            db_path: None,
            sync_interval_secs: 60,
            bootstrap_peers: vec![],
            max_peers: 100,
            public_key: None,
            rate_limit_per_minute: 0,
        };
        let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
        build_router(storage, config, signing_key).0
    }

    #[tokio::test]
    async fn update_reputation_returns_200_with_updated_peer() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        storage
            .add_peer(&PeerInfo::new("did:key:zPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();

        let node_id = "did:key:zPeer";
        let path = format!("/v1/peers/{node_id}");
        let req = Request::builder()
            .method("PATCH")
            .uri(&path)
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reputation":0.9}"#))
            .unwrap();

        let resp = build_app(Arc::clone(&storage)).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let peer: Value = serde_json::from_slice(&body).unwrap();
        let rep = peer["reputation"].as_f64().unwrap();
        assert!((rep - 0.9).abs() < 1e-5, "expected 0.9, got {rep}");

        // Also verify it persisted.
        let peers = storage.list_peers().await.unwrap();
        assert!((peers[0].reputation - 0.9).abs() < 1e-5);
    }

    #[tokio::test]
    async fn update_reputation_unknown_peer_returns_404() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zUnknown")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reputation":0.5}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_own_reputation_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // The node's own DID as set in build_app is "did:key:zNode".
        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zNode")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reputation":1.0}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn update_reputation_out_of_range_returns_400() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        storage
            .add_peer(&PeerInfo::new("did:key:zPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zPeer")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"reputation":1.5}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
