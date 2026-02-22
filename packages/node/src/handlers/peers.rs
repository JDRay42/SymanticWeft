//! Peer management handlers — `GET /v1/peers`, `POST /v1/peers`, and
//! `PATCH /v1/peers/{node_id}` (spec §7, ADR-0008).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
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
/// # Community voting gate (ADR-0008 §Phase 2)
///
/// Only known local peers may influence reputation. The caller identifies itself
/// via the `X-Node-ID` request header. The handler then enforces two constraints:
///
/// 1. **Membership**: the caller's `node_id` must appear in this node's peer list.
///    Outsiders receive `403 Forbidden`.
///
/// 2. **Threshold**: the caller's own reputation must be at or above the
///    community voting threshold: `max(0.0, mean − σ_factor × stddev)`, where
///    the statistics are computed over all peers and `σ_factor` is
///    `SWEFT_REPUTATION_VOTE_SIGMA_FACTOR` (default 1.0).
///    When all peers have the same reputation (stddev = 0), the threshold equals
///    the mean — so every peer can vote, which is the correct behaviour for new
///    or homogeneous communities.
///
/// # Weighted update
///
/// A qualifying vote is applied as a weighted average rather than a direct
/// assignment, with the caller's reputation as the weight:
///
/// ```text
/// new_rep = current_rep × (1 − caller_rep) + proposed_rep × caller_rep
/// ```
///
/// A caller with reputation 1.0 overrides the current value entirely; a caller
/// at the threshold has proportionally less influence. This is the EigenTrust
/// update rule adapted for direct `PATCH` calls (ADR-0008).
///
/// Returns 200 with the updated [`PeerInfo`] on success, 400 if the body is
/// invalid, 403 for self-updates or access control violations, or 404 if the
/// target peer is not known to this node.
pub async fn update_reputation(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    headers: HeaderMap,
    Json(update): Json<ReputationUpdate>,
) -> Result<impl IntoResponse, AppError> {
    // 1. Validate the proposed value.
    if !update.reputation.is_finite() || update.reputation < 0.0 || update.reputation > 1.0 {
        return Err(AppError::BadRequest(
            "reputation must be a finite number in [0.0, 1.0]".into(),
        ));
    }

    // 2. A node cannot update its own reputation.
    if node_id == state.config.node_id {
        return Err(AppError::Forbidden(
            "a node cannot update its own reputation".into(),
        ));
    }

    // 3. Require caller identity via X-Node-ID header.
    let caller_id = headers
        .get("x-node-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Forbidden(
                "X-Node-ID header is required; only known local peers may update reputation".into(),
            )
        })?
        .to_string();

    // 4. Caller must be a known local peer (community membership gate).
    let peers = state.storage.list_peers().await?;
    let caller = peers
        .iter()
        .find(|p| p.node_id == caller_id)
        .ok_or_else(|| {
            AppError::Forbidden(
                "caller is not a known local peer; reputation updates are community-internal".into(),
            )
        })?
        .clone();

    // 5. Community threshold gate: caller must be at or above mean − σ·stddev.
    let stats = state.storage.peer_reputation_stats().await?;
    let threshold = (stats.mean - state.config.reputation_vote_sigma_factor * stats.stddev).max(0.0);
    if caller.reputation < threshold {
        return Err(AppError::Forbidden(format!(
            "caller reputation {:.3} is below the community voting threshold {:.3}",
            caller.reputation, threshold,
        )));
    }

    // 6. Locate the target peer (before writing, so we can do the weighted merge).
    let target = peers
        .iter()
        .find(|p| p.node_id == node_id)
        .ok_or_else(|| AppError::NotFound(format!("peer {node_id} not found")))?;

    // 7. Weighted update: new = current × (1 − weight) + proposed × weight.
    let weight = caller.reputation;
    let new_reputation = (target.reputation * (1.0 - weight) + update.reputation * weight)
        .clamp(0.0, 1.0);

    state
        .storage
        .update_peer_reputation(&node_id, new_reputation)
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
            reputation_vote_sigma_factor: 1.0,
        };
        let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
        build_router(storage, config, signing_key).0
    }

    /// Helper: add a peer with an explicit reputation directly in storage.
    async fn add_peer_with_rep(storage: &Arc<dyn Storage>, node_id: &str, api_base: &str, rep: f32) {
        let mut p = PeerInfo::new(node_id, api_base);
        p.reputation = rep;
        storage.add_peer(&p).await.unwrap();
        // MemoryStorage inserts the struct as-is for new peers, so reputation is preserved.
        // Ensure the stored value matches by updating explicitly.
        storage.update_peer_reputation(node_id, rep).await.unwrap();
    }

    #[tokio::test]
    async fn update_reputation_returns_200_with_updated_peer() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // Voter has reputation 1.0 so weight = 1.0 and the weighted update is a direct override.
        // In a 2-peer community (voter=1.0, target=0.5): mean=0.75, stddev=0.25, threshold=0.5.
        // voter.rep=1.0 >= 0.5 ✓; new = 0.5*(1-1.0) + 0.9*1.0 = 0.9.
        add_peer_with_rep(&storage, "did:key:zVoter", "https://voter.example.com/v1", 1.0).await;
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
            .header("x-node-id", "did:key:zVoter")
            .body(Body::from(r#"{"reputation":0.9}"#))
            .unwrap();

        let resp = build_app(Arc::clone(&storage)).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let peer: Value = serde_json::from_slice(&body).unwrap();
        let rep = peer["reputation"].as_f64().unwrap();
        // weight=1.0 → new = 0.5*(0) + 0.9*(1) = 0.9
        assert!((rep - 0.9).abs() < 1e-4, "expected 0.9, got {rep}");

        let peers = storage.list_peers().await.unwrap();
        let target = peers.iter().find(|p| p.node_id == node_id).unwrap();
        assert!((target.reputation - 0.9).abs() < 1e-4);
    }

    #[tokio::test]
    async fn update_reputation_unknown_peer_returns_404() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // Voter present (rep 0.5); in a 1-peer community: mean=0.5, stddev=0, threshold=0.5.
        // voter.rep=0.5 >= 0.5 ✓. Target unknown → 404.
        storage
            .add_peer(&PeerInfo::new("did:key:zVoter", "https://voter.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zUnknown")
            .header("content-type", "application/json")
            .header("x-node-id", "did:key:zVoter")
            .body(Body::from(r#"{"reputation":0.5}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn update_own_reputation_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // Self-update is checked before X-Node-ID, so no header needed here.
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
        // Value validation is checked first, before X-Node-ID or membership.
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

    #[tokio::test]
    async fn missing_x_node_id_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        storage
            .add_peer(&PeerInfo::new("did:key:zPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zPeer")
            .header("content-type", "application/json")
            // No X-Node-ID header.
            .body(Body::from(r#"{"reputation":0.7}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn outsider_x_node_id_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        storage
            .add_peer(&PeerInfo::new("did:key:zPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();
        // zOutsider is NOT in the peer list.

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zPeer")
            .header("content-type", "application/json")
            .header("x-node-id", "did:key:zOutsider")
            .body(Body::from(r#"{"reputation":0.7}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn below_threshold_caller_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // Seed two peers: one high-rep and one low-rep (the "voter").
        // With sigma_factor=1.0: mean=(0.9+0.1)/2=0.5, stddev=0.4, threshold=0.5-0.4=0.1.
        // voter.rep=0.1 < 0.1 is false (0.1 >= 0.1), so let's use 0.05 as voter rep.
        // mean=(0.9+0.05)/2=0.475, stddev≈0.425, threshold=0.475-0.425=0.05.
        // voter.rep=0.05 >= 0.05 → still passes. Need a tighter example.
        // Use 3 peers: [0.9, 0.9, 0.05]. mean≈0.617, stddev≈0.399, threshold≈0.218.
        // voter.rep=0.05 < 0.218 → rejected ✓.
        add_peer_with_rep(&storage, "did:key:zHighA", "https://ha.example.com/v1", 0.9).await;
        add_peer_with_rep(&storage, "did:key:zHighB", "https://hb.example.com/v1", 0.9).await;
        add_peer_with_rep(&storage, "did:key:zLowVoter", "https://low.example.com/v1", 0.05).await;
        storage
            .add_peer(&PeerInfo::new("did:key:zTarget", "https://target.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zTarget")
            .header("content-type", "application/json")
            .header("x-node-id", "did:key:zLowVoter")
            .body(Body::from(r#"{"reputation":0.9}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn new_community_all_same_rep_everyone_can_vote() {
        // In a community where every peer has the same reputation (stddev=0),
        // the threshold equals the mean — and since every peer is at the mean,
        // every peer can vote. This is the correct behaviour for new communities.
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // All peers start at default 0.5.
        storage
            .add_peer(&PeerInfo::new("did:key:zVoter", "https://voter.example.com/v1"))
            .await
            .unwrap();
        storage
            .add_peer(&PeerInfo::new("did:key:zTarget", "https://target.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zTarget")
            .header("content-type", "application/json")
            .header("x-node-id", "did:key:zVoter")
            .body(Body::from(r#"{"reputation":0.7}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        // voter.rep=0.5 >= threshold=0.5 → allowed.
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn weighted_update_scales_with_caller_reputation() {
        // Verify the weighted-average formula: new = current*(1-weight) + proposed*weight.
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        // All peers at 0.5 → threshold=0.5; voter at 0.5 passes.
        // new = 0.5*(1-0.5) + 0.8*0.5 = 0.25 + 0.4 = 0.65.
        storage
            .add_peer(&PeerInfo::new("did:key:zVoter", "https://voter.example.com/v1"))
            .await
            .unwrap();
        storage
            .add_peer(&PeerInfo::new("did:key:zTarget", "https://target.example.com/v1"))
            .await
            .unwrap();

        let req = Request::builder()
            .method("PATCH")
            .uri("/v1/peers/did:key:zTarget")
            .header("content-type", "application/json")
            .header("x-node-id", "did:key:zVoter")
            .body(Body::from(r#"{"reputation":0.8}"#))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let peer: Value = serde_json::from_slice(&body).unwrap();
        let rep = peer["reputation"].as_f64().unwrap();
        // 0.5*(0.5) + 0.8*(0.5) = 0.25 + 0.40 = 0.65
        assert!((rep - 0.65).abs() < 1e-4, "expected 0.65 (weighted blend), got {rep}");
    }
}
