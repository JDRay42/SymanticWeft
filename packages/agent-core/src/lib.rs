//! Pure-logic agent primitives for SemanticWeft.
//!
//! This crate has **no I/O** — no networking, no filesystem, no async
//! runtime.  It compiles to native Rust and to WebAssembly without any
//! changes.  The host environment (native binary, browser, Deno, Python
//! via wasmtime, …) is responsible for all HTTP calls and key persistence;
//! this crate handles identity, address parsing, URL construction, and
//! signing.
//!
//! # Crates that use this
//!
//! | Environment | How it's used |
//! |---|---|
//! | Native Rust agent | Depend on this crate directly; add `reqwest` for I/O |
//! | Browser / Node.js | Via `semanticweft-wasm` (WASM bindings) + `fetch()` |
//! | Any other language | Via `semanticweft-wasm` loaded in a WASM runtime |

pub mod address;
pub mod identity;
pub mod session;

pub use address::{AddressError, AgentAddress};
pub use identity::AgentIdentity;
pub use session::{NodeSession, SessionError};
