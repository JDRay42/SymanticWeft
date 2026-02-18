//! Unit handlers — submit, retrieve, list, subgraph, and sync (spec §5).
//!
//! # Visibility model
//!
//! Business logic, not storage, decides what a caller may see:
//! - Unauthenticated callers receive `public` units only.
//! - TODO: Extend once auth middleware is in place (network + limited units).
//!
//! # Fan-out
//!
//! When a `network` or `limited` unit is submitted, the node is responsible for
//! delivering it to the relevant agents' inboxes. Fan-out is marked TODO here
//! and will be added in the federation implementation phase.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use semanticweft::{validate_unit, Graph, SemanticUnit, Visibility};
use semanticweft_node_api::{ListResponse, SubgraphResponse};

use crate::{error::AppError, storage::UnitFilter};

use super::AppState;

// ---------------------------------------------------------------------------
// Query param structs (serde-compatible for axum Query extractor)
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/units` and `GET /v1/sync`.
///
/// `type` accepts comma-separated unit type names, e.g. `?type=assertion,inference`.
#[derive(Debug, Deserialize, Default)]
pub struct UnitQueryParams {
    /// Comma-separated unit types to include (e.g. `assertion,inference`).
    #[serde(rename = "type")]
    pub unit_type: Option<String>,

    /// Filter by author DID.
    pub author: Option<String>,

    /// ISO 8601 lower bound on `created_at`.
    pub since: Option<String>,

    /// Keyset pagination cursor (UUIDv7 `id` of the last seen unit).
    pub after: Option<String>,

    /// Page size (1–500, default 50).
    pub limit: Option<u32>,
}

/// Query parameters for `GET /v1/units/{id}/subgraph`.
#[derive(Debug, Deserialize, Default)]
pub struct SubgraphQueryParams {
    /// Maximum traversal depth in both directions. Defaults to 10.
    pub depth: Option<u32>,
}

// ---------------------------------------------------------------------------
// POST /v1/units
// ---------------------------------------------------------------------------

/// `POST /v1/units` — submit a new unit.
///
/// Validates the unit against the spec, rejects duplicates, stores the unit,
/// and returns it with HTTP 201. Fan-out to followers is TODO.
pub async fn submit(
    State(state): State<AppState>,
    Json(unit): Json<SemanticUnit>,
) -> Result<impl IntoResponse, AppError> {
    validate_unit(&unit).map_err(|e| AppError::UnprocessableEntity(e.to_string()))?;
    state.storage.put_unit(&unit).await?;

    // TODO: fan-out to followers when visibility is Network or Limited.

    Ok((StatusCode::CREATED, Json(unit)))
}

// ---------------------------------------------------------------------------
// GET /v1/units/:id
// ---------------------------------------------------------------------------

/// `GET /v1/units/{id}` — retrieve a unit by its UUIDv7 ID.
///
/// Returns 404 for non-public units (visibility enforcement; see module docs).
pub async fn get_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SemanticUnit>, AppError> {
    let unit = state
        .storage
        .get_unit(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("unit {id} not found")))?;

    // Unauthenticated: only serve public units.
    // TODO: expand when auth middleware is added.
    let vis = unit.visibility.as_ref().unwrap_or(&Visibility::Public);
    if *vis != Visibility::Public {
        return Err(AppError::NotFound(format!("unit {id} not found")));
    }

    Ok(Json(unit))
}

// ---------------------------------------------------------------------------
// GET /v1/units
// ---------------------------------------------------------------------------

/// `GET /v1/units` — list units with optional filtering and keyset pagination.
pub async fn list(
    State(state): State<AppState>,
    Query(params): Query<UnitQueryParams>,
) -> Result<Json<ListResponse>, AppError> {
    let filter = build_filter(params, vec![Visibility::Public]);
    let (units, has_more) = state.storage.list_units(&filter).await?;
    Ok(Json(ListResponse::from_page(units, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/sync
// ---------------------------------------------------------------------------

/// `GET /v1/sync` — the federation pull endpoint.
///
/// Semantically identical to `GET /v1/units` but signals federation intent.
/// Always returns public units only (nodes do not receive private data).
pub async fn sync(
    State(state): State<AppState>,
    Query(params): Query<UnitQueryParams>,
) -> Result<Json<ListResponse>, AppError> {
    let filter = build_filter(params, vec![Visibility::Public]);
    let (units, has_more) = state.storage.list_units(&filter).await?;
    Ok(Json(ListResponse::from_page(units, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/units/:id/subgraph
// ---------------------------------------------------------------------------

/// `GET /v1/units/{id}/subgraph` — retrieve the connected subgraph around a unit.
///
/// Traverses outgoing (referenced) and incoming (referencing) edges up to
/// `depth` hops in each direction. Unreachable or non-public units are
/// silently omitted.
pub async fn subgraph(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<SubgraphQueryParams>,
) -> Result<Json<SubgraphResponse>, AppError> {
    // Verify the root unit exists and is visible.
    let root = state
        .storage
        .get_unit(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("unit {id} not found")))?;

    let vis = root.visibility.as_ref().unwrap_or(&Visibility::Public);
    if *vis != Visibility::Public {
        return Err(AppError::NotFound(format!("unit {id} not found")));
    }

    let max_depth = params.depth.unwrap_or(10).min(50) as usize;

    // BFS: load the connected subgraph from storage, depth-limited.
    let mut graph = Graph::new();
    graph.add(root);

    let mut frontier: Vec<String> = vec![id.clone()];

    for _ in 0..max_depth {
        if frontier.is_empty() {
            break;
        }
        let mut next_frontier: Vec<String> = Vec::new();

        for node_id in &frontier {
            // Outgoing edges: units that node_id references.
            let outgoing_ids: Vec<String> = graph
                .get(node_id)
                .and_then(|u| u.references.as_ref())
                .map(|refs| refs.iter().map(|r| r.id.clone()).collect())
                .unwrap_or_default();

            for ref_id in outgoing_ids {
                if graph.get(&ref_id).is_none() {
                    if let Ok(Some(u)) = state.storage.get_unit(&ref_id).await {
                        next_frontier.push(u.id.clone());
                        graph.add(u);
                    }
                }
            }

            // Incoming edges: units that reference node_id.
            let incoming = state.storage.get_referencing_units(node_id).await?;
            for u in incoming {
                if graph.get(&u.id).is_none() {
                    next_frontier.push(u.id.clone());
                    graph.add(u);
                }
            }
        }

        frontier = next_frontier;
    }

    // Extract the connected subgraph using the core Graph abstraction.
    let subgraph = graph.subgraph(&id);
    let units = subgraph.units().cloned().collect();

    Ok(Json(SubgraphResponse { units }))
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn build_filter(params: UnitQueryParams, visibilities: Vec<Visibility>) -> UnitFilter {
    let limit = params.limit.map(|l| l.clamp(1, 500)).unwrap_or(50);

    let unit_types: Vec<semanticweft::UnitType> = params
        .unit_type
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.trim().parse().ok())
        .collect();

    UnitFilter {
        unit_types,
        author: params.author,
        since: params.since,
        after: params.after,
        limit,
        visibilities,
    }
}
