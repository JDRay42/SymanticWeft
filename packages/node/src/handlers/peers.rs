//! Peer management handlers — `GET /v1/peers`, `POST /v1/peers`, and
//! `PATCH /v1/peers/{node_id}` (spec §7, ADR-0008).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use semanticweft_node_api::{PeerInfo, PeersResponse, ReputationUpdate};

use crate::error::AppError;

use super::AppState;

/// `GET /v1/peers` — return all peers known to this node.
pub async fn list(State(state): State<AppState>) -> Result<Json<PeersResponse>, AppError> {
    let peers = state.storage.list_peers().await?;
    Ok(Json(PeersResponse { peers }))
}

/// `POST /v1/peers` — register a new peer or update its `api_base`.
///
/// Returns 201 with the stored peer record.
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
    Ok((StatusCode::CREATED, Json(peer)))
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
