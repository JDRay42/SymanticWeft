//! Reference implementation of the SemanticWeft protocol.
//!
//! This crate provides the types, validation logic, graph traversal, and
//! human-readable rendering that make up the SemanticWeft core library.
//! It is the Rust-native foundation for the `sweft` CLI, the
//! `semanticweft-wasm` WebAssembly bindings, and the `semanticweft-node-api`
//! HTTP type layer.
//!
//! # Crate layout
//!
//! | Module | Purpose |
//! |--------|---------|
//! | [`types`] | Core data types: [`SemanticUnit`], [`UnitType`], [`RelType`], [`Reference`], [`Source`] |
//! | [`validation`] | Spec-conformance checking via [`validate_unit`] |
//! | [`graph`] | In-memory graph of units with traversal methods |
//! | [`render`] | Human-readable text rendering of units and graphs |
//!
//! # Quick start
//!
//! ```rust,ignore
//! use semanticweft::{SemanticUnit, UnitType, validate_unit};
//!
//! // Create a unit with auto-generated id and timestamp.
//! let unit = SemanticUnit::new(
//!     UnitType::Assertion,
//!     "The boiling point of water at sea level is 100°C.",
//!     "did:key:z6Mk...",
//! );
//!
//! // Validate it against the spec.
//! validate_unit(&unit).expect("unit should be valid");
//!
//! // Serialise to JSON.
//! let json = serde_json::to_string_pretty(&unit).unwrap();
//! ```
//!
//! # Spec references
//!
//! - Unit format: `spec/semantic-unit.md`
//! - Node API: `spec/node-api.md`

pub mod graph;
pub mod render;
pub mod signing;
pub mod types;
pub mod validation;

pub use graph::Graph;
pub use signing::{sign_unit, verify_proof, ProofError, SigningError};
pub use types::{Proof, Reference, RelType, SemanticUnit, Source, UnitType, Visibility};
pub use validation::{validate_unit, ValidationError};
