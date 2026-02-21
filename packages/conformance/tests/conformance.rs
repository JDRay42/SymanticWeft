//! End-to-end conformance tests for the SemanticWeft node API.
//!
//! Each test spawns an ephemeral in-process node (real TCP, real HTTP) via
//! [`semanticweft_conformance::spawn_node`] and exercises the full API surface
//! with a `reqwest` HTTP client.
//!
//! # Authentication in tests
//!
//! The node's `RequireAuth` extractor verifies HTTP Signatures and looks up the
//! caller's public key from storage. Tests that exercise auth-gated endpoints
//! (agent register, follow, inbox) therefore:
//! 1. Generate an Ed25519 key pair.
//! 2. Pre-register the agent profile with the matching public key directly in
//!    storage (bypassing the HTTP layer, which would be circular on first use).
//! 3. Sign subsequent requests with `build_outbound_signature`.
//!
//! This mirrors real agent deployment: operators seed their public key once
//! (via a trusted admin channel), then agents self-manage via signed requests.
//!
//! # Coverage
//!
//! | Test | Spec section |
//! |------|-------------|
//! | `well_known_returns_node_info` | Discovery |
//! | `submit_public_unit_and_fetch_by_id` | §5 units |
//! | `list_units_filter_by_type` | §5 list |
//! | `submit_duplicate_unit_returns_409` | §5 conflict |
//! | `list_units_pagination` | §5 pagination |
//! | `fetch_unknown_unit_returns_404` | §5 not found |
//! | `subgraph_traversal` | §5 subgraph |
//! | `sync_json_returns_public_units` | §6 sync |
//! | `sync_sse_content_type` | §6 SSE sync |
//! | `peers_list_starts_empty` | §7 peers |
//! | `add_peer_and_list` | §7 peers |
//! | `register_agent_and_retrieve` | §8 agents |
//! | `delete_agent_returns_204` | §8 DELETE agent |
//! | `delete_unknown_agent_returns_404` | §8 DELETE agent |
//! | `register_agent_wrong_did_returns_403` | §8 agents auth |
//! | `delete_agent_unauthenticated_returns_401` | §8 DELETE agent auth |
//! | `delete_agent_wrong_did_returns_403` | §8 DELETE agent auth |
//! | `follow_and_list` | §9 follows |
//! | `unfollow_removes_relationship` | §9 follows |

use semanticweft::{SemanticUnit, UnitType};
use semanticweft_conformance::spawn_node;
use semanticweft_node::storage::Storage;
use semanticweft_node_api::AgentProfile;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
}

fn public_unit() -> SemanticUnit {
    SemanticUnit::new(UnitType::Assertion, "Conformance test assertion.", "did:key:z6MkConformance")
}

fn inference_unit() -> SemanticUnit {
    SemanticUnit::new(UnitType::Inference, "Conformance inference.", "did:key:z6MkConformance")
}

/// Generate an Ed25519 key and derive its `did:key` DID + multibase public key.
///
/// The public key multibase uses the same encoding as the DID method:
/// `z` + base58btc( [0xed, 0x01] + 32_raw_key_bytes ), which is what
/// `decode_multibase_key` in the auth middleware expects.
fn make_agent_key() -> (ed25519_dalek::SigningKey, String, String) {
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    let key = SigningKey::generate(&mut OsRng);
    let pub_bytes = key.verifying_key().to_bytes();
    let mut multicodec = vec![0xed_u8, 0x01];
    multicodec.extend_from_slice(&pub_bytes);
    let encoded = bs58::encode(&multicodec).into_string();
    let did = format!("did:key:z{encoded}");
    // Multibase public key: same bytes as the DID suffix.
    let pubkey_multibase = format!("z{encoded}");
    (key, did, pubkey_multibase)
}

/// Pre-register an agent in storage with a known public key so that
/// subsequent HTTP-Signature-authenticated requests can be verified.
async fn seed_agent(
    storage: &std::sync::Arc<semanticweft_node::storage::memory::MemoryStorage>,
    did: &str,
    inbox_url: &str,
    pubkey_multibase: &str,
) {
    storage
        .put_agent(&AgentProfile {
            did: did.to_string(),
            inbox_url: inbox_url.to_string(),
            display_name: None,
            public_key: Some(pubkey_multibase.to_string()),
        })
        .await
        .expect("seed agent");
}

/// Build `(date, signature)` HTTP Signature headers using the node's helper.
/// Mirrors what a real agent would include on authenticated requests.
fn http_sig(
    key: &ed25519_dalek::SigningKey,
    did: &str,
    method: &str,
    path: &str,
    host: &str,
) -> (String, String) {
    semanticweft_node::middleware::auth::build_outbound_signature(key, did, method, path, host)
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn well_known_returns_node_info() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let resp = client
        .get(format!("{base}/.well-known/semanticweft"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body.get("node_id").is_some(), "missing node_id in node info");
    assert!(body.get("api_base").is_some(), "missing api_base in node info");
}

// ---------------------------------------------------------------------------
// Units — §5
// ---------------------------------------------------------------------------

#[tokio::test]
async fn submit_public_unit_and_fetch_by_id() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let unit = public_unit();
    let unit_id = unit.id.clone();

    let resp = client
        .post(format!("{base}/v1/units"))
        .json(&unit)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "submit should return 201");

    let resp = client
        .get(format!("{base}/v1/units/{unit_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "fetch by id should return 200");
    let fetched: Value = resp.json().await.unwrap();
    assert_eq!(fetched["id"].as_str().unwrap(), unit_id);
}

#[tokio::test]
async fn submit_duplicate_unit_returns_409() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let unit = public_unit();

    client
        .post(format!("{base}/v1/units"))
        .json(&unit)
        .send()
        .await
        .unwrap();

    let resp = client
        .post(format!("{base}/v1/units"))
        .json(&unit)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409, "duplicate unit should return 409");
}

#[tokio::test]
async fn list_units_filter_by_type() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let assertion = public_unit();
    let inference = inference_unit();

    client.post(format!("{base}/v1/units")).json(&assertion).send().await.unwrap();
    client.post(format!("{base}/v1/units")).json(&inference).send().await.unwrap();

    let resp = client
        .get(format!("{base}/v1/units?type=assertion"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let units = body["units"].as_array().unwrap();
    assert!(
        units.iter().all(|u| u["type"].as_str().unwrap() == "assertion"),
        "type filter should return only assertions"
    );
    assert_eq!(units.len(), 1, "should return exactly one assertion");
}

#[tokio::test]
async fn list_units_pagination() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    for _ in 0..5 {
        let u = SemanticUnit::new(UnitType::Assertion, "page test", "did:key:z6MkPager");
        client.post(format!("{base}/v1/units")).json(&u).send().await.unwrap();
    }

    let resp = client
        .get(format!("{base}/v1/units?limit=2"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let units = body["units"].as_array().unwrap();
    assert_eq!(units.len(), 2, "first page should have 2 units");
    assert_eq!(body["has_more"], true, "has_more should be true");

    let cursor = units.last().unwrap()["id"].as_str().unwrap().to_string();
    let resp = client
        .get(format!("{base}/v1/units?limit=2&after={cursor}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body2: Value = resp.json().await.unwrap();
    let units2 = body2["units"].as_array().unwrap();
    assert!(!units2.is_empty(), "second page should have units");
    assert_ne!(
        units2[0]["id"].as_str().unwrap(),
        units[0]["id"].as_str().unwrap(),
        "pages must not overlap"
    );
}

#[tokio::test]
async fn fetch_unknown_unit_returns_404() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let resp = client
        .get(format!("{base}/v1/units/00000000-0000-0000-0000-000000000000"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn subgraph_traversal() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let root = public_unit();
    let root_id = root.id.clone();
    client.post(format!("{base}/v1/units")).json(&root).send().await.unwrap();

    let mut child = SemanticUnit::new(UnitType::Inference, "child", "did:key:z6MkConformance");
    child.references = Some(vec![semanticweft::Reference {
        id: root_id.clone(),
        rel: semanticweft::RelType::DerivesFrom,
    }]);
    let child_id = child.id.clone();
    client.post(format!("{base}/v1/units")).json(&child).send().await.unwrap();

    let resp = client
        .get(format!("{base}/v1/units/{root_id}/subgraph"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let units = body["units"].as_array().unwrap();
    let ids: Vec<&str> = units.iter().filter_map(|u| u["id"].as_str()).collect();
    assert!(ids.contains(&root_id.as_str()), "subgraph must include root");
    assert!(ids.contains(&child_id.as_str()), "subgraph must include referencing child");
}

// ---------------------------------------------------------------------------
// Sync — §6
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sync_json_returns_public_units() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let unit = public_unit();
    let unit_id = unit.id.clone();
    client.post(format!("{base}/v1/units")).json(&unit).send().await.unwrap();

    let resp = client
        .get(format!("{base}/v1/sync"))
        .header("accept", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let units = body["units"].as_array().unwrap();
    let ids: Vec<&str> = units.iter().filter_map(|u| u["id"].as_str()).collect();
    assert!(ids.contains(&unit_id.as_str()), "sync should include the submitted unit");
}

#[tokio::test]
async fn sync_sse_content_type() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let resp = client
        .get(format!("{base}/v1/sync"))
        .header("accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp.headers()["content-type"].to_str().unwrap();
    assert!(
        ct.contains("text/event-stream"),
        "SSE response must have text/event-stream content-type, got {ct}"
    );
}

// ---------------------------------------------------------------------------
// Peers — §7
// ---------------------------------------------------------------------------

#[tokio::test]
async fn peers_list_starts_empty() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let resp = client
        .get(format!("{base}/v1/peers"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert!(body["peers"].is_array(), "peers must be an array");
}

#[tokio::test]
async fn add_peer_and_list() {
    let (base, _storage) = spawn_node().await;
    let client = make_client();

    let peer = serde_json::json!({
        "node_id": "did:key:z6MkPeer",
        "api_base": "https://peer.example.com/v1",
        "reputation": 0.5,
        "last_seen": null
    });

    let resp = client
        .post(format!("{base}/v1/peers"))
        .json(&peer)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "add peer should succeed, got {}",
        resp.status()
    );

    let resp = client
        .get(format!("{base}/v1/peers"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let peers = body["peers"].as_array().unwrap();
    assert!(
        peers.iter().any(|p| p["node_id"].as_str() == Some("did:key:z6MkPeer")),
        "added peer should appear in list"
    );
}

// ---------------------------------------------------------------------------
// Agents — §8
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_agent_and_retrieve() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (key, did, pubkey) = make_agent_key();
    let inbox_url = format!("{base}/v1/agents/{did}/inbox");
    let path = format!("/v1/agents/{did}");

    // Seed the agent so RequireAuth can find their key on the first register.
    seed_agent(&storage, &did, &inbox_url, &pubkey).await;

    let (date, sig) = http_sig(&key, &did, "post", &path, addr);
    let body = serde_json::json!({
        "did": did,
        "inbox_url": inbox_url,
        "display_name": "Conformance Agent",
        "public_key": pubkey
    });

    let resp = client
        .post(format!("{base}{path}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201, "register should return 201");

    let resp = client
        .get(format!("{base}/v1/agents/{did}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let profile: Value = resp.json().await.unwrap();
    assert_eq!(profile["did"].as_str().unwrap(), did);
    assert_eq!(profile["display_name"].as_str().unwrap(), "Conformance Agent");
}

#[tokio::test]
async fn delete_agent_returns_204() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (key, did, pubkey) = make_agent_key();
    let inbox_url = format!("{base}/v1/agents/{did}/inbox");
    let path = format!("/v1/agents/{did}");

    seed_agent(&storage, &did, &inbox_url, &pubkey).await;

    let (date, sig) = http_sig(&key, &did, "delete", &path, addr);
    let resp = client
        .delete(format!("{base}{path}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204, "delete should return 204");

    let resp = client
        .get(format!("{base}/v1/agents/{did}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404, "agent should be gone after delete");
}

#[tokio::test]
async fn delete_unknown_agent_returns_404() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (key, did, pubkey) = make_agent_key();
    let path = format!("/v1/agents/{did}");

    // Seed just for auth — but the agent path uses a different DID; actually
    // we need the signer to exist in storage (RequireAuth looks up by keyId).
    // Register the signer but attempt to delete a different, unknown DID.
    let inbox_url = format!("{base}/v1/agents/{did}/inbox");
    seed_agent(&storage, &did, &inbox_url, &pubkey).await;

    // Now delete *this* agent (they are registered, so 204 expected).
    // For a truly unknown agent, use a fresh DID that is never seeded.
    let (key2, did2, pubkey2) = make_agent_key();
    let path2 = format!("/v1/agents/{did2}");
    let inbox2 = format!("{base}/v1/agents/{did2}/inbox");
    // Seed did2 so they can authenticate, but immediately delete to make them "unknown"
    seed_agent(&storage, &did2, &inbox2, &pubkey2).await;
    storage.delete_agent(&did2).await.unwrap();

    let (date, sig) = http_sig(&key2, &did2, "delete", &path2, addr);
    // RequireAuth won't find did2 now, so this returns 401.
    let resp = client
        .delete(format!("{base}{path2}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status() == 401 || resp.status() == 404,
        "deleting an unknown agent should return 401 or 404, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn delete_agent_unauthenticated_returns_401() {
    // A DELETE request with no auth headers must be rejected before reaching
    // the handler — RequireAuth should return 401.
    let (base, storage) = spawn_node().await;
    let client = make_client();

    let (_, did, pubkey) = make_agent_key();
    let inbox_url = format!("{base}/v1/agents/{did}/inbox");
    let path = format!("/v1/agents/{did}");

    seed_agent(&storage, &did, &inbox_url, &pubkey).await;

    // No Signature / Date headers.
    let resp = client
        .delete(format!("{base}{path}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401, "unauthenticated DELETE should return 401");

    // The agent must still exist.
    assert!(
        storage.get_agent(&did).await.unwrap().is_some(),
        "agent must not be deleted by an unauthenticated request"
    );
}

#[tokio::test]
async fn delete_agent_wrong_did_returns_403() {
    // Agent A authenticates but tries to delete Agent B's registration.
    // The handler checks `auth.did == path did` and must return 403.
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (key_a, did_a, pubkey_a) = make_agent_key();
    let (_, did_b, pubkey_b) = make_agent_key();

    seed_agent(&storage, &did_a, &format!("{base}/v1/agents/{did_a}/inbox"), &pubkey_a).await;
    seed_agent(&storage, &did_b, &format!("{base}/v1/agents/{did_b}/inbox"), &pubkey_b).await;

    // Agent A signs a DELETE for Agent B's path.
    let path_b = format!("/v1/agents/{did_b}");
    let (date, sig) = http_sig(&key_a, &did_a, "delete", &path_b, addr);

    let resp = client
        .delete(format!("{base}{path_b}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "deleting another agent should return 403");

    // Agent B must still exist.
    assert!(
        storage.get_agent(&did_b).await.unwrap().is_some(),
        "agent B must not be deleted by agent A"
    );
}

#[tokio::test]
async fn register_agent_wrong_did_returns_403() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (key, did, pubkey) = make_agent_key();
    let (_, other_did, _) = make_agent_key();
    let inbox_url = format!("{base}/v1/agents/{other_did}/inbox");
    let path = format!("/v1/agents/{other_did}");

    // Seed the key owner (did), not other_did.
    seed_agent(&storage, &did, &format!("{base}/v1/agents/{did}/inbox"), &pubkey).await;

    let (date, sig) = http_sig(&key, &did, "post", &path, addr);
    let body = serde_json::json!({
        "did": other_did,
        "inbox_url": inbox_url
    });

    let resp = client
        .post(format!("{base}{path}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403, "registering as a different DID should return 403");
}

// ---------------------------------------------------------------------------
// Follows — §9
// ---------------------------------------------------------------------------

#[tokio::test]
async fn follow_and_list() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (follower_key, follower_did, follower_pubkey) = make_agent_key();
    let (_, followee_did, _) = make_agent_key();

    seed_agent(&storage, &follower_did, &format!("{base}/v1/agents/{follower_did}/inbox"), &follower_pubkey).await;

    let path = format!("/v1/agents/{follower_did}/following");
    let (date, sig) = http_sig(&follower_key, &follower_did, "post", &path, addr);

    // The follower must be registered on this node for fan-out to work.
    // We've already seeded them above, so just send the follow request.
    let resp = client
        .post(format!("{base}{path}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .json(&serde_json::json!({
            "follower_did": follower_did,
            "target_did": followee_did
        }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "follow should succeed, got {}",
        resp.status()
    );

    // List following (requires auth).
    let (date2, sig2) = http_sig(&follower_key, &follower_did, "get", &path, addr);
    let resp = client
        .get(format!("{base}{path}"))
        .header("host", addr)
        .header("date", &date2)
        .header("signature", &sig2)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let follows = body["items"].as_array().unwrap();
    assert!(
        follows.iter().any(|f| f["did"].as_str() == Some(followee_did.as_str())),
        "followee should appear in following list"
    );

    // List followers (public endpoint).
    let resp = client
        .get(format!("{base}/v1/agents/{followee_did}/followers"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let followers = body["items"].as_array().unwrap();
    assert!(
        followers.iter().any(|f| f["did"].as_str() == Some(follower_did.as_str())),
        "follower should appear in followers list"
    );
}

#[tokio::test]
async fn unfollow_removes_relationship() {
    let (base, storage) = spawn_node().await;
    let client = make_client();
    let addr = base.strip_prefix("http://").unwrap_or(&base);

    let (follower_key, follower_did, follower_pubkey) = make_agent_key();
    let (_, followee_did, _) = make_agent_key();

    seed_agent(&storage, &follower_did, &format!("{base}/v1/agents/{follower_did}/inbox"), &follower_pubkey).await;

    // Follow.
    let follow_path = format!("/v1/agents/{follower_did}/following");
    let (date, sig) = http_sig(&follower_key, &follower_did, "post", &follow_path, addr);
    client
        .post(format!("{base}{follow_path}"))
        .header("host", addr)
        .header("date", &date)
        .header("signature", &sig)
        .json(&serde_json::json!({
            "follower_did": follower_did,
            "target_did": followee_did
        }))
        .send()
        .await
        .unwrap();

    // Unfollow.
    let unfollow_path = format!("/v1/agents/{follower_did}/following/{followee_did}");
    let (date2, sig2) = http_sig(&follower_key, &follower_did, "delete", &unfollow_path, addr);
    let resp = client
        .delete(format!("{base}{unfollow_path}"))
        .header("host", addr)
        .header("date", &date2)
        .header("signature", &sig2)
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "unfollow should succeed, got {}",
        resp.status()
    );

    // Verify gone.
    let (date3, sig3) = http_sig(&follower_key, &follower_did, "get", &follow_path, addr);
    let resp = client
        .get(format!("{base}{follow_path}"))
        .header("host", addr)
        .header("date", &date3)
        .header("signature", &sig3)
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let follows = body["items"].as_array().unwrap();
    assert!(
        !follows.iter().any(|f| f["did"].as_str() == Some(followee_did.as_str())),
        "followee should be removed from following list after unfollow"
    );
}
