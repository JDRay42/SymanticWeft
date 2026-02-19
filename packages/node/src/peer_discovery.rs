//! Decentralised peer discovery — bootstrap sweep and peer list exchange.
//!
//! On startup, [`PeerDiscovery::bootstrap`] is called once in a background task:
//!
//! 1. For each URL in `SWEFT_BOOTSTRAP_PEERS`: announce this node (`POST`) and
//!    pull the remote peer list (`GET`).
//! 2. For each peer in those lists: verify its identity via
//!    `/.well-known/semanticweft`, then add it (subject to the peer cap).
//!
//! # Design decisions (from ADR-0008)
//!
//! - **Pull-on-connect only.** This node does not push peer info to arbitrary
//!   peers; it only exchanges lists when explicitly connecting.
//! - **Self-announce to bootstrap list only.** The node POSTs its own `PeerInfo`
//!   to configured bootstrap peers, not to every peer it discovers.
//! - **Verification before storage.** A peer whose `/.well-known/semanticweft`
//!   returns a `node_id` that doesn't match the announced `node_id` is rejected
//!   (possible impersonation). Network failures are given benefit of the doubt.
//! - **Reputation-based eviction.** When the list is full, the lowest-reputation
//!   peer is evicted to make room, provided the new peer is not worse.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use semanticweft_node_api::{NodeInfo, PeerInfo, PeersResponse};
use tracing::{info, warn};

use crate::storage::Storage;

// ---------------------------------------------------------------------------
// PeerDiscovery
// ---------------------------------------------------------------------------

/// Handles peer discovery and peer list exchange on startup.
pub struct PeerDiscovery {
    client: Client,
    storage: Arc<dyn Storage>,
    own_info: PeerInfo,
    max_peers: usize,
}

impl PeerDiscovery {
    pub fn new(
        client: Client,
        storage: Arc<dyn Storage>,
        own_info: PeerInfo,
        max_peers: usize,
    ) -> Self {
        Self {
            client,
            storage,
            own_info,
            max_peers,
        }
    }

    /// Run the bootstrap sweep: announce self to each configured URL and pull
    /// their peer lists. Intended to be called once at startup inside a
    /// `tokio::spawn`.
    pub async fn bootstrap(&self, bootstrap_urls: &[String]) {
        if bootstrap_urls.is_empty() {
            info!("peer_discovery: no bootstrap peers configured; skipping sweep");
            return;
        }

        for url in bootstrap_urls {
            let api_base = url.trim_end_matches('/');
            info!("peer_discovery: bootstrapping via {api_base}");
            self.announce_self(api_base).await;
            self.pull_peer_list(api_base).await;
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// POST our own [`PeerInfo`] to `{api_base}/peers`.
    async fn announce_self(&self, api_base: &str) {
        let url = format!("{api_base}/peers");
        match self
            .client
            .post(&url)
            .json(&self.own_info)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                info!("peer_discovery: announced self to {api_base}");
            }
            Ok(r) => {
                warn!(
                    "peer_discovery: announce to {api_base} returned {}",
                    r.status()
                );
            }
            Err(e) => {
                warn!("peer_discovery: announce to {api_base} failed: {e}");
            }
        }
    }

    /// GET the peer list from `{api_base}/peers` and attempt to add each entry.
    async fn pull_peer_list(&self, api_base: &str) {
        let url = format!("{api_base}/peers");
        let response = match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!("peer_discovery: failed to pull peer list from {api_base}: {e}");
                return;
            }
        };

        if !response.status().is_success() {
            warn!(
                "peer_discovery: peer list from {api_base} returned {}",
                response.status()
            );
            return;
        }

        let peers_response: PeersResponse = match response.json().await {
            Ok(r) => r,
            Err(e) => {
                warn!("peer_discovery: invalid peer list from {api_base}: {e}");
                return;
            }
        };

        info!(
            "peer_discovery: received {} peers from {api_base}",
            peers_response.peers.len()
        );

        for peer in peers_response.peers {
            if peer.node_id == self.own_info.node_id {
                continue; // never add ourselves
            }
            self.try_add_peer(peer).await;
        }
    }

    /// Verify a candidate peer and add it to storage if it passes checks and
    /// there is room in the peer list (evicting the worst entry if needed).
    async fn try_add_peer(&self, candidate: PeerInfo) {
        match self.verify_peer(&candidate).await {
            VerifyResult::Verified => {}
            VerifyResult::Mismatch => {
                warn!(
                    "peer_discovery: node_id mismatch for {} — rejecting (possible impersonation)",
                    candidate.api_base
                );
                return;
            }
            VerifyResult::Unreachable => {
                // Give benefit of the doubt for transient network issues.
                warn!(
                    "peer_discovery: could not reach {} for verification — storing tentatively",
                    candidate.api_base
                );
            }
        }

        // Enforce the peer cap.
        let peers = self.storage.list_peers().await.unwrap_or_default();
        if peers.len() >= self.max_peers {
            // list_peers returns highest-reputation first; worst is last.
            if let Some(worst) = peers.last() {
                if worst.reputation <= candidate.reputation {
                    let evict_id = worst.node_id.clone();
                    if let Err(e) = self.storage.remove_peer(&evict_id).await {
                        warn!("peer_discovery: eviction failed: {e}");
                        return;
                    }
                    info!(
                        "peer_discovery: evicted {} (reputation {:.2}) to make room",
                        evict_id, worst.reputation
                    );
                } else {
                    // New peer is worse than the worst existing peer; skip it.
                    return;
                }
            }
        }

        match self.storage.add_peer(&candidate).await {
            Ok(()) => info!("peer_discovery: added peer {}", candidate.node_id),
            Err(e) => warn!("peer_discovery: could not store {}: {e}", candidate.node_id),
        }
    }

    /// Fetch `/.well-known/semanticweft` and confirm the `node_id` matches.
    async fn verify_peer(&self, peer: &PeerInfo) -> VerifyResult {
        // Strip the /v1 suffix to get the node's base URL.
        let base = peer.api_base.trim_end_matches('/').trim_end_matches("/v1");
        let url = format!("{base}/.well-known/semanticweft");

        let response = match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return VerifyResult::Unreachable,
        };

        if !response.status().is_success() {
            return VerifyResult::Unreachable;
        }

        let node_info: NodeInfo = match response.json().await {
            Ok(i) => i,
            Err(_) => return VerifyResult::Unreachable,
        };

        if node_info.node_id == peer.node_id {
            VerifyResult::Verified
        } else {
            VerifyResult::Mismatch
        }
    }
}

enum VerifyResult {
    Verified,
    Mismatch,
    Unreachable,
}
