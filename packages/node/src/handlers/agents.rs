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
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use semanticweft_node_api::{AgentProfile, InboxResponse, RegisterRequest};

use crate::error::AppError;

use super::AppState;

/// `POST /v1/agents/{did}` — register or update an agent.
///
/// The `did` in the path and in the request body must match; returns 400
/// if they differ. Returns 201 with the stored profile.
pub async fn register(
    State(state): State<AppState>,
    Path(did): Path<String>,
    Json(req): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    if req.did != did {
        return Err(AppError::BadRequest(
            "did in request body must match the {did} path parameter".into(),
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
/// Currently returns an empty inbox; fan-out delivery is not yet implemented.
pub async fn inbox(
    State(state): State<AppState>,
    Path(did): Path<String>,
) -> Result<Json<InboxResponse>, AppError> {
    state
        .storage
        .get_agent(&did)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found")))?;

    // TODO: implement per-agent inbox table and fan-out delivery.
    Ok(Json(InboxResponse::empty()))
}
