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
//! After a non-public unit is stored, [`local_fanout`] delivers it to the
//! inboxes of eligible agents registered on this node, and [`remote_fanout`]
//! pushes `limited` units to audience members on remote nodes. Both run in a
//! detached tokio task so they never delay the HTTP response.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use semanticweft::{validate_unit, Graph, SemanticUnit, Visibility};
use semanticweft_agent_core::AgentAddress;
use semanticweft_node_api::{ListResponse, SubgraphResponse};

use crate::{
    error::AppError,
    storage::{Storage, StorageError, UnitFilter},
};

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
/// and returns it with HTTP 201. For `network` and `limited` units, fan-out
/// to registered local recipients runs in a detached background task.
/// For `limited` units, remote push delivery to audience members on other
/// nodes also runs in the same detached task.
pub async fn submit(
    State(state): State<AppState>,
    Json(unit): Json<SemanticUnit>,
) -> Result<impl IntoResponse, AppError> {
    validate_unit(&unit).map_err(|e| AppError::UnprocessableEntity(e.to_string()))?;
    state.storage.put_unit(&unit).await?;

    // Local and remote fan-out run together in a detached task so the HTTP
    // response is never blocked by delivery.
    let storage = Arc::clone(&state.storage);
    let client = state.http_client.clone();
    let unit_fanout = unit.clone();
    tokio::spawn(async move {
        if let Err(e) = local_fanout(Arc::clone(&storage), unit_fanout.clone()).await {
            tracing::warn!("local fan-out error: {e}");
        }
        remote_fanout(client, unit_fanout).await;
    });

    Ok((StatusCode::CREATED, Json(unit)))
}

/// Deliver a submitted unit to the inboxes of eligible agents on this node.
///
/// - `network` units go to all local followers of the author.
/// - `limited` units go to local agents listed in the unit's `audience`.
/// - `public` units are skipped (they enter the sync stream instead).
///
/// Failures per-recipient are logged but do not abort delivery to other
/// recipients.
async fn local_fanout(
    storage: Arc<dyn Storage>,
    unit: SemanticUnit,
) -> Result<(), StorageError> {
    let vis = unit.visibility.clone().unwrap_or(Visibility::Public);

    match vis {
        Visibility::Public => {
            // Public units are discoverable via GET /v1/units and /v1/sync.
            // No inbox delivery needed.
        }
        Visibility::Network => {
            // Deliver to all followers of the author who are registered here.
            let followers = storage.list_followers(&unit.author).await?;
            for did in followers {
                if storage.get_agent(&did).await?.is_some() {
                    if let Err(e) = storage.deliver_to_inbox(&did, &unit).await {
                        tracing::warn!("inbox delivery to {did} failed: {e}");
                    }
                }
            }
        }
        Visibility::Limited => {
            // Deliver to each audience member who is registered here.
            let audience: Vec<String> = unit.audience.clone().unwrap_or_default();
            for did in audience {
                // Local entries have no '@'; skip remote ones here.
                if did.contains('@') {
                    continue;
                }
                if storage.get_agent(&did).await?.is_some() {
                    if let Err(e) = storage.deliver_to_inbox(&did, &unit).await {
                        tracing::warn!("inbox delivery to {did} failed: {e}");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Push a `limited` unit to audience members whose home node is elsewhere.
///
/// Remote entries are identified by the presence of `@` in the audience DID
/// string (format: `did:key:z6Mk…@hostname`). Local entries (no `@`) are
/// handled by [`local_fanout`] and are ignored here.
///
/// For each unique remote hostname the node's `/.well-known/semanticweft`
/// document is fetched once to discover `api_base`. Results are cached in a
/// per-call `HashMap` so multiple recipients on the same node only trigger
/// one discovery request. The unit is then `POST`-ed to each recipient's
/// inbox URL.
///
/// All failures are logged per-recipient; a failure for one recipient never
/// aborts delivery to others.
async fn remote_fanout(client: reqwest::Client, unit: SemanticUnit) {
    // Only `limited` units use push delivery (ADR-0009).
    let vis = unit.visibility.clone().unwrap_or(Visibility::Public);
    if vis != Visibility::Limited {
        return;
    }

    let audience: Vec<String> = unit.audience.clone().unwrap_or_default();

    // Collect remote audience entries (those containing '@'), grouped by hostname.
    let mut by_hostname: HashMap<String, Vec<AgentAddress>> = HashMap::new();
    for entry in &audience {
        if !entry.contains('@') {
            continue; // local entry — handled by local_fanout
        }
        match AgentAddress::parse(entry) {
            Ok(addr) => {
                by_hostname
                    .entry(addr.hostname.clone())
                    .or_default()
                    .push(addr);
            }
            Err(e) => {
                tracing::warn!("remote_fanout: invalid audience entry {entry:?}: {e}");
            }
        }
    }

    if by_hostname.is_empty() {
        return;
    }

    // Discover api_base for each hostname (one request per hostname).
    let mut api_base_cache: HashMap<String, String> = HashMap::new();
    for (hostname, addrs) in &by_hostname {
        // Use well_known_url() from AgentAddress — one URL per hostname.
        let well_known = addrs[0].well_known_url();
        let api_base = match client.get(&well_known).send().await {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(body) => {
                    match serde_json::from_str::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| v.get("api_base")?.as_str().map(str::to_string))
                    {
                        Some(base) => base,
                        None => {
                            tracing::warn!(
                                "remote_fanout: no api_base in discovery document from {hostname}"
                            );
                            continue;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "remote_fanout: failed to read discovery response from {hostname}: {e}"
                    );
                    continue;
                }
            },
            Ok(resp) => {
                tracing::warn!(
                    "remote_fanout: discovery for {hostname} returned status {}",
                    resp.status()
                );
                continue;
            }
            Err(e) => {
                tracing::warn!("remote_fanout: discovery request to {hostname} failed: {e}");
                continue;
            }
        };
        api_base_cache.insert(hostname.clone(), api_base);
    }

    // Deliver to each remote audience member.
    for (hostname, addrs) in &by_hostname {
        let Some(api_base) = api_base_cache.get(hostname) else {
            continue; // discovery failed; already logged above
        };
        for addr in addrs {
            let inbox = addr.inbox_url(api_base);
            match client.post(&inbox).json(&unit).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(
                        "remote_fanout: delivered unit {} to {addr}",
                        unit.id
                    );
                }
                Ok(resp) => {
                    tracing::warn!(
                        "remote_fanout: delivery to {addr} returned status {}",
                        resp.status()
                    );
                }
                Err(e) => {
                    tracing::warn!("remote_fanout: delivery to {addr} failed: {e}");
                }
            }
        }
    }
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
