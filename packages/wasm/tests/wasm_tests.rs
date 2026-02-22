//! wasm-bindgen-test integration tests for the SemanticWeft WASM bindings.
//!
//! Run with:
//!   wasm-pack test packages/wasm --node
//!
//! These tests compile to WASM and execute in a Node.js process, verifying
//! the exported API surface works end-to-end in a JavaScript host.

use wasm_bindgen_test::*;

// Configure all tests in this file to run in Node.js (no browser required).
wasm_bindgen_test_configure!(run_in_node_experimental);

use semanticweft_wasm::{new_unit, parse_agent_address, render, validate, AgentIdentity};

// ---------------------------------------------------------------------------
// validate()
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn validate_valid_unit_succeeds() {
    let json = new_unit("assertion", "The sky is blue.", "did:key:z6MkTest", None).unwrap();
    validate(&json).expect("valid unit should pass validate()");
}

#[wasm_bindgen_test]
fn validate_invalid_json_returns_err() {
    let result = validate("not json at all");
    assert!(result.is_err(), "invalid JSON should fail validate()");
}

#[wasm_bindgen_test]
fn validate_missing_required_field_returns_err() {
    // Missing `author` field.
    let json = r#"{"id":"01950000-0000-7000-8000-000000000000","type":"assertion","content":"hi","created_at":"2026-01-01T00:00:00Z"}"#;
    let result = validate(json);
    assert!(result.is_err(), "unit missing author should fail validate()");
}

// ---------------------------------------------------------------------------
// new_unit()
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn new_unit_assertion_is_valid_json() {
    let json = new_unit("assertion", "CO₂ is rising.", "did:key:z6MkTest", None).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("new_unit output must be valid JSON");
    assert_eq!(parsed["type"].as_str(), Some("assertion"));
    assert_eq!(parsed["content"].as_str(), Some("CO₂ is rising."));
    assert_eq!(parsed["author"].as_str(), Some("did:key:z6MkTest"));
}

#[wasm_bindgen_test]
fn new_unit_all_types_accepted() {
    for t in &["assertion", "question", "inference", "challenge", "constraint"] {
        new_unit(t, "content", "did:key:z6MkTest", None)
            .unwrap_or_else(|e| panic!("type {t} should be accepted, got {e:?}"));
    }
}

#[wasm_bindgen_test]
fn new_unit_unknown_type_returns_err() {
    let result = new_unit("bogus", "content", "did:key:z6MkTest", None);
    assert!(result.is_err(), "unknown type should return an error");
}

#[wasm_bindgen_test]
fn new_unit_with_options() {
    let opts = r#"{"confidence":0.8,"assumptions":["Trend continues."]}"#.to_string();
    let json = new_unit("inference", "Warming will increase.", "did:key:z6MkTest", Some(opts))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let conf = parsed["confidence"].as_f64().unwrap();
    assert!((conf - 0.8).abs() < 1e-9, "confidence should be 0.8, got {conf}");
    assert_eq!(
        parsed["assumptions"].as_array().unwrap()[0].as_str(),
        Some("Trend continues.")
    );
}

// ---------------------------------------------------------------------------
// render()
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn render_single_unit_contains_content() {
    let json = new_unit("assertion", "unique-render-marker", "did:key:z6MkTest", None).unwrap();
    let text = render(&json).unwrap();
    assert!(
        text.contains("unique-render-marker"),
        "render output should contain unit content, got: {text}"
    );
}

#[wasm_bindgen_test]
fn render_empty_array_returns_err() {
    let result = render("[]");
    assert!(result.is_err(), "empty array should return an error");
}

// ---------------------------------------------------------------------------
// AgentIdentity
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn agent_identity_new_produces_valid_did() {
    let id = AgentIdentity::new();
    let did = id.did();
    assert!(did.starts_with("did:key:z6Mk"), "DID should start with did:key:z6Mk, got {did}");
}

#[wasm_bindgen_test]
fn agent_identity_seed_hex_roundtrip() {
    let id = AgentIdentity::new();
    let hex = id.seed_hex();
    assert_eq!(hex.len(), 64, "seed hex should be 64 chars");

    let restored = AgentIdentity::from_seed_hex(&hex).unwrap();
    assert_eq!(id.did(), restored.did(), "DID must survive seed roundtrip");
}

#[wasm_bindgen_test]
fn agent_identity_invalid_seed_hex_returns_err() {
    let result = AgentIdentity::from_seed_hex("not-hex");
    assert!(result.is_err(), "invalid hex should return an error");
}

#[wasm_bindgen_test]
fn agent_identity_public_key_multibase_has_z_prefix() {
    let id = AgentIdentity::new();
    assert!(
        id.public_key_multibase().starts_with('z'),
        "public key multibase should start with z"
    );
}

// ---------------------------------------------------------------------------
// parseAgentAddress()
// ---------------------------------------------------------------------------

#[wasm_bindgen_test]
fn parse_agent_address_valid() {
    let result = parse_agent_address("did:key:z6MkTest@sweft.example.com").unwrap();
    // The function returns a JSON string as JsValue; in the Node.js test
    // environment we can convert it to a string and check key fields.
    let s = result.as_string().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
    assert_eq!(parsed["did"].as_str(), Some("did:key:z6MkTest"));
    assert_eq!(parsed["hostname"].as_str(), Some("sweft.example.com"));
    assert!(parsed["wellKnownUrl"]
        .as_str()
        .unwrap()
        .contains("/.well-known/semanticweft"));
}

#[wasm_bindgen_test]
fn parse_agent_address_missing_at_returns_err() {
    let result = parse_agent_address("did:key:z6MkTestNoAt");
    assert!(result.is_err(), "address without @ should return an error");
}
