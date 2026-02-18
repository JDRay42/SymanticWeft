//! Unit submission, retrieval, and listing types (spec §5).

use serde::{Deserialize, Serialize};

use semanticweft::SemanticUnit;

// ---------------------------------------------------------------------------
// Submit
// ---------------------------------------------------------------------------

/// The request body for `POST /v1/units` is a bare [`SemanticUnit`].
///
/// There is no wrapper — the unit JSON is posted directly. This type alias
/// documents intent.
pub type SubmitRequest = SemanticUnit;

/// The response body for a successful `POST /v1/units`.
///
/// The node echoes the stored unit back. This is the same type as the
/// request body; the alias documents the 201 vs 200 distinction at the
/// call-site.
pub type SubmitResponse = SemanticUnit;

// ---------------------------------------------------------------------------
// List / Sync
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/units` and `GET /v1/sync` (spec §4.4).
///
/// All fields are optional. Unrecognised parameters MUST be ignored by nodes.
///
/// # Serialisation note
///
/// This struct is intended to be constructed by node implementations from
/// URL query string parsing. HTTP framework integration (e.g., `axum`,
/// `actix-web`) is not included in this crate.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    /// Filter: include only units of these types (ANDed with other filters;
    /// multiple values are ORed with each other).
    ///
    /// Valid values: `assertion`, `question`, `inference`, `challenge`,
    /// `constraint`.
    pub unit_types: Vec<String>,

    /// Filter: include only units whose `author` exactly matches this value.
    pub author: Option<String>,

    /// Filter: include only units whose `created_at` is at or after this
    /// ISO 8601 timestamp.
    pub since: Option<String>,

    /// Pagination cursor: include only units whose `id` is lexicographically
    /// after this UUIDv7 string.
    pub after: Option<String>,

    /// Maximum number of units to return. Default 50, max 500.
    pub limit: Option<u32>,
}

impl ListQuery {
    /// Effective limit, clamped to [1, 500]. Returns 50 if not specified.
    pub fn effective_limit(&self) -> u32 {
        self.limit.map(|l| l.clamp(1, 500)).unwrap_or(50)
    }
}

/// Response body for `GET /v1/units` and `GET /v1/sync` (spec §4.3).
///
/// # Example
///
/// ```json
/// {
///   "units": [ { ... }, { ... } ],
///   "cursor": "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d",
///   "has_more": true
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ListResponse {
    /// The units on this page, in ascending `id` order.
    pub units: Vec<SemanticUnit>,

    /// The `id` of the last unit in `units`. Pass as `?after=` to fetch the
    /// next page. Absent when `units` is empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,

    /// `true` if there are more units after this page.
    pub has_more: bool,
}

impl ListResponse {
    /// Construct a response from a page of units and a total-availability flag.
    ///
    /// The `cursor` is set to the `id` of the last unit in `units`.
    pub fn from_page(units: Vec<SemanticUnit>, has_more: bool) -> Self {
        let cursor = units.last().map(|u| u.id.clone());
        Self {
            units,
            cursor,
            has_more,
        }
    }

    /// Construct an empty (end-of-stream) response.
    pub fn empty() -> Self {
        Self {
            units: vec![],
            cursor: None,
            has_more: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Subgraph
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/units/{id}/subgraph` (spec §5.4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubgraphQuery {
    /// Maximum traversal depth in either direction.
    /// Nodes MUST support at least 10. `None` means unbounded.
    pub depth: Option<u32>,
}

/// Response body for `GET /v1/units/{id}/subgraph` (spec §5.4).
///
/// Contains all units in the connected subgraph that the node holds.
/// Units referenced but not held locally are silently omitted.
///
/// # Example
///
/// ```json
/// { "units": [ { ... }, { ... } ] }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubgraphResponse {
    /// All units in the connected subgraph held by this node.
    /// Order is unspecified.
    pub units: Vec<SemanticUnit>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn unit(id: &str) -> SemanticUnit {
        SemanticUnit {
            id: id.into(),
            unit_type: semanticweft::UnitType::Assertion,
            content: "test".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "test-agent".into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: None,
            extensions: HashMap::new(),
        }
    }

    #[test]
    fn list_response_cursor_is_last_id() {
        let units = vec![
            unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c"),
            unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d"),
        ];
        let resp = ListResponse::from_page(units, false);
        assert_eq!(
            resp.cursor.as_deref(),
            Some("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d")
        );
        assert!(!resp.has_more);
    }

    #[test]
    fn list_response_empty() {
        let resp = ListResponse::empty();
        assert!(resp.units.is_empty());
        assert!(resp.cursor.is_none());
        assert!(!resp.has_more);
    }

    #[test]
    fn list_response_roundtrip() {
        let u = unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c");
        let resp = ListResponse::from_page(vec![u], true);
        let json = serde_json::to_string(&resp).unwrap();
        let back: ListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cursor, resp.cursor);
        assert!(back.has_more);
        assert_eq!(back.units.len(), 1);
    }

    #[test]
    fn subgraph_response_roundtrip() {
        let resp = SubgraphResponse {
            units: vec![
                unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c"),
                unit("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d"),
            ],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: SubgraphResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.units.len(), 2);
    }

    #[test]
    fn list_query_effective_limit() {
        let mut q = ListQuery::default();
        assert_eq!(q.effective_limit(), 50);
        q.limit = Some(0);
        assert_eq!(q.effective_limit(), 1);
        q.limit = Some(1000);
        assert_eq!(q.effective_limit(), 500);
        q.limit = Some(100);
        assert_eq!(q.effective_limit(), 100);
    }
}
