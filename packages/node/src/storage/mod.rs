//! Storage abstraction layer for the SemanticWeft node.
//!
//! The [`Storage`] trait defines the contract between the HTTP handler layer
//! and persistence. All visibility and authorization logic lives in the
//! handlers; storage is purely a data access layer.
//!
//! # Implementations
//!
//! | Type | When to use |
//! |------|-------------|
//! | [`MemoryStorage`] | Tests, conformance suite, ephemeral nodes |
//! | [`SqliteStorage`] | Production; durable single-file database |
//!
//! [`MemoryStorage`]: memory::MemoryStorage
//! [`SqliteStorage`]: sqlite::SqliteStorage

pub mod memory;
pub mod sqlite;

use async_trait::async_trait;
use semanticweft::{SemanticUnit, UnitType, Visibility};
use semanticweft_node_api::{AgentProfile, PeerInfo};

// ---------------------------------------------------------------------------
// StorageError
// ---------------------------------------------------------------------------

/// Errors that storage operations can return.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The requested item does not exist.
    #[error("not found")]
    NotFound,

    /// An item with the same key already exists (e.g. duplicate unit ID).
    #[error("conflict: {0}")]
    Conflict(String),

    /// An unexpected error in the underlying storage backend.
    #[error("internal storage error: {0}")]
    Internal(String),
}

// ---------------------------------------------------------------------------
// UnitFilter
// ---------------------------------------------------------------------------

/// Query parameters for [`Storage::list_units`].
///
/// Built by handlers from URL query parameters. The handler is responsible
/// for deciding which `visibilities` the caller is entitled to see and setting
/// the field accordingly; storage applies it as a data filter without any
/// knowledge of who is asking.
#[derive(Debug, Clone, Default)]
pub struct UnitFilter {
    /// Include only units whose `unit_type` is one of these values.
    /// Empty means all types are included.
    pub unit_types: Vec<UnitType>,

    /// Include only units whose `author` exactly matches this value.
    pub author: Option<String>,

    /// Include only units whose `created_at >= since` (ISO 8601 string).
    pub since: Option<String>,

    /// Cursor for keyset pagination: include only units whose `id > after`
    /// (UUIDv7 lexicographic order). `None` means start from the beginning.
    pub after: Option<String>,

    /// Maximum number of units to return. The implementation fetches
    /// `limit + 1` internally to determine `has_more`, then truncates.
    pub limit: u32,

    /// Include only units whose `visibility` is one of these values.
    /// Empty means all visibilities are included.
    ///
    /// Handlers set this based on the caller's identity:
    /// - Unauthenticated: `[Public]`
    /// - Authenticated follower of author: `[Public, Network]`
    /// - Agent listed in `audience`: `[Public, Network, Limited]`
    pub visibilities: Vec<Visibility>,
}

// ---------------------------------------------------------------------------
// Storage trait
// ---------------------------------------------------------------------------

/// The persistence contract for a SemanticWeft node.
///
/// All methods are `async` and return `Result<_, StorageError>`. Implementations
/// must be `Send + Sync + 'static` so they can be held in an `Arc<dyn Storage>`.
#[async_trait]
pub trait Storage: Send + Sync + 'static {
    // --- Units ---------------------------------------------------------------

    /// Persist a unit. Returns [`StorageError::Conflict`] if a unit with the
    /// same `id` already exists (IDs are immutable content-addressed keys).
    async fn put_unit(&self, unit: &SemanticUnit) -> Result<(), StorageError>;

    /// Retrieve a unit by its UUIDv7 `id`. Returns `None` if not found.
    async fn get_unit(&self, id: &str) -> Result<Option<SemanticUnit>, StorageError>;

    /// Return a page of units matching `filter`, ordered by `id` ascending.
    ///
    /// Returns `(units, has_more)` where `has_more` is `true` when there are
    /// additional units beyond this page.
    async fn list_units(
        &self,
        filter: &UnitFilter,
    ) -> Result<(Vec<SemanticUnit>, bool), StorageError>;

    /// Find all units whose `references` array contains `id` (incoming edges).
    ///
    /// Used by the subgraph handler to traverse in both directions.
    async fn get_referencing_units(&self, id: &str) -> Result<Vec<SemanticUnit>, StorageError>;

    // --- Agents --------------------------------------------------------------

    /// Register or update an agent profile (upsert by `did`).
    async fn put_agent(&self, profile: &AgentProfile) -> Result<(), StorageError>;

    /// Retrieve an agent profile by DID. Returns `None` if not registered.
    async fn get_agent(&self, did: &str) -> Result<Option<AgentProfile>, StorageError>;

    // --- Follows -------------------------------------------------------------

    /// Record that `follower` follows `followee`. Idempotent.
    async fn add_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError>;

    /// Remove a follow relationship. Idempotent (no error if it didn't exist).
    async fn remove_follow(&self, follower: &str, followee: &str) -> Result<(), StorageError>;

    /// Return the DIDs that `did` follows, in unspecified order.
    async fn list_following(&self, did: &str) -> Result<Vec<String>, StorageError>;

    /// Return the DIDs that follow `did`, in unspecified order.
    async fn list_followers(&self, did: &str) -> Result<Vec<String>, StorageError>;

    /// Return `true` if `follower` follows `followee`.
    async fn is_following(
        &self,
        follower: &str,
        followee: &str,
    ) -> Result<bool, StorageError>;

    // --- Peers ---------------------------------------------------------------

    /// Add or update a peer entry (upsert by `node_id`).
    async fn add_peer(&self, peer: &PeerInfo) -> Result<(), StorageError>;

    /// Return all known peers.
    async fn list_peers(&self) -> Result<Vec<PeerInfo>, StorageError>;

    // --- Sync cursors --------------------------------------------------------

    /// Return the last-seen UUIDv7 sync cursor for a peer, or `None` if we
    /// have never synced with that peer.
    async fn get_cursor(&self, peer_url: &str) -> Result<Option<String>, StorageError>;

    /// Persist the sync cursor for a peer (upsert by `peer_url`).
    async fn set_cursor(&self, peer_url: &str, cursor: &str) -> Result<(), StorageError>;
}
