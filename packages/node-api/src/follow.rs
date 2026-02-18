//! Follow-graph types — `POST/DELETE/GET /v1/agents/{did}/following` (spec §8.3, ADR-0007).
//!
//! An agent "follows" another agent to receive that agent's `network`-visibility
//! units via fan-out. The home node of the followed agent sends units to the
//! follower's inbox when they are published.

use serde::{Deserialize, Serialize};

/// Request body for `POST /v1/agents/{did}/following` — follow an agent.
///
/// After a successful follow, the node will fan-out `network`-visibility units
/// from `target_did` to `did`'s inbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FollowRequest {
    /// The DID of the agent that wants to follow.
    pub follower_did: String,

    /// The DID of the agent to follow.
    pub target_did: String,
}

/// A single entry in a follow list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FollowEntry {
    /// DID of the agent in this follow relationship.
    pub did: String,

    /// Optional inbox URL cached from the agent's profile, for display purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbox_url: Option<String>,
}

/// Paginated list of follow relationships.
///
/// Returned by both:
/// - `GET /v1/agents/{did}/following` — agents that `did` follows
/// - `GET /v1/agents/{did}/followers` — agents that follow `did`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FollowListResponse {
    /// Entries in this page of the follow list.
    pub items: Vec<FollowEntry>,

    /// Cursor for the next page; absent when this is the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

impl FollowListResponse {
    /// Construct an empty follow list (no relationships).
    pub fn empty() -> Self {
        Self {
            items: vec![],
            next_cursor: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follow_request_roundtrip() {
        let req = FollowRequest {
            follower_did: "did:key:z6MkFollower".into(),
            target_did: "did:key:z6MkTarget".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: FollowRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn follow_list_empty() {
        let resp = FollowListResponse::empty();
        assert!(resp.items.is_empty());
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("next_cursor"));
    }

    #[test]
    fn follow_list_with_entries() {
        let resp = FollowListResponse {
            items: vec![
                FollowEntry {
                    did: "did:key:z6MkA".into(),
                    inbox_url: Some("https://node.example.com/v1/agents/did%3Akey%3Az6MkA/inbox".into()),
                },
                FollowEntry {
                    did: "did:key:z6MkB".into(),
                    inbox_url: None,
                },
            ],
            next_cursor: Some("019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c".into()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: FollowListResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back, resp);
        assert_eq!(back.items.len(), 2);
    }
}
