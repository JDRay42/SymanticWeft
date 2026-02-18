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

use super::AppState;

/// `POST /v1/agents/{did}/following` — follow a target agent.
///
/// Returns 400 if `follower_did` in the body does not match the `{did}` path
/// parameter. Returns 404 if the follower agent is not registered on this node.
/// Returns 204 on success.
pub async fn follow(
    State(state): State<AppState>,
    Path(did): Path<String>,
    Json(req): Json<FollowRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.follower_did != did {
        return Err(AppError::BadRequest(
            "follower_did in body must match the {did} path parameter".into(),
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
/// Idempotent: returns 204 even if the relationship did not exist.
pub async fn unfollow(
    State(state): State<AppState>,
    Path((did, target)): Path<(String, String)>,
) -> Result<impl IntoResponse, AppError> {
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
