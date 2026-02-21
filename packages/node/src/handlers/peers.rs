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
