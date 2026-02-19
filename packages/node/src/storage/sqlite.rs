//! SQLite-backed storage implementation.
//!
//! Uses `rusqlite` (with bundled SQLite) wrapped in an `Arc<Mutex<Connection>>`
//! to satisfy the `Send + Sync` requirements. All blocking calls are offloaded
//! to a thread-pool via `tokio::task::spawn_blocking`.
//!
//! # Schema
//!
//! - `units` — full JSON blob plus indexed columns for filtering.
//! - `unit_references` — denormalised edge index for inbound subgraph traversal.
//! - `agents` — registered agent profiles.
//! - `follows` — (follower, followee) edges.
//! - `peers` — known peer nodes with reputation and last_seen (ADR-0008).
//! - `sync_cursors` — last-seen UUIDv7 per peer, for incremental federation.
//! - `node_config` — key-value store for node-level settings (e.g. identity seed).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::{params, Connection};
use semanticweft::{SemanticUnit, Visibility};
use semanticweft_node_api::{AgentProfile, PeerInfo};

use super::{Storage, StorageError, UnitFilter};

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS units (
    id          TEXT PRIMARY KEY,
    unit_type   TEXT NOT NULL,
    author      TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    visibility  TEXT NOT NULL DEFAULT 'public',
    data        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_units_author     ON units(author);
CREATE INDEX IF NOT EXISTS idx_units_type       ON units(unit_type);
CREATE INDEX IF NOT EXISTS idx_units_visibility ON units(visibility);
CREATE INDEX IF NOT EXISTS idx_units_created_at ON units(created_at);

-- Denormalised reference index: one row per (referencing, referenced) pair.
-- Populated on unit insert; used by get_referencing_units for subgraph traversal.
CREATE TABLE IF NOT EXISTS unit_references (
    referencing_id  TEXT NOT NULL,
    referenced_id   TEXT NOT NULL,
    PRIMARY KEY (referencing_id, referenced_id)
);
CREATE INDEX IF NOT EXISTS idx_unit_refs_referenced ON unit_references(referenced_id);

CREATE TABLE IF NOT EXISTS agents (
    did          TEXT PRIMARY KEY,
    inbox_url    TEXT NOT NULL,
    display_name TEXT,
    public_key   TEXT
);

CREATE TABLE IF NOT EXISTS follows (
    follower TEXT NOT NULL,
    followee TEXT NOT NULL,
    PRIMARY KEY (follower, followee)
);
CREATE INDEX IF NOT EXISTS idx_follows_followee ON follows(followee);

CREATE TABLE IF NOT EXISTS peers (
    node_id    TEXT PRIMARY KEY,
    api_base   TEXT NOT NULL,
    reputation REAL NOT NULL DEFAULT 0.5,
    last_seen  TEXT
);

CREATE TABLE IF NOT EXISTS sync_cursors (
    peer_url TEXT PRIMARY KEY,
    cursor   TEXT NOT NULL
);

-- Key-value store for durable node-level settings (e.g. identity seed).
CREATE TABLE IF NOT EXISTS node_config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

// ---------------------------------------------------------------------------
// SqliteStorage
// ---------------------------------------------------------------------------

/// SQLite-backed implementation of [`Storage`].
///
/// Holds a single database connection protected by a `Mutex`. All operations
/// run inside `spawn_blocking` to avoid blocking the async runtime.
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open (or create) the SQLite database at `path` and apply the schema.
    pub fn open(path: &str) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Self::migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory SQLite database (data is lost when dropped).
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        // In-memory DBs are always fresh; no migration needed.
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Apply additive migrations for existing databases that pre-date schema
    /// additions. Each `ALTER TABLE` is attempted and the error for "duplicate
    /// column name" is swallowed — this is intentional and safe.
    fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
        // peers: reputation and last_seen columns (added in this version)
        let _ = conn.execute(
            "ALTER TABLE peers ADD COLUMN reputation REAL NOT NULL DEFAULT 0.5",
            [],
        );
        let _ = conn.execute("ALTER TABLE peers ADD COLUMN last_seen TEXT", []);

        // node_config table is covered by CREATE TABLE IF NOT EXISTS in SCHEMA.
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Error conversions
// ---------------------------------------------------------------------------

fn map_err(e: rusqlite::Error) -> StorageError {
    StorageError::Internal(e.to_string())
}

fn map_json_err(e: serde_json::Error) -> StorageError {
    StorageError::Internal(format!("JSON error: {e}"))
}

// ---------------------------------------------------------------------------
// Dynamic query parameter helper
// ---------------------------------------------------------------------------

/// Typed SQL parameter for building dynamic WHERE clauses.
enum SqlParam {
    Text(String),
    Integer(i64),
}

impl rusqlite::ToSql for SqlParam {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, Value};
        match self {
            SqlParam::Text(s) => Ok(ToSqlOutput::Owned(Value::Text(s.clone()))),
            SqlParam::Integer(i) => Ok(ToSqlOutput::Owned(Value::Integer(*i))),
        }
    }
}

// ---------------------------------------------------------------------------
// Storage impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Storage for SqliteStorage {
    // --- Units ---------------------------------------------------------------

    async fn put_unit(&self, unit: &SemanticUnit) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let unit = unit.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            // Check for an existing unit with this ID.
            let exists: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM units WHERE id = ?1",
                    params![unit.id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(map_err)?
                > 0;

            if exists {
                return Err(StorageError::Conflict(format!(
                    "unit {} already exists",
                    unit.id
                )));
            }

            let data = serde_json::to_string(&unit).map_err(map_json_err)?;
            let visibility = unit
                .visibility
                .as_ref()
                .unwrap_or(&Visibility::Public)
                .to_string();

            conn.execute(
                "INSERT INTO units (id, unit_type, author, created_at, visibility, data)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    unit.id,
                    unit.unit_type.to_string(),
                    unit.author,
                    unit.created_at,
                    visibility,
                    data,
                ],
            )
            .map_err(map_err)?;

            // Populate the reference index.
            if let Some(refs) = &unit.references {
                for r in refs {
                    conn.execute(
                        "INSERT OR IGNORE INTO unit_references (referencing_id, referenced_id)
                         VALUES (?1, ?2)",
                        params![unit.id, r.id],
                    )
                    .map_err(map_err)?;
                }
            }

            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn get_unit(&self, id: &str) -> Result<Option<SemanticUnit>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT data FROM units WHERE id = ?1",
                params![id],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(data) => {
                    let unit: SemanticUnit =
                        serde_json::from_str(&data).map_err(map_json_err)?;
                    Ok(Some(unit))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(map_err(e)),
            }
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn list_units(
        &self,
        filter: &UnitFilter,
    ) -> Result<(Vec<SemanticUnit>, bool), StorageError> {
        let conn = Arc::clone(&self.conn);
        let filter = filter.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();

            let mut sql = String::from("SELECT data FROM units WHERE 1=1");
            let mut params_vec: Vec<SqlParam> = Vec::new();

            if !filter.unit_types.is_empty() {
                let placeholders: Vec<&str> =
                    filter.unit_types.iter().map(|_| "?").collect();
                sql.push_str(&format!(
                    " AND unit_type IN ({})",
                    placeholders.join(",")
                ));
                for t in &filter.unit_types {
                    params_vec.push(SqlParam::Text(t.to_string()));
                }
            }

            if let Some(author) = &filter.author {
                sql.push_str(" AND author = ?");
                params_vec.push(SqlParam::Text(author.clone()));
            }

            if let Some(since) = &filter.since {
                sql.push_str(" AND created_at >= ?");
                params_vec.push(SqlParam::Text(since.clone()));
            }

            if let Some(after) = &filter.after {
                sql.push_str(" AND id > ?");
                params_vec.push(SqlParam::Text(after.clone()));
            }

            if !filter.visibilities.is_empty() {
                let placeholders: Vec<&str> =
                    filter.visibilities.iter().map(|_| "?").collect();
                sql.push_str(&format!(
                    " AND visibility IN ({})",
                    placeholders.join(",")
                ));
                for v in &filter.visibilities {
                    params_vec.push(SqlParam::Text(v.to_string()));
                }
            }

            sql.push_str(" ORDER BY id ASC LIMIT ?");
            params_vec.push(SqlParam::Integer(filter.limit as i64 + 1));

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_vec.iter().map(|p| p as &dyn rusqlite::ToSql).collect();

            let mut stmt = conn.prepare(&sql).map_err(map_err)?;
            let data_rows: Vec<String> = stmt
                .query_map(params_refs.as_slice(), |row| row.get::<_, String>(0))
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;

            let limit = filter.limit as usize;
            let has_more = data_rows.len() > limit;
            let data_rows = if has_more {
                &data_rows[..limit]
            } else {
                &data_rows[..]
            };

            let units: Vec<SemanticUnit> = data_rows
                .iter()
                .map(|data| serde_json::from_str(data).map_err(map_json_err))
                .collect::<Result<Vec<_>, _>>()?;

            Ok((units, has_more))
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn get_referencing_units(&self, id: &str) -> Result<Vec<SemanticUnit>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let id = id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT u.data FROM units u
                     JOIN unit_references r ON r.referencing_id = u.id
                     WHERE r.referenced_id = ?1",
                )
                .map_err(map_err)?;

            let units: Vec<SemanticUnit> = stmt
                .query_map(params![id], |row| row.get::<_, String>(0))
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?
                .iter()
                .map(|data| serde_json::from_str(data).map_err(map_json_err))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(units)
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    // --- Agents --------------------------------------------------------------

    async fn put_agent(&self, profile: &AgentProfile) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let profile = profile.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO agents (did, inbox_url, display_name, public_key)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(did) DO UPDATE SET
                   inbox_url    = excluded.inbox_url,
                   display_name = excluded.display_name,
                   public_key   = excluded.public_key",
                params![
                    profile.did,
                    profile.inbox_url,
                    profile.display_name,
                    profile.public_key,
                ],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn get_agent(&self, did: &str) -> Result<Option<AgentProfile>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let did = did.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT did, inbox_url, display_name, public_key
                 FROM agents WHERE did = ?1",
                params![did],
                |row| {
                    Ok(AgentProfile {
                        did: row.get(0)?,
                        inbox_url: row.get(1)?,
                        display_name: row.get(2)?,
                        public_key: row.get(3)?,
                    })
                },
            );
            match result {
                Ok(profile) => Ok(Some(profile)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(map_err(e)),
            }
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    // --- Follows -------------------------------------------------------------

    async fn add_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let follower = follower.to_string();
        let followee = followee.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT OR IGNORE INTO follows (follower, followee) VALUES (?1, ?2)",
                params![follower, followee],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn remove_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let follower = follower.to_string();
        let followee = followee.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "DELETE FROM follows WHERE follower = ?1 AND followee = ?2",
                params![follower, followee],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn list_following(&self, did: &str) -> Result<Vec<String>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let did = did.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT followee FROM follows WHERE follower = ?1 ORDER BY followee ASC")
                .map_err(map_err)?;
            let result = stmt
                .query_map(params![did], |row| row.get::<_, String>(0))
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;
            Ok(result)
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn list_followers(&self, did: &str) -> Result<Vec<String>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let did = did.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare("SELECT follower FROM follows WHERE followee = ?1 ORDER BY follower ASC")
                .map_err(map_err)?;
            let result = stmt
                .query_map(params![did], |row| row.get::<_, String>(0))
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;
            Ok(result)
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn is_following(
        &self,
        follower: &str,
        followee: &str,
    ) -> Result<bool, StorageError> {
        let conn = Arc::clone(&self.conn);
        let follower = follower.to_string();
        let followee = followee.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM follows WHERE follower = ?1 AND followee = ?2",
                    params![follower, followee],
                    |row| row.get(0),
                )
                .map_err(map_err)?;
            Ok(count > 0)
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    // --- Peers ---------------------------------------------------------------

    async fn add_peer(&self, peer: &PeerInfo) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let peer = peer.clone();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            // On conflict: update api_base and last_seen but NOT reputation.
            conn.execute(
                "INSERT INTO peers (node_id, api_base, reputation, last_seen)
                 VALUES (?1, ?2, 0.5, ?3)
                 ON CONFLICT(node_id) DO UPDATE SET
                   api_base  = excluded.api_base,
                   last_seen = excluded.last_seen",
                params![peer.node_id, peer.api_base, peer.last_seen],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn remove_peer(&self, node_id: &str) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let node_id = node_id.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute("DELETE FROM peers WHERE node_id = ?1", params![node_id])
                .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn list_peers(&self) -> Result<Vec<PeerInfo>, StorageError> {
        let conn = Arc::clone(&self.conn);

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let mut stmt = conn
                .prepare(
                    "SELECT node_id, api_base, reputation, last_seen
                     FROM peers ORDER BY reputation DESC, node_id ASC",
                )
                .map_err(map_err)?;
            let result = stmt
                .query_map([], |row| {
                    Ok(PeerInfo {
                        node_id:    row.get(0)?,
                        api_base:   row.get(1)?,
                        reputation: row.get::<_, f64>(2)? as f32,
                        last_seen:  row.get(3)?,
                    })
                })
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;
            Ok(result)
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    // --- Node configuration -------------------------------------------------

    async fn get_node_config(&self, key: &str) -> Result<Option<String>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT value FROM node_config WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(v) => Ok(Some(v)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(map_err(e)),
            }
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn set_node_config(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let key = key.to_string();
        let value = value.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO node_config (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    // --- Sync cursors --------------------------------------------------------

    async fn get_cursor(&self, peer_url: &str) -> Result<Option<String>, StorageError> {
        let conn = Arc::clone(&self.conn);
        let peer_url = peer_url.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            let result = conn.query_row(
                "SELECT cursor FROM sync_cursors WHERE peer_url = ?1",
                params![peer_url],
                |row| row.get::<_, String>(0),
            );
            match result {
                Ok(cursor) => Ok(Some(cursor)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(map_err(e)),
            }
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
    }

    async fn set_cursor(&self, peer_url: &str, cursor: &str) -> Result<(), StorageError> {
        let conn = Arc::clone(&self.conn);
        let peer_url = peer_url.to_string();
        let cursor = cursor.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().unwrap();
            conn.execute(
                "INSERT INTO sync_cursors (peer_url, cursor) VALUES (?1, ?2)
                 ON CONFLICT(peer_url) DO UPDATE SET cursor = excluded.cursor",
                params![peer_url, cursor],
            )
            .map_err(map_err)?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Internal(format!("task join error: {e}")))?
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
            content: "test".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "did:key:z6MkTest".into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: None,
            visibility: None,
            audience: None,
            extensions: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn put_and_get_unit() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.put_unit(&u).await.unwrap();
        let got = s.get_unit(&u.id).await.unwrap().unwrap();
        assert_eq!(got.id, u.id);
    }

    #[tokio::test]
    async fn put_unit_conflict() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let u = unit("019526b2-f68a-7c3e-a0b4-000000000001");
        s.put_unit(&u).await.unwrap();
        let err = s.put_unit(&u).await.unwrap_err();
        assert!(matches!(err, StorageError::Conflict(_)));
    }

    #[tokio::test]
    async fn list_units_pagination() {
        let s = SqliteStorage::open_in_memory().unwrap();
        for i in 1u8..=5 {
            let mut u = unit("placeholder");
            u.id = format!("019526b2-f68a-7c3e-a0b4-0000000000{i:02x}");
            s.put_unit(&u).await.unwrap();
        }
        let filter = UnitFilter {
            limit: 2,
            visibilities: vec![Visibility::Public],
            ..Default::default()
        };
        let (page, has_more) = s.list_units(&filter).await.unwrap();
        assert_eq!(page.len(), 2);
        assert!(has_more);
    }

    #[tokio::test]
    async fn follow_roundtrip() {
        let s = SqliteStorage::open_in_memory().unwrap();
        s.add_follow("alice", "bob").await.unwrap();
        assert!(s.is_following("alice", "bob").await.unwrap());

        let following = s.list_following("alice").await.unwrap();
        assert!(following.contains(&"bob".to_string()));

        s.remove_follow("alice", "bob").await.unwrap();
        assert!(!s.is_following("alice", "bob").await.unwrap());
    }

    #[tokio::test]
    async fn peer_roundtrip_with_reputation() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let p = PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1");
        s.add_peer(&p).await.unwrap();

        let peers = s.list_peers().await.unwrap();
        assert_eq!(peers.len(), 1);
        assert!((peers[0].reputation - 0.5).abs() < f32::EPSILON);
        assert!(peers[0].last_seen.is_none());
    }

    #[tokio::test]
    async fn add_peer_does_not_overwrite_reputation() {
        let s = SqliteStorage::open_in_memory().unwrap();
        s.add_peer(&PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();

        // Simulate re-announcement from peer (api_base may have changed).
        let mut updated = PeerInfo::new("did:key:z6MkPeer", "https://new-url.example.com/v1");
        updated.last_seen = Some("2026-02-19T10:00:00Z".into());
        s.add_peer(&updated).await.unwrap();

        let peers = s.list_peers().await.unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].api_base, "https://new-url.example.com/v1");
        // Reputation should still be the default 0.5, not reset.
        assert!((peers[0].reputation - 0.5).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn remove_peer() {
        let s = SqliteStorage::open_in_memory().unwrap();
        s.add_peer(&PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1"))
            .await
            .unwrap();
        s.remove_peer("did:key:z6MkPeer").await.unwrap();
        assert!(s.list_peers().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn node_config_roundtrip() {
        let s = SqliteStorage::open_in_memory().unwrap();
        assert!(s.get_node_config("foo").await.unwrap().is_none());
        s.set_node_config("foo", "bar").await.unwrap();
        assert_eq!(s.get_node_config("foo").await.unwrap().as_deref(), Some("bar"));
        s.set_node_config("foo", "baz").await.unwrap();
        assert_eq!(s.get_node_config("foo").await.unwrap().as_deref(), Some("baz"));
    }

    #[tokio::test]
    async fn cursor_roundtrip() {
        let s = SqliteStorage::open_in_memory().unwrap();
        let p = PeerInfo::new("did:key:z6MkPeer", "https://peer.example.com/v1");
        s.add_peer(&p).await.unwrap();

        s.set_cursor("https://peer.example.com", "some-cursor")
            .await
            .unwrap();
        let c = s.get_cursor("https://peer.example.com").await.unwrap();
        assert_eq!(c.as_deref(), Some("some-cursor"));
    }
}
