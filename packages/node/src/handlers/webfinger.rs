//! WebFinger handler — `GET /.well-known/webfinger` (RFC 7033).
//!
//! Resolves an agent address of the form `did:key:z6Mk…@hostname` to a JSON
//! Resource Descriptor (JRD) linking to the agent's profile on this node.
//!
//! # Request
//!
//! ```text
//! GET /.well-known/webfinger?resource=acct:did:key:z6Mk…@example.com
//! ```
//!
//! The `acct:` prefix is optional; the bare DID address is also accepted.
//!
//! # Response
//!
//! ```json
//! {
//!   "subject": "acct:did:key:z6Mk…@example.com",
//!   "links": [
//!     {
//!       "rel": "self",
//!       "type": "application/json",
//!       "href": "https://example.com/v1/agents/did:key:z6Mk…"
//!     }
//!   ]
//! }
//! ```
//!
//! Returns 404 when the agent is not registered on this node, or 400 when the
//! `resource` parameter is absent or cannot be parsed as an agent address.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::error::AppError;
use super::AppState;

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct WebFingerQuery {
    pub resource: Option<String>,
}

// ---------------------------------------------------------------------------
// JRD types
// ---------------------------------------------------------------------------

/// A JSON Resource Descriptor (JRD) per RFC 7033 §4.4.
#[derive(Serialize)]
pub struct Jrd {
    pub subject: String,
    pub links: Vec<JrdLink>,
}

#[derive(Serialize)]
pub struct JrdLink {
    pub rel: String,
    #[serde(rename = "type")]
    pub link_type: String,
    pub href: String,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// `GET /.well-known/webfinger?resource=acct:{did}@{host}`
///
/// Resolves a SemanticWeft agent address to its profile URL on this node.
pub async fn webfinger(
    State(state): State<AppState>,
    Query(params): Query<WebFingerQuery>,
) -> Result<Response, AppError> {
    let resource = params
        .resource
        .ok_or_else(|| AppError::BadRequest("missing required query parameter: resource".into()))?;

    // Strip the optional "acct:" prefix.
    let address = resource
        .strip_prefix("acct:")
        .unwrap_or(resource.as_str());

    // Split on the last '@' to separate the DID from the host.
    let at_pos = address
        .rfind('@')
        .ok_or_else(|| AppError::BadRequest(format!("resource {resource:?} is not a valid agent address (expected did@host format)")))?;

    let did = &address[..at_pos];

    // Validate it looks like a DID.
    if !did.starts_with("did:") {
        return Err(AppError::BadRequest(format!(
            "resource {resource:?}: expected a DID before '@', got {did:?}"
        )));
    }

    // Look up the agent in storage — 404 if not registered here.
    let _profile = state
        .storage
        .get_agent(did)
        .await
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::NotFound(format!("agent {did} not found on this node")))?;

    // Build the profile URL: api_base already ends with "/v1", so append the path.
    let encoded_did = urlencoding::encode(did);
    let profile_href = format!("{}/agents/{encoded_did}", state.config.api_base);

    let subject = format!("acct:{address}");

    let jrd = Jrd {
        subject,
        links: vec![JrdLink {
            rel: "self".into(),
            link_type: "application/json".into(),
            href: profile_href,
        }],
    };

    // RFC 7033 §10.2 requires Content-Type: application/jrd+json.
    let resp = (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/jrd+json; charset=utf-8",
        )],
        Json(jrd),
    )
        .into_response();

    Ok(resp)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        body::Body,
        http::{Request, StatusCode},
        Router,
    };
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use semanticweft_node_api::AgentProfile;
    use tower::ServiceExt;

    use crate::{config::NodeConfig, router::build_router, storage::{memory::MemoryStorage, Storage}};

    fn build_app(storage: Arc<dyn Storage>) -> Router {
        let config = NodeConfig {
            node_id: "did:key:zNode".into(),
            name: None,
            api_base: "http://localhost/v1".into(),
            contact: None,
            bind_addr: "127.0.0.1:3000".parse().unwrap(),
            db_path: None,
            sync_interval_secs: 60,
            bootstrap_peers: vec![],
            max_peers: 100,
            public_key: None,
            rate_limit_per_minute: 0,
        };
        let signing_key = Arc::new(SigningKey::generate(&mut OsRng));
        build_router(storage, config, signing_key).0
    }

    #[tokio::test]
    async fn registered_agent_returns_jrd() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let did = "did:key:z6MkAgent123";
        storage
            .put_agent(&AgentProfile {
                did: did.to_string(),
                inbox_url: format!("http://localhost/v1/agents/{did}/inbox"),
                display_name: None,
                public_key: None,
            })
            .await
            .unwrap();

        let app = build_app(storage);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/.well-known/webfinger?resource=acct:{did}@localhost"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert!(ct.contains("application/jrd+json"));
    }

    #[tokio::test]
    async fn unknown_agent_returns_404() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let app = build_app(storage);

        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/webfinger?resource=acct:did:key:z6MkUnknown@localhost")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn missing_resource_returns_400() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let app = build_app(storage);

        let req = Request::builder()
            .method("GET")
            .uri("/.well-known/webfinger")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn bare_did_without_acct_prefix_also_works() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let did = "did:key:z6MkAgent456";
        storage
            .put_agent(&AgentProfile {
                did: did.to_string(),
                inbox_url: format!("http://localhost/v1/agents/{did}/inbox"),
                display_name: None,
                public_key: None,
            })
            .await
            .unwrap();

        let app = build_app(storage);
        let req = Request::builder()
            .method("GET")
            .uri(format!("/.well-known/webfinger?resource={did}@localhost"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
