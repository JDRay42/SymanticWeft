//! Node discovery handler — `GET /.well-known/semanticweft` (spec §6).

use axum::{extract::State, Json};
use semanticweft_node_api::{Capability, NodeInfo};

use super::AppState;

/// `GET /.well-known/semanticweft`
///
/// Returns this node's discovery document. Agents and peers use it to learn
/// the node's stable identifier, API base URL, and supported capabilities.
pub async fn well_known(State(state): State<AppState>) -> Json<NodeInfo> {
    let cfg = &state.config;
    let mut info = NodeInfo::new(&cfg.node_id, &cfg.api_base);
    info.name = cfg.name.clone();
    info.contact = cfg.contact.clone();
    info.public_key = cfg.public_key.clone();
    info.capabilities = vec![
        Capability::Sync,
        Capability::Subgraph,
        Capability::Peers,
        Capability::Agents,
        Capability::Follows,
    ];
    Json(info)
}
