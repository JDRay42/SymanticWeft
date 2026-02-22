//! Unit handlers — submit, retrieve, list, subgraph, and sync (spec §5).
//!
//! # Visibility model
//!
//! Business logic, not storage, decides what a caller may see:
//! - Unauthenticated callers receive `public` units only.
//! - Authenticated callers may also receive `network` units if they follow the author.
//! - `limited` units are visible only if the caller DID is in the unit's audience.
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
    http::{header, HeaderMap, StatusCode},
    response::{sse::Event, sse::Sse, IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use semanticweft::{validate_unit, Graph, Reference, RelType, SemanticUnit, UnitType, Visibility};
use semanticweft_agent_core::AgentAddress;
use semanticweft_node_api::{ListResponse, SubgraphResponse};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use crate::{
    error::AppError,
    middleware::auth::{build_outbound_signature, OptionalAuth},
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
    auth: OptionalAuth,
    Json(unit): Json<SemanticUnit>,
) -> Result<Response, AppError> {
    validate_unit(&unit).map_err(|e| AppError::UnprocessableEntity(e.to_string()))?;
    if unit.proof.is_some() {
        semanticweft::verify_proof(&unit).map_err(|e| AppError::BadRequest(e.to_string()))?;
    }

    // Non-public units require authentication as the unit's author.
    let vis = unit.visibility.as_ref().unwrap_or(&Visibility::Public);
    if *vis != Visibility::Public {
        let caller_did = auth.0.as_deref().ok_or_else(|| {
            AppError::Unauthorized("authentication required for non-public units".into())
        })?;
        if caller_did != unit.author {
            return Err(AppError::Forbidden(
                "you may only submit units authored by your own DID".into(),
            ));
        }
    }

    match state.storage.put_unit(&unit).await {
        Ok(()) => {}
        Err(StorageError::Conflict(_)) => {
            // The spec (§5.1) requires that re-submission of a unit with the
            // same `id` and identical content is idempotent (200 OK).
            // Re-submission with different content is a true conflict (409).
            let existing = state
                .storage
                .get_unit(&unit.id)
                .await?
                .ok_or_else(|| AppError::Internal("conflict on put_unit but unit not found".into()))?;
            if existing == unit {
                // Idempotent re-submission: return the stored unit, no fan-out.
                return Ok((StatusCode::OK, Json(existing)).into_response());
            }
            return Err(AppError::Conflict(format!(
                "unit {} already exists with different content",
                unit.id
            )));
        }
        Err(e) => return Err(AppError::from(e)),
    }

    // Broadcast public units to live SSE subscribers.
    let unit_vis = unit.visibility.as_ref().unwrap_or(&Visibility::Public);
    if *unit_vis == Visibility::Public {
        let _ = state.sse_tx.send(Arc::new(unit.clone()));
    }

    // Local and remote fan-out run together in a detached task so the HTTP
    // response is never blocked by delivery.
    let storage = Arc::clone(&state.storage);
    let client = state.http_client.clone();
    let node_did = state.config.node_id.clone();
    let node_api_base = state.config.api_base.clone();
    let signing_key = Arc::clone(&state.signing_key);
    let unit_fanout = unit.clone();
    tokio::spawn(async move {
        if let Err(e) = local_fanout(Arc::clone(&storage), unit_fanout.clone()).await {
            tracing::warn!("local fan-out error: {e}");
        }
        remote_fanout(client, unit_fanout, storage, node_did, node_api_base, signing_key).await;
    });

    Ok((StatusCode::CREATED, Json(unit)).into_response())
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

/// Push a non-public unit to eligible recipients on remote nodes.
///
/// Handles two visibility modes:
///
/// - **`network`**: the unit is pushed to each follower of the author who has
///   an agent profile registered on this node with an `inbox_url` that does
///   not originate from this node (i.e. their home node is elsewhere). Local
///   followers are already handled by [`local_fanout`].
///
/// - **`limited`**: audience entries containing `@` (format
///   `did:key:z6Mk…@hostname`) identify recipients on remote nodes. For each
///   unique hostname the node's `/.well-known/semanticweft` document is
///   fetched once to discover `api_base`; results are cached so multiple
///   recipients on the same node only trigger one discovery request.
///
/// **Delivery failure notifications**: when a push attempt fails (network
/// error, 4xx, 5xx), this node generates a `constraint` unit and delivers it
/// to the original author's local inbox — if the author is registered here.
/// The notification carries `references: [{ id: <original-unit-id>, rel:
/// "notifies" }]` so the author can correlate it to the triggering unit.
///
/// All failures are handled per-recipient; a failure for one recipient never
/// aborts delivery to others.
async fn remote_fanout(
    client: reqwest::Client,
    unit: SemanticUnit,
    storage: Arc<dyn Storage>,
    node_did: String,
    node_api_base: String,
    signing_key: std::sync::Arc<ed25519_dalek::SigningKey>,
) {
    let vis = unit.visibility.clone().unwrap_or(Visibility::Public);

    match vis {
        Visibility::Public => return,

        Visibility::Network => {
            // Push to followers whose registered inbox_url belongs to a different node.
            let followers = match storage.list_followers(&unit.author).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!("remote_fanout: failed to list followers: {e}");
                    return;
                }
            };

            for follower_did in &followers {
                let profile = match storage.get_agent(follower_did).await {
                    Ok(Some(p)) => p,
                    Ok(None) => continue, // not registered here; can't reach them
                    Err(e) => {
                        tracing::warn!("remote_fanout: get_agent({follower_did}) failed: {e}");
                        continue;
                    }
                };

                // Skip followers whose inbox is served by this node.
                if profile.inbox_url.starts_with(&node_api_base) {
                    continue;
                }

                push_to_inbox(
                    &client,
                    &unit,
                    &profile.inbox_url,
                    &signing_key,
                    &node_did,
                    &storage,
                )
                .await;
            }
            return;
        }

        Visibility::Limited => {} // handled below
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
                            notify_author_of_failure(
                                &storage,
                                &node_did,
                                &unit,
                                &format!("@{hostname}"),
                                &format!("discovery document from {hostname} missing api_base"),
                            )
                            .await;
                            continue;
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "remote_fanout: failed to read discovery response from {hostname}: {e}"
                    );
                    notify_author_of_failure(
                        &storage,
                        &node_did,
                        &unit,
                        &format!("@{hostname}"),
                        &format!("could not read discovery response from {hostname}: {e}"),
                    )
                    .await;
                    continue;
                }
            },
            Ok(resp) => {
                let status = resp.status().as_u16();
                tracing::warn!(
                    "remote_fanout: discovery for {hostname} returned status {status}"
                );
                notify_author_of_failure(
                    &storage,
                    &node_did,
                    &unit,
                    &format!("@{hostname}"),
                    &format!("discovery request to {hostname} returned HTTP {status}"),
                )
                .await;
                continue;
            }
            Err(e) => {
                tracing::warn!("remote_fanout: discovery request to {hostname} failed: {e}");
                notify_author_of_failure(
                    &storage,
                    &node_did,
                    &unit,
                    &format!("@{hostname}"),
                    &format!("could not reach {hostname} for discovery: {e}"),
                )
                .await;
                continue;
            }
        };
        api_base_cache.insert(hostname.clone(), api_base);
    }

    // Deliver to each remote audience member.
    for (hostname, addrs) in &by_hostname {
        let Some(api_base) = api_base_cache.get(hostname) else {
            continue; // discovery failed; already notified above
        };
        for addr in addrs {
            let inbox_url = addr.inbox_url(api_base);
            push_to_inbox(&client, &unit, &inbox_url, &signing_key, &node_did, &storage).await;
        }
    }
}

/// POST a unit to a remote agent inbox with an HTTP Signature, logging any failure.
///
/// Failure notifications are delivered to the original author's local inbox
/// (if they are registered here) via [`notify_author_of_failure`].
async fn push_to_inbox(
    client: &reqwest::Client,
    unit: &SemanticUnit,
    inbox_url: &str,
    signing_key: &ed25519_dalek::SigningKey,
    node_did: &str,
    storage: &Arc<dyn Storage>,
) {
    let (req_host, req_path) = match reqwest::Url::parse(inbox_url) {
        Ok(parsed) => {
            let host = parsed.host_str().unwrap_or("localhost").to_string();
            let path = if let Some(q) = parsed.query() {
                format!("{}?{}", parsed.path(), q)
            } else {
                parsed.path().to_string()
            };
            (host, path)
        }
        Err(_) => {
            tracing::warn!("push_to_inbox: cannot parse URL {inbox_url}");
            return;
        }
    };

    let (date_val, sig_val) =
        build_outbound_signature(signing_key, node_did, "post", &req_path, &req_host);

    match client
        .post(inbox_url)
        .header("date", &date_val)
        .header("signature", &sig_val)
        .header("host", &req_host)
        .json(unit)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            tracing::debug!("push_to_inbox: delivered unit {} to {inbox_url}", unit.id);
        }
        Ok(resp) => {
            let status = resp.status().as_u16();
            tracing::warn!("push_to_inbox: delivery to {inbox_url} returned HTTP {status}");
            notify_author_of_failure(
                storage,
                node_did,
                unit,
                inbox_url,
                &format!("remote inbox returned HTTP {status}"),
            )
            .await;
        }
        Err(e) => {
            tracing::warn!("push_to_inbox: delivery to {inbox_url} failed: {e}");
            notify_author_of_failure(
                storage,
                node_did,
                unit,
                inbox_url,
                &format!("delivery failed: {e}"),
            )
            .await;
        }
    }
}

/// Deliver a delivery-failure notification to the original author's local inbox.
///
/// The notification is a `constraint` unit authored by the node itself, with
/// `visibility: limited` and `audience: [author]`. The `references` field
/// carries `{ id: <original-unit-id>, rel: "notifies" }` as the association
/// fingerprint so the author can correlate it to the unit that triggered it.
///
/// If the author is not registered on this node the notification is silently
/// dropped — there is nowhere local to deliver it. All storage errors are
/// logged and swallowed so a notification failure never disrupts the caller.
async fn notify_author_of_failure(
    storage: &Arc<dyn Storage>,
    node_did: &str,
    original_unit: &SemanticUnit,
    recipient: &str,
    reason: &str,
) {
    // Only notify if the author is registered locally.
    match storage.get_agent(&original_unit.author).await {
        Ok(Some(_)) => {}
        Ok(None) => return, // author not local; nowhere to deliver
        Err(e) => {
            tracing::warn!(
                "remote_fanout: could not check author registration for notification: {e}"
            );
            return;
        }
    }

    let mut notification = SemanticUnit::new(
        UnitType::Constraint,
        format!(
            "Delivery of unit {} to recipient {recipient} failed: {reason}",
            original_unit.id
        ),
        node_did,
    );
    notification.references = Some(vec![Reference {
        id: original_unit.id.clone(),
        rel: RelType::Notifies,
    }]);
    notification.visibility = Some(Visibility::Limited);
    notification.audience = Some(vec![original_unit.author.clone()]);

    if let Err(e) = storage
        .deliver_to_inbox(&original_unit.author, &notification)
        .await
    {
        tracing::warn!(
            "remote_fanout: failed to deliver failure notification to {}: {e}",
            original_unit.author
        );
    }
}

// ---------------------------------------------------------------------------
// GET /v1/units/:id
// ---------------------------------------------------------------------------

/// `GET /v1/units/{id}` — retrieve a unit by its UUIDv7 ID.
///
/// Visibility is enforced per auth state (see module docs).
pub async fn get_by_id(
    State(state): State<AppState>,
    Path(id): Path<String>,
    auth: OptionalAuth,
) -> Result<Json<SemanticUnit>, AppError> {
    let unit = state
        .storage
        .get_unit(&id)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("unit {id} not found")))?;

    let vis = unit.visibility.as_ref().unwrap_or(&Visibility::Public);
    match vis {
        Visibility::Public => {} // always visible
        Visibility::Network => {
            // Visible to authenticated followers of the author.
            let caller_did = auth.0.as_deref().ok_or_else(|| {
                AppError::NotFound(format!("unit {id} not found"))
            })?;
            let is_follower = state
                .storage
                .is_following(caller_did, &unit.author)
                .await
                .unwrap_or(false);
            if !is_follower {
                return Err(AppError::NotFound(format!("unit {id} not found")));
            }
        }
        Visibility::Limited => {
            // Visible only if caller DID is in unit.audience.
            let caller_did = auth.0.as_deref().ok_or_else(|| {
                AppError::NotFound(format!("unit {id} not found"))
            })?;
            let audience = unit.audience.as_deref().unwrap_or(&[]);
            if !audience.iter().any(|a| a == caller_did) {
                return Err(AppError::NotFound(format!("unit {id} not found")));
            }
        }
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
    auth: OptionalAuth,
) -> Result<Json<ListResponse>, AppError> {
    let (visibilities, network_for_authors) = if let Some(ref caller_did) = auth.0 {
        // Authenticated callers may see network units, but only from authors
        // they follow. Fetch the follow list and pass it to the filter.
        let following = state.storage.list_following(caller_did).await?;
        if following.is_empty() {
            (vec![Visibility::Public], vec![])
        } else {
            (vec![Visibility::Public, Visibility::Network], following)
        }
    } else {
        (vec![Visibility::Public], vec![])
    };

    let mut filter = build_filter(params, visibilities);
    filter.network_for_authors = network_for_authors;
    let (units, has_more) = state.storage.list_units(&filter).await?;
    Ok(Json(ListResponse::from_page(units, has_more)))
}

// ---------------------------------------------------------------------------
// GET /v1/sync
// ---------------------------------------------------------------------------

/// `GET /v1/sync` — the federation pull endpoint.
///
/// Returns public units for node-to-node replication. Supports two response
/// formats negotiated via the `Accept` header:
///
/// - **`application/json`** (default): returns a `ListResponse` JSON object
///   with `units` array and `has_more` flag, suitable for polling clients.
///
/// - **`text/event-stream`**: returns a **persistent** Server-Sent Events
///   stream. The handler first replays all historical public units from the
///   cursor position (same semantics as the JSON path), then keeps the
///   connection alive and pushes each newly submitted public unit in real
///   time as it arrives. Clients should use the `id` of the last received
///   event as the `Last-Event-ID` header on reconnect to resume without
///   gaps. Slow consumers that fall more than [`SSE_CHANNEL_CAPACITY`] events
///   behind will receive a synthetic `event: lag` event and must reconnect
///   from their last known cursor.
///
/// The `Last-Event-ID` header (when present) is used as the keyset
/// pagination cursor, identical to the `?after=` query parameter.
pub async fn sync(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(mut params): Query<UnitQueryParams>,
) -> Result<Response, AppError> {
    // `Last-Event-ID` from a reconnecting SSE client acts as the cursor.
    if params.after.is_none() {
        if let Some(last_id) = headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
            params.after = Some(last_id.to_string());
        }
    }

    let wants_sse = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|accept| accept.contains("text/event-stream"))
        .unwrap_or(false);

    let filter = build_filter(params, vec![Visibility::Public]);

    if wants_sse {
        // Subscribe BEFORE querying storage so we don't miss units submitted
        // between the DB read and the channel subscription.
        let rx = state.sse_tx.subscribe();

        let (historical_units, _has_more) = state.storage.list_units(&filter).await?;

        // Build a stream: first the historical batch, then live events.
        let historical = tokio_stream::iter(historical_units).map(|u| {
            let json = serde_json::to_string(&u).unwrap_or_else(|_| "{}".to_string());
            Ok::<Event, std::convert::Infallible>(Event::default().id(u.id).data(json))
        });

        let live = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(unit) => {
                let json = serde_json::to_string(&*unit).unwrap_or_else(|_| "{}".to_string());
                Some(Ok::<Event, std::convert::Infallible>(
                    Event::default().id(unit.id.clone()).data(json),
                ))
            }
            Err(_lagged) => {
                // The subscriber fell behind. Send a lag event so the client
                // knows to reconnect from its last cursor.
                Some(Ok(Event::default().event("lag").data(
                    r#"{"error":"subscriber_lagged","action":"reconnect_from_cursor"}"#,
                )))
            }
        });

        let stream = historical.chain(live);
        Ok(Sse::new(stream)
            .keep_alive(axum::response::sse::KeepAlive::new())
            .into_response())
    } else {
        let (units, has_more) = state.storage.list_units(&filter).await?;
        Ok(Json(ListResponse::from_page(units, has_more)).into_response())
    }
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
        network_for_authors: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use tower::ServiceExt;
    use semanticweft::{sign_unit, SemanticUnit, UnitType};

    use crate::config::NodeConfig;
    use crate::router::build_router;
    use crate::storage::memory::MemoryStorage;

    fn build_app() -> axum::Router {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = NodeConfig::from_env();
        let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
        build_router(storage, config, signing_key).0
    }

    fn make_unit() -> SemanticUnit {
        SemanticUnit::new(
            UnitType::Assertion,
            "Test content for proof integration test.",
            "did:key:z6MkTest",
        )
    }

    fn make_signing_key_and_did() -> (SigningKey, String) {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let pub_bytes = verifying_key.to_bytes();
        let mut multicodec = vec![0xed_u8, 0x01];
        multicodec.extend_from_slice(&pub_bytes);
        let did = format!("did:key:z{}", bs58::encode(&multicodec).into_string());
        (signing_key, did)
    }

    #[tokio::test]
    async fn submit_unit_without_proof_returns_201() {
        let app = build_app();
        let unit = make_unit();
        let body = serde_json::to_string(&unit).unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/units")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_unit_with_valid_proof_returns_201() {
        let app = build_app();
        let mut unit = make_unit();
        let (signing_key, did) = make_signing_key_and_did();
        sign_unit(&mut unit, &signing_key, &did).unwrap();
        let body = serde_json::to_string(&unit).unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/units")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn submit_unit_with_tampered_proof_returns_400() {
        let app = build_app();
        let mut unit = make_unit();
        let (signing_key, did) = make_signing_key_and_did();
        sign_unit(&mut unit, &signing_key, &did).unwrap();
        // Tamper with content after signing
        unit.content = "tampered content".into();
        let body = serde_json::to_string(&unit).unwrap();
        let req = Request::builder()
            .method("POST")
            .uri("/v1/units")
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn sync_with_sse_accept_returns_event_stream() {
        use std::time::Duration;

        let app = build_app();

        // Submit a public unit first so it appears in the historical replay.
        let unit = make_unit();
        let unit_id = unit.id.clone();
        let submit_req = Request::builder()
            .method("POST")
            .uri("/v1/units")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&unit).unwrap()))
            .unwrap();
        let submit_resp = app.clone().oneshot(submit_req).await.unwrap();
        assert_eq!(submit_resp.status(), StatusCode::CREATED);

        // Now GET /v1/sync with Accept: text/event-stream.
        let sync_req = Request::builder()
            .method("GET")
            .uri("/v1/sync")
            .header("accept", "text/event-stream")
            .body(Body::empty())
            .unwrap();
        let sync_resp = app.oneshot(sync_req).await.unwrap();

        assert_eq!(sync_resp.status(), StatusCode::OK);
        let ct = sync_resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("text/event-stream"), "content-type should be text/event-stream, got {ct}");

        // The SSE stream is now persistent (live). We read with a short deadline
        // to capture the historical replay frames without blocking forever.
        use http_body_util::BodyExt;
        let body_bytes = tokio::time::timeout(
            Duration::from_millis(200),
            sync_resp.into_body().collect(),
        )
        .await
        .map(|r| r.unwrap().to_bytes())
        .unwrap_or_else(|_| bytes::Bytes::new());
        let body_text = std::str::from_utf8(&body_bytes).unwrap_or("");

        // The unit ID should appear as an SSE event id in the historical replay.
        assert!(
            body_text.contains(&format!("id:{unit_id}"))
                || body_text.contains(&format!("id: {unit_id}")),
            "SSE body should contain the submitted unit id ({unit_id}):\n{body_text}"
        );
        // Each event's data should be present.
        assert!(
            body_text.contains("data:") || body_text.contains("data: "),
            "SSE body should contain 'data:':\n{body_text}"
        );
        // The old snapshot sentinel must NOT appear — it indicates a regression.
        assert!(
            !body_text.contains("event: end"),
            "persistent SSE must not emit 'event: end' sentinel"
        );
    }

    #[tokio::test]
    async fn sync_without_sse_accept_returns_json() {
        let app = build_app();
        let req = Request::builder()
            .method("GET")
            .uri("/v1/sync")
            .header("accept", "application/json")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("application/json"), "should be JSON when SSE not requested, got {ct}");
    }
}
