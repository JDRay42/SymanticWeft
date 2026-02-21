//! Public surface for the `semanticweft-node` crate.
//!
//! Exposes the router builder and config types so that external crates
//! (e.g. the conformance test suite) can spin up an in-process node without
//! spawning a subprocess.

pub mod config;
pub mod error;
pub mod federation;
pub mod handlers;
pub mod identity;
pub mod middleware;
pub mod peer_discovery;
pub mod router;
pub mod storage;

pub use config::NodeConfig;
pub use middleware::auth::build_outbound_signature;
pub use router::build_router;
pub use storage::{memory::MemoryStorage, Storage};
