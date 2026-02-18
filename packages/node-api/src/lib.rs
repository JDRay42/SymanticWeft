//! Request and response types for the SemanticWeft Node API.
//!
//! This crate encodes the HTTP API contract defined in `spec/node-api.md`
//! as Rust types. It is the machine-readable companion to the prose
//! specification, and the foundation for the Phase 5 reference node
//! implementation.
//!
//! # Endpoints covered
//!
//! | Method | Path | Type |
//! |--------|------|------|
//! | POST | `/v1/units` | [`SubmitRequest`] → [`SubmitResponse`] |
//! | GET | `/v1/units/{id}` | → [`semanticweft::SemanticUnit`] |
//! | GET | `/v1/units` | [`ListQuery`] → [`ListResponse`] |
//! | GET | `/v1/units/{id}/subgraph` | [`SubgraphQuery`] → [`SubgraphResponse`] |
//! | GET | `/v1/sync` | [`ListQuery`] → [`ListResponse`] (+ SSE) |
//! | GET | `/.well-known/semanticweft` | → [`NodeInfo`] |
//! | GET | `/v1/peers` | → [`PeersResponse`] |
//! | POST | `/v1/peers` | [`PeerInfo`] → [`PeerInfo`] |

pub mod error;
pub mod node;
pub mod peer;
pub mod unit;

pub use error::ErrorResponse;
pub use node::{Capability, NodeInfo, PowParams};
pub use peer::{PeerInfo, PeersResponse};
pub use unit::{ListQuery, ListResponse, SubgraphQuery, SubgraphResponse, SubmitResponse};
