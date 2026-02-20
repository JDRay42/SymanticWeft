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
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use semanticweft::{validate_unit, Graph, Reference, RelType, SemanticUnit, UnitType, Visibility};
use semanticweft_agent_core::AgentAddress;
use semanticweft_node_api::{ListResponse, SubgraphResponse};

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
) -> Result<impl IntoResponse, AppError> {
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

    state.storage.put_unit(&unit).await?;

    // Local and remote fan-out run together in a detached task so the HTTP
    // response is never blocked by delivery.
    let storage = Arc::clone(&state.storage);
    let client = state.http_client.clone();
    let node_did = state.config.node_id.clone();
    let signing_key = Arc::clone(&state.signing_key);
    let unit_fanout = unit.clone();
    tokio::spawn(async move {
        if let Err(e) = local_fanout(Arc::clone(&storage), unit_fanout.clone()).await {
            tracing::warn!("local fan-out error: {e}");
        }
        remote_fanout(client, unit_fanout, storage, node_did, signing_key).await;
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
/// **Delivery failure notifications**: when a push attempt fails (network
/// error, 4xx, 5xx), this node generates a `constraint` unit and delivers it
/// to the original author's local inbox — if the author is registered here.
/// The notification carries `references: [{ id: <original-unit-id>, rel:
/// "notifies" }]` as the association fingerprint so the author can correlate
/// it to the unit that triggered it.
///
/// All failures are handled per-recipient; a failure for one recipient never
/// aborts delivery to others.
async fn remote_fanout(
    client: reqwest::Client,
    unit: SemanticUnit,
    storage: Arc<dyn Storage>,
    node_did: String,
    signing_key: std::sync::Arc<ed25519_dalek::SigningKey>,
) {
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
            let inbox = addr.inbox_url(api_base);
            // Parse the inbox URL to extract host and path for signing.
            let (req_host, req_path) = match reqwest::Url::parse(&inbox) {
                Ok(parsed) => {
                    let host: String = parsed.host_str().unwrap_or(hostname.as_str()).to_string();
                    let path = if let Some(q) = parsed.query() {
                        format!("{}?{}", parsed.path(), q)
                    } else {
                        parsed.path().to_string()
                    };
                    (host, path)
                }
                Err(_) => (hostname.clone(), format!("/v1/agents/{}/inbox", addr.did)),
            };
            let (date_val, sig_val) = build_outbound_signature(
                &signing_key,
                &node_did,
                "post",
                &req_path,
                &req_host,
            );
            match client
                .post(&inbox)
                .header("date", &date_val)
                .header("signature", &sig_val)
                .header("host", &req_host)
                .json(&unit)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(
                        "remote_fanout: delivered unit {} to {addr}",
                        unit.id
                    );
                }
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    tracing::warn!(
                        "remote_fanout: delivery to {addr} returned status {status}"
                    );
                    notify_author_of_failure(
                        &storage,
                        &node_did,
                        &unit,
                        &addr.to_string(),
                        &format!("remote inbox at {addr} returned HTTP {status}"),
                    )
                    .await;
                }
                Err(e) => {
                    tracing::warn!("remote_fanout: delivery to {addr} failed: {e}");
                    notify_author_of_failure(
                        &storage,
                        &node_did,
                        &unit,
                        &addr.to_string(),
                        &format!("delivery to {addr} failed: {e}"),
                    )
                    .await;
                }
            }
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
    let visibilities = if auth.0.is_some() {
        vec![Visibility::Public, Visibility::Network]
    } else {
        vec![Visibility::Public]
    };
    let filter = build_filter(params, visibilities);
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
/// - **`text/event-stream`**: returns a Server-Sent Events stream. Each unit
///   is emitted as one SSE event whose `id` field is the unit's UUID and whose
///   `data` field is the unit's JSON. A final `event: end` marks the end of
///   the current page. Clients should use the `id` of the last received event
///   as the `Last-Event-ID` header on reconnect to resume from that cursor.
///
/// The `Last-Event-ID` header (when present) is used as the keyset pagination
/// cursor, identical to the `after` query parameter.
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
    let (units, has_more) = state.storage.list_units(&filter).await?;

    if wants_sse {
        // Build a Server-Sent Events response.
        // Each unit is one event; the final "end" event signals page completion.
        let mut body = String::new();
        for unit in &units {
            let json = serde_json::to_string(unit)
                .unwrap_or_else(|_| "{}".to_string());
            body.push_str(&format!("id: {}\ndata: {}\n\n", unit.id, json));
        }
        // Terminal event so the client knows this page is done.
        let has_more_str = if has_more { "true" } else { "false" };
        body.push_str(&format!("event: end\ndata: {{\"has_more\":{has_more_str}}}\n\n"));

        Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(axum::body::Body::from(body))
            .unwrap())
    } else {
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
        build_router(storage, config, signing_key)
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
}
