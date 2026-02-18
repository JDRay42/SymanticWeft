//! Peer management handlers — `GET /v1/peers` and `POST /v1/peers` (spec §7).

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use semanticweft_node_api::{PeerInfo, PeersResponse};

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
