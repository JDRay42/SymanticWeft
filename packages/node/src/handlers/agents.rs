//! Agent registration and inbox handlers (spec §8, ADR-0007).
//!
//! - `POST /v1/agents/{did}` — register or update an agent profile.
//! - `GET  /v1/agents/{did}` — retrieve an agent profile.
//! - `GET  /v1/agents/{did}/inbox` — retrieve the agent's pending inbox items.
//! - `POST /v1/agents/{did}/inbox` — deliver a unit to the agent's inbox (S2S, requires NodeAuth).
//!
//! # Inbox
//!
//! The inbox GET endpoint lists units already delivered to the agent's inbox.
//! The inbox POST endpoint accepts node-to-node push delivery, authenticated
//! via an HTTP Signature whose key is embedded in the delivering node's `did:key`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use semanticweft::{validate_unit, SemanticUnit};
use semanticweft_node_api::{
    AgentProfile, AgentReputationUpdate, AgentStatus, ApplyRequest, InboxResponse, RegisterRequest,
};
use tracing::{info, warn};

use crate::error::AppError;
use crate::middleware::auth::{NodeAuth, RequireAuth};

use super::AppState;

/// Query parameters for `GET /v1/agents/{did}/inbox`.
#[derive(Debug, Deserialize, Default)]
pub struct InboxQueryParams {
    /// Keyset pagination cursor (unit `id` of the last seen item).
    pub after: Option<String>,
    /// Page size (1–100, default 20).
    pub limit: Option<u32>,
}

/// `POST /v1/agents/{did}` — register or update an agent.
///
/// The `did` in the path and in the request body must match; returns 400
/// if they differ. Returns 201 with the stored profile.
pub async fn register(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.did != did {
        return Err(AppError::BadRequest(
            "did in request body must match the {did} path parameter".into(),
        ));
    }

    if auth.did != did {
        return Err(AppError::Forbidden(
            "cannot register as a different DID".into(),
        ));
    }

    let profile = AgentProfile {
        did: req.did,
        inbox_url: req.inbox_url,
        display_name: req.display_name,
        public_key: req.public_key,
        status: AgentStatus::Full,
        contribution_count: 0,
        reputation: 0.5,
    };

    state.storage.put_agent(&profile).await?;
    Ok((StatusCode::CREATED, Json(profile)))
}

/// `GET /v1/agents/{did}` — retrieve a registered agent's profile.
///
/// Returns 404 if the DID is not registered on this node.
pub async fn get_agent(
    State(state): State<AppState>,
    Path(did): Path<String>,
) -> Result<Json<AgentProfile>, AppError> {
    let profile = state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    Ok(Json(profile))
}

/// `GET /v1/agents/{did}/inbox` — list units delivered to the agent's inbox.
///
/// Returns 404 if the agent is not registered on this node.
/// Units are ordered oldest-first (UUIDv7 ascending). Use `?after=<id>` for
/// keyset pagination.
pub async fn inbox(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
    Query(params): Query<InboxQueryParams>,
) -> Result<Json<InboxResponse>, AppError> {
    state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    if auth.did != did {
        return Err(AppError::NotFound(format!("agent {did} not found")));
    }

    let limit = params.limit.map(|l| l.clamp(1, 100)).unwrap_or(20);
    let (items, has_more) = state
        .storage
        .get_inbox(&did, params.after.as_deref(), limit)
        .await?;

    let next_cursor = if has_more {
        items.last().map(|u| u.id.clone())
    } else {
        None
    };

    Ok(Json(InboxResponse { items, next_cursor }))
}

/// `DELETE /v1/agents/{did}` — deregister an agent from this node.
///
/// Removes the agent profile and purges their inbox. Only the agent itself
/// (authenticated via the same DID) may delete their own registration.
/// Returns 204 No Content on success, 404 if the DID is not registered,
/// 401 if unauthenticated, or 403 if the caller's DID does not match the
/// path DID.
pub async fn delete_agent(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
) -> Result<impl IntoResponse, AppError> {
    if auth.did != did {
        return Err(AppError::Forbidden(
            "cannot deregister a different agent".into(),
        ));
    }

    // Return 404 if the agent is not registered.
    state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    state.storage.delete_agent(&did).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /v1/agents/{did}/inbox` — node-to-node push delivery (spec §8.6).
///
/// Accepts a [`SemanticUnit`] from a remote node and delivers it to the
/// target agent's inbox. Requires a valid HTTP Signature from the delivering
/// node; the key is decoded directly from the `did:key` in the `keyId` field
/// (no storage lookup needed since `did:key` is self-describing).
///
/// Returns 201 on success, 401 if the signature is invalid, 404 if the agent
/// is not registered on this node, or 422 if the unit fails validation.
pub async fn inbox_deliver(
    State(state): State<AppState>,
    Path(did): Path<String>,
    _node_auth: NodeAuth,
    Json(unit): Json<SemanticUnit>,
) -> Result<impl IntoResponse, AppError> {
    // Verify the target agent is registered on this node.
    state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    // Validate the unit structure.
    validate_unit(&unit).map_err(|e| AppError::UnprocessableEntity(e.to_string()))?;

    // Deliver to the agent's inbox.
    state.storage.deliver_to_inbox(&did, &unit).await?;

    Ok(StatusCode::CREATED)
}

/// `POST /v1/agents/{did}/apply` — self-service application for node membership (ADR-0013).
///
/// Implements the tiered admission protocol. All self-service applicants begin
/// as `Probationary` regardless of tier:
///
/// - **Tier 1 (operator):** Use `POST /v1/agents/{did}` for direct Full admission.
/// - **Tier 2 (sponsored):** Providing a `sponsor_did` that belongs to an existing
///   `Full`-status member is noted in the webhook notification but does not change
///   the starting status — it signals community endorsement.
/// - **Tier 3 (unsponsored):** Admitted as Probationary with no endorsement.
///
/// A Probationary agent graduates automatically to Full once their contribution
/// count reaches the node's configured `probation_threshold`.
///
/// If `SWEFT_OPERATOR_WEBHOOK` is set, a notification is fired asynchronously
/// (fire-and-forget) with the admission details. Webhook delivery failure does
/// not affect the admission response.
///
/// Returns 201 with the stored profile, 409 if the DID is already registered.
pub async fn apply(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
    Json(req): Json<ApplyRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.did != did {
        return Err(AppError::BadRequest(
            "did in request body must match the {did} path parameter".into(),
        ));
    }
    if auth.did != did {
        return Err(AppError::Forbidden("cannot apply as a different DID".into()));
    }

    // Reject if already registered.
    if state.storage.get_agent(&did).await?.is_some() {
        return Err(AppError::Conflict(format!("agent {did} is already registered")));
    }

    // Check sponsor validity (informational; invalid sponsor does not block admission).
    let sponsor_valid = match req.sponsor_did.as_deref() {
        Some(s) => matches!(
            state.storage.get_agent(s).await?,
            Some(ref p) if p.status == AgentStatus::Full
        ),
        None => false,
    };

    let profile = AgentProfile {
        did: req.did.clone(),
        inbox_url: req.inbox_url,
        display_name: req.display_name,
        public_key: req.public_key,
        status: AgentStatus::Probationary,
        contribution_count: 0,
        reputation: 0.5,
    };

    state.storage.put_agent(&profile).await?;
    info!(
        "agent_admission: {} joined as probationary (sponsor={:?}, sponsor_valid={})",
        did, req.sponsor_did, sponsor_valid
    );

    // Fire operator webhook asynchronously — failure does not affect the response.
    if let Some(ref webhook_url) = state.config.operator_webhook_url {
        let client = state.http_client.clone();
        let url = webhook_url.clone();
        let node_id = state.config.node_id.clone();
        let sponsor_did = req.sponsor_did.clone();
        let profile_clone = profile.clone();
        tokio::spawn(async move {
            let payload = serde_json::json!({
                "event": "agent_admission",
                "node_id": node_id,
                "agent": profile_clone,
                "sponsor_did": sponsor_did,
                "sponsor_valid": sponsor_valid,
            });
            if let Err(e) = client.post(&url).json(&payload).send().await {
                warn!("operator webhook delivery failed: {e}");
            }
        });
    }

    Ok((StatusCode::CREATED, Json(profile)))
}

/// `PATCH /v1/agents/{did}/reputation` — update an agent's reputation score.
///
/// # Community voting gate
///
/// Only registered agents on this node may vote. The caller authenticates via
/// HTTP Signature ([`RequireAuth`]). The handler then enforces:
///
/// 1. **Membership**: the caller must be a registered agent on this node.
/// 2. **Threshold**: the caller's own reputation must be at or above the
///    community voting threshold: `max(0.0, mean − σ_factor × stddev)`, where
///    the statistics are computed over all registered agents and `σ_factor` is
///    `SWEFT_REPUTATION_VOTE_SIGMA_FACTOR` (default 1.0).
/// 3. **No self-voting**: an agent cannot update its own reputation.
///
/// # Weighted update
///
/// A qualifying vote is applied as a weighted average (EigenTrust):
///
/// ```text
/// new_rep = current_rep × (1 − caller_rep) + proposed_rep × caller_rep
/// ```
///
/// Returns 200 with the updated [`AgentProfile`] on success.
pub async fn update_reputation(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
    Json(update): Json<AgentReputationUpdate>,
) -> Result<impl IntoResponse, AppError> {
    // 1. Validate the proposed value.
    if !update.reputation.is_finite() || update.reputation < 0.0 || update.reputation > 1.0 {
        return Err(AppError::BadRequest(
            "reputation must be a finite number in [0.0, 1.0]".into(),
        ));
    }

    // 2. No self-voting.
    if auth.did == did {
        return Err(AppError::Forbidden(
            "an agent cannot update its own reputation".into(),
        ));
    }

    // 3. Caller must be a registered agent (membership gate).
    let caller = state
        .storage
        .get_agent(&auth.did)
        .await?
        .ok_or_else(|| {
            AppError::Forbidden(
                "caller is not a registered agent; reputation updates are community-internal"
                    .into(),
            )
        })?;

    // 4. Community threshold gate: caller must be at or above mean − σ·stddev.
    let stats = state.storage.agent_reputation_stats().await?;
    let threshold =
        (stats.mean - state.config.reputation_vote_sigma_factor * stats.stddev).max(0.0);
    if caller.reputation < threshold {
        return Err(AppError::Forbidden(format!(
            "caller reputation {:.3} is below the community voting threshold {:.3}",
            caller.reputation, threshold,
        )));
    }

    // 5. Locate the target agent.
    let target = state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    // 6. Weighted update: new = current × (1 − weight) + proposed × weight.
    let weight = caller.reputation;
    let new_reputation = (target.reputation * (1.0 - weight) + update.reputation * weight)
        .clamp(0.0, 1.0);

    state
        .storage
        .update_agent_reputation(&did, new_reputation)
        .await
        .map_err(|e| match e {
            crate::storage::StorageError::NotFound => {
                AppError::NotFound(format!("agent {did} not found"))
            }
            other => AppError::Internal(other.to_string()),
        })?;

    // Return the updated agent profile.
    let profile = state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    Ok((StatusCode::OK, Json(profile)))
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
    use semanticweft::{SemanticUnit, UnitType};
    use semanticweft_node_api::AgentProfile;
    use tower::ServiceExt;

    use crate::{
        config::NodeConfig,
        middleware::auth::build_outbound_signature,
        router::build_router,
        storage::{memory::MemoryStorage, Storage},
    };

    fn make_node_key_and_did() -> (SigningKey, String) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let pub_bytes = signing_key.verifying_key().to_bytes();
        let mut multicodec = vec![0xed_u8, 0x01];
        multicodec.extend_from_slice(&pub_bytes);
        let did = format!("did:key:z{}", bs58::encode(&multicodec).into_string());
        (signing_key, did)
    }

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
            operator_webhook_url: None,
            probation_threshold: 10,
        };
        let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
        build_router(storage, config, signing_key).0
    }

    fn make_unit(author: &str) -> SemanticUnit {
        SemanticUnit::new(UnitType::Assertion, "Test inbox delivery.", author)
    }

    #[tokio::test]
    async fn inbox_deliver_with_valid_node_sig_returns_201() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let agent_did = "did:key:z6MkAgentTarget";

        // Register the target agent.
        storage.put_agent(&AgentProfile {
            did: agent_did.to_string(),
            inbox_url: format!("http://localhost/v1/agents/{agent_did}/inbox"),
            display_name: None,
            public_key: None,
            status: semanticweft_node_api::AgentStatus::Full,
            contribution_count: 0,
            reputation: 0.5,
        }).await.unwrap();

        let (node_key, node_did) = make_node_key_and_did();
        let unit = make_unit(node_did.as_str());
        let path = format!("/v1/agents/{agent_did}/inbox");
        let (date, sig_header) = build_outbound_signature(
            &node_key, &node_did, "post", &path, "localhost",
        );

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig_header)
            .body(Body::from(serde_json::to_string(&unit).unwrap()))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn inbox_deliver_without_signature_returns_401() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let agent_did = "did:key:z6MkAgentNoSig";

        storage.put_agent(&AgentProfile {
            did: agent_did.to_string(),
            inbox_url: format!("http://localhost/v1/agents/{agent_did}/inbox"),
            display_name: None,
            public_key: None,
            status: semanticweft_node_api::AgentStatus::Full,
            contribution_count: 0,
            reputation: 0.5,
        }).await.unwrap();

        let unit = make_unit("did:key:z6MkSomeSender");
        let path = format!("/v1/agents/{agent_did}/inbox");

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&unit).unwrap()))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn inbox_deliver_to_unknown_agent_returns_404() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (node_key, node_did) = make_node_key_and_did();
        let unknown_did = "did:key:z6MkNotRegistered";
        let unit = make_unit(&node_did);
        let path = format!("/v1/agents/{unknown_did}/inbox");
        let (date, sig_header) = build_outbound_signature(
            &node_key, &node_did, "post", &path, "localhost",
        );

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig_header)
            .body(Body::from(serde_json::to_string(&unit).unwrap()))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // DELETE /v1/agents/{did}
    // -----------------------------------------------------------------------

    /// Helper: register an agent profile directly in storage with a known key.
    async fn seed_agent_with_key(
        storage: &Arc<dyn Storage>,
        signing_key: &SigningKey,
    ) -> (String, String) {
        let pub_bytes = signing_key.verifying_key().to_bytes();
        let mut multicodec = vec![0xed_u8, 0x01];
        multicodec.extend_from_slice(&pub_bytes);
        let did = format!("did:key:z{}", bs58::encode(&multicodec).into_string());
        let pubkey_multibase = format!("z{}", bs58::encode(&multicodec).into_string());
        storage
            .put_agent(&AgentProfile {
                did: did.clone(),
                inbox_url: format!("http://localhost/v1/agents/{did}/inbox"),
                display_name: None,
                public_key: Some(pubkey_multibase.clone()),
                status: semanticweft_node_api::AgentStatus::Full,
                contribution_count: 0,
                reputation: 0.5,
            })
            .await
            .unwrap();
        (did, pubkey_multibase)
    }

    #[tokio::test]
    async fn delete_own_agent_returns_204() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key, did) = make_node_key_and_did();
        seed_agent_with_key(&storage, &key).await;

        let path = format!("/v1/agents/{did}");
        let (date, sig) = build_outbound_signature(&key, &did, "delete", &path, "localhost");

        let req = Request::builder()
            .method("DELETE")
            .uri(&path)
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig)
            .body(Body::empty())
            .unwrap();

        let resp = build_app(storage.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Agent must be gone from storage.
        assert!(storage.get_agent(&did).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_agent_unauthenticated_returns_401() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key, did) = make_node_key_and_did();
        seed_agent_with_key(&storage, &key).await;

        let path = format!("/v1/agents/{did}");

        // No Signature or Date headers — RequireAuth must reject this.
        let req = Request::builder()
            .method("DELETE")
            .uri(&path)
            .body(Body::empty())
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn delete_agent_wrong_did_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

        // Register two independent agents.
        let (key_a, did_a) = make_node_key_and_did();
        let (_, did_b) = make_node_key_and_did();
        seed_agent_with_key(&storage, &key_a).await;
        // Register did_b with a dummy key so it exists in storage.
        storage
            .put_agent(&AgentProfile {
                did: did_b.clone(),
                inbox_url: format!("http://localhost/v1/agents/{did_b}/inbox"),
                display_name: None,
                public_key: None,
                status: semanticweft_node_api::AgentStatus::Full,
                contribution_count: 0,
                reputation: 0.5,
            })
            .await
            .unwrap();

        // Agent A tries to delete Agent B's registration — must be 403.
        let path = format!("/v1/agents/{did_b}");
        let (date, sig) = build_outbound_signature(&key_a, &did_a, "delete", &path, "localhost");

        let req = Request::builder()
            .method("DELETE")
            .uri(&path)
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig)
            .body(Body::empty())
            .unwrap();

        let resp = build_app(storage.clone()).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);

        // Agent B must still exist.
        assert!(storage.get_agent(&did_b).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn delete_agent_twice_returns_401_on_second() {
        // After a successful DELETE the agent's record (including their public key)
        // is gone, so a second DELETE attempt fails auth with 401 — not 204 or 404.
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key, did) = make_node_key_and_did();
        seed_agent_with_key(&storage, &key).await;

        let path = format!("/v1/agents/{did}");

        let signed_delete = |storage: Arc<dyn Storage>| {
            let key = key.clone();
            let did = did.clone();
            let path = path.clone();
            async move {
                let (date, sig) =
                    build_outbound_signature(&key, &did, "delete", &path, "localhost");
                let req = Request::builder()
                    .method("DELETE")
                    .uri(&path)
                    .header("host", "localhost")
                    .header("date", &date)
                    .header("signature", &sig)
                    .body(Body::empty())
                    .unwrap();
                build_app(storage).oneshot(req).await.unwrap()
            }
        };

        // First delete succeeds.
        let resp = signed_delete(Arc::clone(&storage)).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        // Second delete: no key in storage → RequireAuth returns 401.
        let resp = signed_delete(Arc::clone(&storage)).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
