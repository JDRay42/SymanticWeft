//! Agent registration and inbox handlers (spec §8, ADR-0007).
//!
//! - `POST /v1/agents/{did}` — register or update an agent profile.
//! - `GET  /v1/agents/{did}` — retrieve an agent profile.
//! - `GET  /v1/agents/{did}/inbox` — retrieve the agent's pending inbox items.
//!
//! # Inbox
//!
//! The inbox endpoint currently returns an empty response. Full fan-out
//! delivery (storing units in per-agent inbox queues and letting agents drain
//! them) is deferred to the federation implementation phase.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use semanticweft_node_api::{AgentProfile, InboxResponse, RegisterRequest};

use crate::error::AppError;
use crate::middleware::auth::RequireAuth;

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
