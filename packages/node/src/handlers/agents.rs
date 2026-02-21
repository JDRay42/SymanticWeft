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
use semanticweft_node_api::{AgentProfile, InboxResponse, RegisterRequest};

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
