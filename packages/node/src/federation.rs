//! Node-to-node federation (pull sync).
//!
//! This module implements the puller side of the sync protocol described in
//! `spec/node-api.md` §7. A node periodically calls `GET <peer>/v1/sync?after=<cursor>`
//! on each known peer, stores any units it receives (ignoring conflicts for units
//! it already holds), and advances its cursor.
//!
//! # Design
//!
//! [`FederationSync`] owns an HTTP client and a reference to the local storage.
//! Callers create one instance and either call [`FederationSync::sync_peer`] on
//! demand or drive the background loop via [`FederationSync::run`].
//!
//! Errors from individual peers are **logged** and **do not stop** the loop;
//! the node will retry on the next interval.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use semanticweft_node_api::ListResponse;
use tracing::{info, warn};

use crate::storage::{Storage, StorageError};

// ---------------------------------------------------------------------------
// SyncError
// ---------------------------------------------------------------------------

/// Errors that can occur while syncing a single peer.
#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    /// The HTTP request or response failed.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// The peer returned a non-2xx HTTP status code.
    #[error("peer returned status {0}")]
    BadStatus(u16),

    /// A local storage operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),
}

// ---------------------------------------------------------------------------
// FederationSync
// ---------------------------------------------------------------------------

/// Handles pulling units from peer nodes.
///
/// One instance is shared across the background sync task. It holds a cloneable
/// [`reqwest::Client`] (which internally pools connections) and an [`Arc`] to
/// the local storage.
pub struct FederationSync {
    client: Client,
    storage: Arc<dyn Storage>,
}

impl FederationSync {
    /// Create a new `FederationSync`.
    ///
    /// # Arguments
    ///
    /// * `client` — pre-configured `reqwest::Client` (e.g. with a timeout).
    /// * `storage` — the local storage backend to write synced units into.
    pub fn new(client: Client, storage: Arc<dyn Storage>) -> Self {
        Self { client, storage }
    }

    /// Fetch **one page** of units from a peer and store them locally.
    ///
    /// The method:
    /// 1. Reads the last-known cursor for `api_base` from storage.
    /// 2. Calls `GET {api_base}/sync?after={cursor}&limit=500`.
    /// 3. For each unit in the response, calls [`Storage::put_unit`]; a
    ///    [`StorageError::Conflict`] (unit already stored) is silently ignored.
    /// 4. Computes a **credibility** score for each new unit using the peer's
    ///    local reputation and the author reputations included in the sync
    ///    response: `credibility = peer_reputation × author_reputation`.
    /// 5. Advances the cursor in storage to the `cursor` field of the response.
    ///
    /// Returns `true` when `has_more` is set in the response (more pages
    /// available), `false` when this was the last page.
    pub async fn sync_peer_once(&self, api_base: &str) -> Result<bool, SyncError> {
        let cursor = self.storage.get_cursor(api_base).await?;

        let url = format!("{}/sync", api_base.trim_end_matches('/'));
        let mut req = self.client.get(&url).query(&[("limit", "500")]);
        if let Some(ref after) = cursor {
            req = req.query(&[("after", after.as_str())]);
        }

        let response = req.send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(SyncError::BadStatus(status.as_u16()));
        }

        let page: ListResponse = response.json().await?;
        let has_more = page.has_more;

        // Look up the peer's local reputation so we can weight author scores.
        let peer_reputation = self.peer_reputation_for(api_base).await;

        for unit in &page.units {
            let is_new = match self.storage.put_unit(unit).await {
                Ok(()) => true,
                Err(StorageError::Conflict(_)) => false,
                Err(e) => return Err(SyncError::Storage(e)),
            };

            // Compute credibility for newly stored units.
            if is_new {
                let author_rep = page
                    .author_reputations
                    .get(&unit.author)
                    .copied()
                    .unwrap_or(0.5);
                let credibility = peer_reputation * author_rep;
                // Best-effort; credibility storage failure is not fatal.
                let _ = self
                    .storage
                    .set_unit_credibility(&unit.id, credibility)
                    .await;
            }
        }

        // Advance the cursor to the last unit ID seen on this page.
        if let Some(ref new_cursor) = page.cursor {
            self.storage.set_cursor(api_base, new_cursor).await?;
        }

        Ok(has_more)
    }

    /// Look up the local reputation of the peer identified by `api_base`.
    ///
    /// Falls back to `0.5` (neutral) if the peer is not in our registry.
    async fn peer_reputation_for(&self, api_base: &str) -> f32 {
        match self.storage.list_peers().await {
            Ok(peers) => peers
                .iter()
                .find(|p| p.api_base == api_base)
                .map(|p| p.reputation)
                .unwrap_or(0.5),
            Err(_) => 0.5,
        }
    }

    /// Drain all available pages from a peer in one shot.
    ///
    /// Calls [`sync_peer_once`] in a loop until `has_more` is `false` or an
    /// error is encountered. This brings the local store fully up to date with
    /// the peer's public unit stream as of the time of the call.
    pub async fn sync_peer(&self, api_base: &str) -> Result<(), SyncError> {
        loop {
            match self.sync_peer_once(api_base).await? {
                true => {} // has_more — fetch next page
                false => return Ok(()),
            }
        }
    }

    /// Run the federation sync loop indefinitely.
    ///
    /// On each iteration the loop:
    /// 1. Lists all known peers from storage.
    /// 2. Calls [`sync_peer`] for each one.
    /// 3. Sleeps for `interval`.
    ///
    /// Individual peer errors are logged and do not abort the loop. This method
    /// never returns; spawn it with [`tokio::spawn`].
    pub async fn run(self, interval: Duration) {
        loop {
            let peers = match self.storage.list_peers().await {
                Ok(p) => p,
                Err(e) => {
                    warn!("federation: failed to list peers: {e}");
                    vec![]
                }
            };

            for peer in peers {
                info!("federation: syncing {}", peer.api_base);
                if let Err(e) = self.sync_peer(&peer.api_base).await {
                    warn!("federation: sync failed for {}: {e}", peer.api_base);
                }
            }

            tokio::time::sleep(interval).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::{routing::get, Json, Router};
    use semanticweft::{SemanticUnit, UnitType};
    use semanticweft_node_api::ListResponse;
    use tokio::net::TcpListener;

    use crate::storage::memory::MemoryStorage;
    use crate::storage::UnitFilter;

    fn make_unit(id: &str) -> SemanticUnit {
        SemanticUnit {
            id: id.into(),
            unit_type: UnitType::Assertion,
            content: "federated content".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "did:key:z6MkPeer".into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: None,
            visibility: None,
            audience: None,
            proof: None,
            extensions: HashMap::new(),
        }
    }

    /// Minimal axum handler that always returns one unit with `has_more: false`.
    async fn one_unit_handler() -> Json<ListResponse> {
        let unit = make_unit("019526b2-0000-7000-a000-000000000001");
        Json(ListResponse::from_page(vec![unit], false))
    }

    /// Spawn a loopback axum server and return its base URL (e.g. `http://127.0.0.1:PORT/v1`).
    async fn spawn_mock_server(router: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://{addr}/v1")
    }

    // -----------------------------------------------------------------------
    // Test: units fetched from a peer are stored locally
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn sync_peer_stores_units_locally() {
        let app = Router::new().route("/v1/sync", get(one_unit_handler));
        let api_base = spawn_mock_server(app).await;

        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let syncer = FederationSync::new(Client::new(), Arc::clone(&storage));

        syncer.sync_peer(&api_base).await.unwrap();

        let filter = UnitFilter {
            limit: 50,
            ..Default::default()
        };
        let (units, _) = storage.list_units(&filter).await.unwrap();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].id, "019526b2-0000-7000-a000-000000000001");
    }

    // -----------------------------------------------------------------------
    // Test: cursor is advanced to the last unit ID after a successful sync
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn cursor_advances_after_sync() {
        let app = Router::new().route("/v1/sync", get(one_unit_handler));
        let api_base = spawn_mock_server(app).await;

        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let syncer = FederationSync::new(Client::new(), Arc::clone(&storage));

        // No cursor before the first sync.
        assert!(storage.get_cursor(&api_base).await.unwrap().is_none());

        syncer.sync_peer_once(&api_base).await.unwrap();

        // Cursor should now be the ID of the only unit returned.
        let cursor = storage.get_cursor(&api_base).await.unwrap();
        assert_eq!(
            cursor.as_deref(),
            Some("019526b2-0000-7000-a000-000000000001")
        );
    }

    // -----------------------------------------------------------------------
    // Test: receiving a unit that already exists locally is not an error
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn duplicate_units_are_silently_ignored() {
        let app = Router::new().route("/v1/sync", get(one_unit_handler));
        let api_base = spawn_mock_server(app).await;

        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());

        // Pre-insert the unit so that storage already holds it.
        storage
            .put_unit(&make_unit("019526b2-0000-7000-a000-000000000001"))
            .await
            .unwrap();

        let syncer = FederationSync::new(Client::new(), Arc::clone(&storage));

        // Syncing must succeed despite the unit already being present.
        syncer.sync_peer_once(&api_base).await.unwrap();

        // Still exactly one unit — no duplicate was inserted.
        let filter = UnitFilter {
            limit: 50,
            ..Default::default()
        };
        let (units, _) = storage.list_units(&filter).await.unwrap();
        assert_eq!(units.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test: multi-page sync drains all pages until has_more is false
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn multi_page_sync_drains_all_pages() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // The mock serves page 1 (has_more=true) on the first call,
        // page 2 (has_more=false) on the second.
        static CALL_COUNT: AtomicUsize = AtomicUsize::new(0);

        async fn paged_handler() -> Json<ListResponse> {
            let call = CALL_COUNT.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                let unit = SemanticUnit {
                    id: "019526b2-0000-7000-a000-000000000001".into(),
                    unit_type: UnitType::Assertion,
                    content: "page1".into(),
                    created_at: "2026-02-18T12:00:00Z".into(),
                    author: "did:key:z6MkPeer".into(),
                    confidence: None,
                    assumptions: None,
                    source: None,
                    references: None,
                    visibility: None,
                    audience: None,
                    proof: None,
                    extensions: HashMap::new(),
                };
                Json(ListResponse::from_page(vec![unit], true))
            } else {
                let unit = SemanticUnit {
                    id: "019526b2-0000-7000-a000-000000000002".into(),
                    unit_type: UnitType::Assertion,
                    content: "page2".into(),
                    created_at: "2026-02-18T12:00:01Z".into(),
                    author: "did:key:z6MkPeer".into(),
                    confidence: None,
                    assumptions: None,
                    source: None,
                    references: None,
                    visibility: None,
                    audience: None,
                    proof: None,
                    extensions: HashMap::new(),
                };
                Json(ListResponse::from_page(vec![unit], false))
            }
        }

        let app = Router::new().route("/v1/sync", get(paged_handler));
        let api_base = spawn_mock_server(app).await;

        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let syncer = FederationSync::new(Client::new(), Arc::clone(&storage));

        syncer.sync_peer(&api_base).await.unwrap();

        let filter = UnitFilter {
            limit: 50,
            ..Default::default()
        };
        let (units, _) = storage.list_units(&filter).await.unwrap();
        assert_eq!(units.len(), 2, "both pages should be stored");

        // Cursor should be the last unit from the second page.
        let cursor = storage.get_cursor(&api_base).await.unwrap();
        assert_eq!(
            cursor.as_deref(),
            Some("019526b2-0000-7000-a000-000000000002")
        );
    }
}
