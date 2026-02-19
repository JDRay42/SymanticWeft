//! In-memory storage implementation.
//!
//! All data is held in RAM behind a [`RwLock`] and is lost when the process
//! exits. Use this for tests, the conformance suite, and ephemeral nodes.
//!
//! Units are stored in a [`BTreeMap`] keyed by UUIDv7 ID. Because UUIDv7 IDs
//! sort lexicographically in creation order, keyset pagination with `id > cursor`
//! is simply a range query on the map — no secondary index needed.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::RwLock;

use async_trait::async_trait;
use semanticweft::{SemanticUnit, Visibility};
use semanticweft_node_api::{AgentProfile, PeerInfo};

use super::{Storage, StorageError, UnitFilter};

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct Inner {
    units: BTreeMap<String, SemanticUnit>,
    agents: HashMap<String, AgentProfile>,
    follows: HashSet<(String, String)>,
    peers: HashMap<String, PeerInfo>,
    cursors: HashMap<String, String>,
    node_config: HashMap<String, String>,
    /// Per-agent inbox: agent DID → BTreeMap<unit_id, SemanticUnit>.
    /// BTreeMap gives lexicographic order on UUIDv7 IDs for free keyset pagination.
    inbox: HashMap<String, BTreeMap<String, SemanticUnit>>,
}

impl Inner {
    fn new() -> Self {
        Self {
            units: BTreeMap::new(),
            agents: HashMap::new(),
            follows: HashSet::new(),
            peers: HashMap::new(),
            cursors: HashMap::new(),
            node_config: HashMap::new(),
            inbox: HashMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryStorage
// ---------------------------------------------------------------------------

/// Thread-safe, in-memory implementation of [`Storage`].
pub struct MemoryStorage {
    inner: RwLock<Inner>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::new()),
        }
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Storage impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Storage for MemoryStorage {
    // --- Units ---------------------------------------------------------------

    async fn put_unit(&self, unit: &SemanticUnit) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        if inner.units.contains_key(&unit.id) {
            return Err(StorageError::Conflict(format!(
                "unit {} already exists",
                unit.id
            )));
        }
        inner.units.insert(unit.id.clone(), unit.clone());
        Ok(())
    }

    async fn get_unit(&self, id: &str) -> Result<Option<SemanticUnit>, StorageError> {
        let inner = self.inner.read().unwrap();
        Ok(inner.units.get(id).cloned())
    }

    async fn list_units(
        &self,
        filter: &UnitFilter,
    ) -> Result<(Vec<SemanticUnit>, bool), StorageError> {
        let inner = self.inner.read().unwrap();
        let limit = filter.limit as usize;

        // Start from the cursor position using BTreeMap range.
        let iter: Box<dyn Iterator<Item = &SemanticUnit>> = if let Some(after) = &filter.after {
            use std::ops::Bound;
            Box::new(
                inner
                    .units
                    .range((Bound::Excluded(after.clone()), Bound::Unbounded))
                    .map(|(_, v)| v),
            )
        } else {
            Box::new(inner.units.values())
        };

        let mut results: Vec<SemanticUnit> = iter
            .filter(|u| {
                // Unit type filter
                if !filter.unit_types.is_empty() && !filter.unit_types.contains(&u.unit_type) {
                    return false;
                }
                // Author filter
                if let Some(author) = &filter.author {
                    if u.author != *author {
                        return false;
                    }
                }
                // Since filter (lexicographic ISO 8601 comparison)
                if let Some(since) = &filter.since {
                    if u.created_at.as_str() < since.as_str() {
                        return false;
                    }
                }
                // Visibility filter
                if !filter.visibilities.is_empty() {
                    let vis = u.visibility.as_ref().unwrap_or(&Visibility::Public);
                    if !filter.visibilities.contains(vis) {
                        return false;
                    }
                }
                true
            })
            .take(limit + 1)
            .cloned()
            .collect();

        let has_more = results.len() > limit;
        if has_more {
            results.truncate(limit);
        }

        Ok((results, has_more))
    }

    async fn get_referencing_units(&self, id: &str) -> Result<Vec<SemanticUnit>, StorageError> {
        let inner = self.inner.read().unwrap();
        let results = inner
            .units
            .values()
            .filter(|u| {
                u.references
                    .as_ref()
                    .map_or(false, |refs| refs.iter().any(|r| r.id == id))
            })
            .cloned()
            .collect();
        Ok(results)
    }

    // --- Agents --------------------------------------------------------------

    async fn put_agent(&self, profile: &AgentProfile) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner.agents.insert(profile.did.clone(), profile.clone());
        Ok(())
    }

    async fn get_agent(&self, did: &str) -> Result<Option<AgentProfile>, StorageError> {
        let inner = self.inner.read().unwrap();
        Ok(inner.agents.get(did).cloned())
    }

    // --- Follows -------------------------------------------------------------

    async fn add_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner
            .follows
            .insert((follower.to_string(), followee.to_string()));
        Ok(())
    }

    async fn remove_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner
            .follows
            .remove(&(follower.to_string(), followee.to_string()));
        Ok(())
    }

    async fn list_following(&self, did: &str) -> Result<Vec<String>, StorageError> {
        let inner = self.inner.read().unwrap();
        let result = inner
            .follows
            .iter()
            .filter(|(follower, _)| follower == did)
            .map(|(_, followee)| followee.clone())
            .collect();
        Ok(result)
    }

    async fn list_followers(&self, did: &str) -> Result<Vec<String>, StorageError> {
        let inner = self.inner.read().unwrap();
        let result = inner
            .follows
            .iter()
            .filter(|(_, followee)| followee == did)
            .map(|(follower, _)| follower.clone())
            .collect();
        Ok(result)
    }

    async fn is_following(
        &self,
        follower: &str,
        followee: &str,
    ) -> Result<bool, StorageError> {
        let inner = self.inner.read().unwrap();
        Ok(inner
            .follows
            .contains(&(follower.to_string(), followee.to_string())))
    }

    // --- Peers ---------------------------------------------------------------

    async fn add_peer(&self, peer: &PeerInfo) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        if let Some(existing) = inner.peers.get_mut(&peer.node_id) {
            // Update api_base and last_seen; preserve reputation.
            existing.api_base = peer.api_base.clone();
            existing.last_seen = peer.last_seen.clone();
        } else {
            inner.peers.insert(peer.node_id.clone(), peer.clone());
        }
        Ok(())
    }

    async fn remove_peer(&self, node_id: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner.peers.remove(node_id);
        Ok(())
    }

    async fn list_peers(&self) -> Result<Vec<PeerInfo>, StorageError> {
        let inner = self.inner.read().unwrap();
        let mut peers: Vec<PeerInfo> = inner.peers.values().cloned().collect();
        // Sort by reputation descending, then node_id ascending for stability.
        peers.sort_by(|a, b| {
            b.reputation
                .partial_cmp(&a.reputation)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        Ok(peers)
    }

    // --- Node configuration -------------------------------------------------

    async fn get_node_config(&self, key: &str) -> Result<Option<String>, StorageError> {
        let inner = self.inner.read().unwrap();
        Ok(inner.node_config.get(key).cloned())
    }

    async fn set_node_config(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner.node_config.insert(key.to_string(), value.to_string());
        Ok(())
    }

    // --- Sync cursors --------------------------------------------------------

    async fn get_cursor(&self, peer_url: &str) -> Result<Option<String>, StorageError> {
        let inner = self.inner.read().unwrap();
        Ok(inner.cursors.get(peer_url).cloned())
    }

    async fn set_cursor(&self, peer_url: &str, cursor: &str) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner
            .cursors
            .insert(peer_url.to_string(), cursor.to_string());
        Ok(())
    }

    // --- Inbox ---------------------------------------------------------------

    async fn deliver_to_inbox(
        &self,
        agent_did: &str,
        unit: &SemanticUnit,
    ) -> Result<(), StorageError> {
        let mut inner = self.inner.write().unwrap();
        inner
            .inbox
            .entry(agent_did.to_string())
            .or_default()
            .entry(unit.id.clone())
            .or_insert_with(|| unit.clone());
        Ok(())
    }

    async fn get_inbox(
        &self,
        agent_did: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<SemanticUnit>, bool), StorageError> {
        let inner = self.inner.read().unwrap();
        let limit = limit as usize;

        let agent_inbox = match inner.inbox.get(agent_did) {
            Some(m) => m,
            None => return Ok((vec![], false)),
        };

        let iter: Box<dyn Iterator<Item = &SemanticUnit>> = if let Some(after) = after {
            use std::ops::Bound;
            Box::new(
                agent_inbox
                    .range((Bound::Excluded(after.to_string()), Bound::Unbounded))
                    .map(|(_, v)| v),
            )
        } else {
            Box::new(agent_inbox.values())
        };

        let mut items: Vec<SemanticUnit> = iter.take(limit + 1).cloned().collect();
        let has_more = items.len() > limit;
        if has_more {
            items.truncate(limit);
        }
        Ok((items, has_more))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use semanticweft::{SemanticUnit, UnitType};
    use std::collections::HashMap;

    fn unit(id: &str) -> SemanticUnit {
        SemanticUnit {
            id: id.into(),
            unit_type: UnitType::Assertion,
            content: "test content".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "did:key:z6MkTest".into(),
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

    fn agent(did: &str) -> AgentProfile {
        AgentProfile {
            did: did.into(),
            inbox_url: format!("https://node.example.com/v1/agents/{did}/inbox"),
            display_name: None,
            public_key: None,
        }
    }

    #[tokio::test]
    async fn put_and_get_unit() {
        let s = MemoryStorage::new();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.put_unit(&u).await.unwrap();
        let got = s.get_unit(&u.id).await.unwrap();
        assert_eq!(got.unwrap().id, u.id);
    }

    #[tokio::test]
    async fn put_unit_conflict() {
        let s = MemoryStorage::new();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.put_unit(&u).await.unwrap();
        let err = s.put_unit(&u).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    #[tokio::test]
    async fn list_units_pagination() {
        let s = MemoryStorage::new();
        for i in 0u8..5 {
            let mut u = unit(&format!("019526b2-f68a-7c3e-a0b4-0000000000{i:02x}"));
            u.id = format!("019526b2-f68a-7c3e-a0b4-0000000000{i:02x}");
            s.put_unit(&u).await.unwrap();
        }

        let filter = UnitFilter {
            limit: 2,
            ..Default::default()
        };
        let (page, has_more) = s.list_units(&filter).await.unwrap();
        assert_eq!(page.len(), 2);
        assert!(has_more);
    }

    #[tokio::test]
    async fn follow_roundtrip() {
        let s = MemoryStorage::new();
        s.add_follow("alice", "bob").await.unwrap();
        assert!(s.is_following("alice", "bob").await.unwrap());
        assert!(!s.is_following("bob", "alice").await.unwrap());

        let following = s.list_following("alice").await.unwrap();
        assert!(following.contains(&"bob".to_string()));

        let followers = s.list_followers("bob").await.unwrap();
        assert!(followers.contains(&"alice".to_string()));

        s.remove_follow("alice", "bob").await.unwrap();
        assert!(!s.is_following("alice", "bob").await.unwrap());
    }

    #[tokio::test]
    async fn agent_roundtrip() {
        let s = MemoryStorage::new();
        let a = agent("did:key:z6MkAlice");
        s.put_agent(&a).await.unwrap();
        let got = s.get_agent(&a.did).await.unwrap().unwrap();
        assert_eq!(got.did, a.did);
        assert!(s.get_agent("did:key:z6MkNobody").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn peer_roundtrip_with_reputation() {
        let s = MemoryStorage::new();
        let p = PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1");
        s.add_peer(&p).await.unwrap();
        let peers = s.list_peers().await.unwrap();
        assert_eq!(peers.len(), 1);
        assert!((peers[0].reputation - 0.5).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn add_peer_preserves_reputation_on_update() {
        let s = MemoryStorage::new();
        let mut p = PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1");
        p.reputation = 0.8;
        s.add_peer(&p).await.unwrap();

        // Re-announce with new URL; reputation should be preserved.
        let updated = PeerInfo::new("did:key:z6MkPeer", "https://new.example.com/v1");
        s.add_peer(&updated).await.unwrap();

        let peers = s.list_peers().await.unwrap();
        assert_eq!(peers[0].api_base, "https://new.example.com/v1");
        assert!((peers[0].reputation - 0.8).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn remove_peer() {
        let s = MemoryStorage::new();
        s.add_peer(&PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();
        s.remove_peer("did:key:z6MkPeer").await.unwrap();
        assert!(s.list_peers().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn node_config_roundtrip() {
        let s = MemoryStorage::new();
        assert!(s.get_node_config("mykey").await.unwrap().is_none());
        s.set_node_config("mykey", "myvalue").await.unwrap();
        assert_eq!(
            s.get_node_config("mykey").await.unwrap().as_deref(),
            Some("myvalue")
        );
    }

    #[tokio::test]
    async fn cursor_roundtrip() {
        let s = MemoryStorage::new();
        assert!(s.get_cursor("https://peer.example.com").await.unwrap().is_none());
        s.set_cursor("https://peer.example.com", "019526b2-f68a-7c3e-a0b4-000000000001")
            .await
            .unwrap();
        let c = s.get_cursor("https://peer.example.com").await.unwrap();
        assert_eq!(
            c.as_deref(),
            Some("019526b2-f68a-7c3e-a0b4-000000000001")
        );
    }

    #[tokio::test]
    async fn inbox_deliver_and_read() {
        let s = MemoryStorage::new();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.deliver_to_inbox("did:key:alice", &u).await.unwrap();

        let (items, has_more) = s.get_inbox("did:key:alice", None, 10).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, u.id);
        assert!(!has_more);
    }

    #[tokio::test]
    async fn inbox_idempotent() {
        let s = MemoryStorage::new();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.deliver_to_inbox("did:key:alice", &u).await.unwrap();
        s.deliver_to_inbox("did:key:alice", &u).await.unwrap();

        let (items, _) = s.get_inbox("did:key:alice", None, 10).await.unwrap();
        assert_eq!(items.len(), 1, "duplicate delivery must be deduplicated");
    }

    #[tokio::test]
    async fn inbox_pagination() {
        let s = MemoryStorage::new();
        for i in 1u8..=5 {
            let mut u = unit("placeholder");
            u.id = format!("019526b2-f68a-7c3e-a0b4-0000000000{i:02x}");
            s.deliver_to_inbox("did:key:alice", &u).await.unwrap();
        }

        let (page1, has_more) = s.get_inbox("did:key:alice", None, 2).await.unwrap();
        assert_eq!(page1.len(), 2);
        assert!(has_more);

        let cursor = page1.last().map(|u| u.id.clone());
        let (page2, _) = s
            .get_inbox("did:key:alice", cursor.as_deref(), 2)
            .await
            .unwrap();
        assert_eq!(page2.len(), 2);
        assert_ne!(page1[0].id, page2[0].id, "pages must not overlap");
    }

    #[tokio::test]
    async fn inbox_empty_for_unknown_agent() {
        let s = MemoryStorage::new();
        let (items, has_more) = s.get_inbox("did:key:nobody", None, 10).await.unwrap();
        assert!(items.is_empty());
        assert!(!has_more);
    }
}
