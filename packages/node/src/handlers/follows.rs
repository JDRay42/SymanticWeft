//! Follow-graph handlers (spec §8.3, ADR-0007).
//!
//! - `POST   /v1/agents/{did}/following`           — follow a target agent.
//! - `DELETE /v1/agents/{did}/following/{target}`  — unfollow a target agent.
//! - `GET    /v1/agents/{did}/following`           — list agents that `did` follows.
//! - `GET    /v1/agents/{did}/followers`           — list agents that follow `did`.
//!
//! Following an unregistered agent is allowed; the relationship is stored
//! without requiring the target to be registered on this node (they may be
//! on a different node). The follower, however, must be registered here so
//! the node knows where to deliver fan-out units.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use semanticweft_node_api::{FollowEntry, FollowListResponse, FollowRequest};

use crate::error::AppError;
use crate::middleware::auth::RequireAuth;

use super::AppState;

/// `POST /v1/agents/{did}/following` — follow a target agent.
///
/// Returns 400 if `follower_did` in the body does not match the `{did}` path
/// parameter. Returns 401 if unauthenticated. Returns 403 if the authenticated
/// DID does not match `{did}`. Returns 404 if the follower agent is not
/// registered on this node. Returns 204 on success.
pub async fn follow(
    State(state): State<AppState>,
    Path(did): Path<String>,
    auth: RequireAuth,
    Json(req): Json<FollowRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.follower_did != did {
        return Err(AppError::BadRequest(
            "follower_did in body must match the {did} path parameter".into(),
        ));
    }

    if auth.did != did {
        return Err(AppError::Forbidden(
            "cannot follow on behalf of a different agent".into(),
        ));
    }

    // The follower must be registered here so fan-out has an inbox to target.
    state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    state
        .storage
        .add_follow(&req.follower_did, &req.target_did)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /v1/agents/{did}/following/{target}` — unfollow a target agent.
///
/// Returns 401 if unauthenticated. Returns 403 if the authenticated DID does
/// not match `{did}`. Idempotent: returns 204 even if the relationship did
/// not exist.
pub async fn unfollow(
    State(state): State<AppState>,
    Path((did, target)): Path<(String, String)>,
    auth: RequireAuth,
) -> Result<impl IntoResponse, AppError> {
    if auth.did != did {
        return Err(AppError::Forbidden(
            "cannot unfollow on behalf of a different agent".into(),
        ));
    }
    state.storage.remove_follow(&did, &target).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /v1/agents/{did}/following` — list agents that `did` follows.
pub async fn list_following(
    State(state): State<AppState>,
    Path(did): Path<String>,
) -> Result<Json<FollowListResponse>, AppError> {
    let dids = state.storage.list_following(&did).await?;
    let items = dids
        .into_iter()
        .map(|d| FollowEntry {
            did: d,
            inbox_url: None,
        })
        .collect();
    Ok(Json(FollowListResponse {
        items,
        next_cursor: None,
    }))
}

/// `GET /v1/agents/{did}/followers` — list agents that follow `did`.
pub async fn list_followers(
    State(state): State<AppState>,
    Path(did): Path<String>,
) -> Result<Json<FollowListResponse>, AppError> {
    let dids = state.storage.list_followers(&did).await?;
    let items = dids
        .into_iter()
        .map(|d| FollowEntry {
            did: d,
            inbox_url: None,
        })
        .collect();
    Ok(Json(FollowListResponse {
        items,
        next_cursor: None,
    }))
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
    use semanticweft_node_api::AgentProfile;
    use tower::ServiceExt;

    use crate::{
        config::NodeConfig,
        middleware::auth::build_outbound_signature,
        router::build_router,
        storage::{memory::MemoryStorage, Storage},
    };

    fn make_key_and_did() -> (SigningKey, String) {
        let key = SigningKey::generate(&mut OsRng);
        let pub_bytes = key.verifying_key().to_bytes();
        let mut mc = vec![0xed_u8, 0x01];
        mc.extend_from_slice(&pub_bytes);
        let did = format!("did:key:z{}", bs58::encode(&mc).into_string());
        (key, did)
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

    async fn seed(storage: &Arc<dyn Storage>, key: &SigningKey) -> String {
        let pub_bytes = key.verifying_key().to_bytes();
        let mut mc = vec![0xed_u8, 0x01];
        mc.extend_from_slice(&pub_bytes);
        let encoded = bs58::encode(&mc).into_string();
        let did = format!("did:key:z{encoded}");
        storage
            .put_agent(&AgentProfile {
                did: did.clone(),
                inbox_url: format!("http://localhost/v1/agents/{did}/inbox"),
                display_name: None,
                public_key: Some(format!("z{encoded}")),
            })
            .await
            .unwrap();
        did
    }

    // -----------------------------------------------------------------------
    // POST /v1/agents/{did}/following
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn follow_unauthenticated_returns_401() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (_, did) = make_key_and_did();
        let path = format!("/v1/agents/{did}/following");

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({ "follower_did": did, "target_did": "did:key:zTarget" })
                    .to_string(),
            ))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn follow_wrong_did_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key_a, did_a) = make_key_and_did();
        let (_, did_b) = make_key_and_did();
        seed(&storage, &key_a).await;

        // Agent A tries to follow on behalf of Agent B.
        let path = format!("/v1/agents/{did_b}/following");
        let (date, sig) = build_outbound_signature(&key_a, &did_a, "post", &path, "localhost");

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig)
            .body(Body::from(
                serde_json::json!({ "follower_did": did_b, "target_did": "did:key:zTarget" })
                    .to_string(),
            ))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn follow_success_returns_204() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key, did) = make_key_and_did();
        seed(&storage, &key).await;

        let path = format!("/v1/agents/{did}/following");
        let (date, sig) = build_outbound_signature(&key, &did, "post", &path, "localhost");

        let req = Request::builder()
            .method("POST")
            .uri(&path)
            .header("content-type", "application/json")
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig)
            .body(Body::from(
                serde_json::json!({ "follower_did": did, "target_did": "did:key:zTarget" })
                    .to_string(),
            ))
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    // -----------------------------------------------------------------------
    // DELETE /v1/agents/{did}/following/{target}
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn unfollow_unauthenticated_returns_401() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (_, did) = make_key_and_did();
        let path = format!("/v1/agents/{did}/following/did:key:zTarget");

        let req = Request::builder()
            .method("DELETE")
            .uri(&path)
            .body(Body::empty())
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn unfollow_wrong_did_returns_403() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key_a, did_a) = make_key_and_did();
        let (_, did_b) = make_key_and_did();
        seed(&storage, &key_a).await;

        // Agent A tries to unfollow on behalf of Agent B.
        let path = format!("/v1/agents/{did_b}/following/did:key:zTarget");
        let (date, sig) = build_outbound_signature(&key_a, &did_a, "delete", &path, "localhost");

        let req = Request::builder()
            .method("DELETE")
            .uri(&path)
            .header("host", "localhost")
            .header("date", &date)
            .header("signature", &sig)
            .body(Body::empty())
            .unwrap();

        let resp = build_app(storage).oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn unfollow_success_returns_204() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let (key, did) = make_key_and_did();
        seed(&storage, &key).await;
        // Pre-load a follow relationship.
        storage.add_follow(&did, "did:key:zTarget").await.unwrap();

        let path = format!("/v1/agents/{did}/following/did:key:zTarget");
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

        // Relationship must be gone.
        assert!(!storage.is_following(&did, "did:key:zTarget").await.unwrap());
    }
}
