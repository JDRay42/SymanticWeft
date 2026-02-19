//! WebAssembly bindings for the SemanticWeft core and agent-core libraries.
//!
//! Exposes two groups of APIs to JavaScript/TypeScript via `wasm-bindgen`.
//! Compile with `wasm-pack build` to produce an npm-ready package that works
//! in browsers, Node.js, and any other WASM host.
//!
//! ## Unit API — [`validate`], [`new_unit`], [`render`]
//!
//! ```js
//! import init, { validate, new_unit, render } from './semanticweft_wasm.js';
//! await init();
//!
//! const json = new_unit('assertion', 'CO₂ is rising.', 'did:key:z6Mk...');
//! validate(json);
//! console.log(render(json));
//! ```
//!
//! ## Agent API — [`AgentIdentity`], [`parse_agent_address`], [`node_session_from_discovery`]
//!
//! ```js
//! // Generate (or restore) an identity.
//! const id = new AgentIdentity();           // fresh keypair
//! const id = AgentIdentity.fromSeedHex(savedHex);  // restore
//! const did = id.did;
//! localStorage.setItem('seed', id.seedHex); // host persists the seed
//!
//! // Parse a recipient address.
//! const addr = parse_agent_address('did:key:z6Mk…@sweft.example.com');
//! // addr: { did, hostname, wellKnownUrl }
//!
//! // Resolve a session after fetching the discovery doc (I/O is yours).
//! const discovery = await fetch(addr.wellKnownUrl).then(r => r.text());
//! const session = node_session_from_discovery(discovery, id.did);
//! // session: { apiBase, inboxUrl, registerUrl, unitsUrl, peersUrl }
//! ```

use wasm_bindgen::prelude::*;

/// One-time initialisation called at the start of every exported function.
///
/// Installs the `console_error_panic_hook` when the feature is enabled so
/// that Rust panics are forwarded to the browser console as readable errors
/// rather than appearing as generic "unreachable" WASM traps.
fn setup() {
    #[cfg(feature = "console_error_panic_hook")]
    console_error_panic_hook::set_once();
}

/// Validate a Semantic Unit given as a JSON string.
///
/// Returns `undefined` on success.  Throws a descriptive string on failure
/// (either a JSON parse error or a spec-validation error).
///
/// ```js
/// import init, { validate } from './semanticweft_wasm.js';
/// await init();
/// validate(unitJson); // throws if invalid
/// ```
#[wasm_bindgen]
pub fn validate(json: &str) -> Result<(), JsValue> {
    setup();
    let unit: semanticweft::SemanticUnit = serde_json::from_str(json)
        .map_err(|e| JsValue::from_str(&format!("parse error: {e}")))?;
    semanticweft::validate_unit(&unit).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Create a new Semantic Unit and return it as a pretty-printed JSON string.
///
/// `unit_type` must be one of: `assertion`, `question`, `inference`,
/// `challenge`, `constraint`.
///
/// `options` is an optional JSON object supporting the following keys:
/// ```json
/// {
///   "confidence": 0.8,
///   "assumptions": ["assumption text", "..."],
///   "source": "https://example.com/paper"
/// }
/// ```
///
/// Throws a string error if the type is unknown or the resulting unit fails
/// validation.
///
/// ```js
/// const json = new_unit('assertion', 'CO₂ is rising.', 'did:key:z6Mk...');
/// const json2 = new_unit('inference', '...', 'did:key:z6Mk...',
///   JSON.stringify({ confidence: 0.8, assumptions: ['Trend continues.'] }));
/// ```
#[wasm_bindgen]
pub fn new_unit(
    unit_type: &str,
    content: &str,
    author: &str,
    options: Option<String>,
) -> Result<String, JsValue> {
    setup();

    let t: semanticweft::UnitType = unit_type
        .parse()
        .map_err(|e: String| JsValue::from_str(&e))?;

    let mut unit =
        semanticweft::SemanticUnit::new(t, content.to_string(), author.to_string());

    if let Some(opts_json) = options {
        let opts: serde_json::Value = serde_json::from_str(&opts_json)
            .map_err(|e| JsValue::from_str(&format!("options parse error: {e}")))?;

        if let Some(c) = opts.get("confidence").and_then(|v| v.as_f64()) {
            unit.confidence = Some(c);
        }
        if let Some(arr) = opts.get("assumptions").and_then(|v| v.as_array()) {
            let a: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !a.is_empty() {
                unit.assumptions = Some(a);
            }
        }
        if let Some(s) = opts.get("source").and_then(|v| v.as_str()) {
            unit.source = Some(semanticweft::Source::Uri(s.to_string()));
        }
    }

    semanticweft::validate_unit(&unit)
        .map_err(|e| JsValue::from_str(&format!("validation error: {e}")))?;

    serde_json::to_string_pretty(&unit).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Render a unit or graph as human-readable text.
///
/// `json` must be either a single unit object or an array of unit objects.
///
/// A single unit is rendered in full detail; an array is summarised as a
/// grouped graph view.
///
/// ```js
/// const text = render(unitJson);
/// const text = render(JSON.stringify([unit1, unit2, unit3]));
/// ```
#[wasm_bindgen]
pub fn render(json: &str) -> Result<String, JsValue> {
    setup();

    // Try array first, then single object — same logic as the CLI.
    if let Ok(units) = serde_json::from_str::<Vec<semanticweft::SemanticUnit>>(json) {
        if units.is_empty() {
            return Err(JsValue::from_str("input array is empty"));
        }
        if units.len() == 1 {
            return Ok(semanticweft::render::render_unit(&units[0]));
        }
        let graph = semanticweft::Graph::from_units(units);
        return Ok(semanticweft::render::render_graph(&graph));
    }

    let unit: semanticweft::SemanticUnit = serde_json::from_str(json)
        .map_err(|e| JsValue::from_str(&format!("parse error: {e}")))?;
    Ok(semanticweft::render::render_unit(&unit))
}

// ── Agent API ─────────────────────────────────────────────────────────────────

/// An agent's Ed25519 identity.
///
/// Holds the private key in WASM memory.  The host is responsible for
/// persisting [`seedHex`] and restoring it via [`fromSeedHex`] on the next
/// run.  The seed is secret — never transmit it over the network.
#[wasm_bindgen]
pub struct AgentIdentity {
    inner: semanticweft_agent_core::AgentIdentity,
}

#[wasm_bindgen]
impl AgentIdentity {
    /// Generate a fresh identity using the host's random number source.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        setup();
        Self {
            inner: semanticweft_agent_core::AgentIdentity::generate(),
        }
    }

    /// Restore an identity from a previously saved hex-encoded seed.
    ///
    /// Throws if `seed_hex` is not exactly 64 hex characters (32 bytes).
    #[wasm_bindgen(js_name = fromSeedHex)]
    pub fn from_seed_hex(seed_hex: &str) -> Result<AgentIdentity, JsValue> {
        setup();
        let bytes = hex_to_32_bytes(seed_hex)?;
        Ok(Self {
            inner: semanticweft_agent_core::AgentIdentity::from_seed(&bytes),
        })
    }

    /// The agent's `did:key` DID, e.g. `did:key:z6Mk…`.
    ///
    /// This is the stable public identity.  Share it freely.
    #[wasm_bindgen(getter)]
    pub fn did(&self) -> String {
        self.inner.did()
    }

    /// The seed as a lowercase hex string (64 characters = 32 bytes).
    ///
    /// **Keep this secret.**  Pass it to [`fromSeedHex`] on the next run.
    #[wasm_bindgen(getter, js_name = seedHex)]
    pub fn seed_hex(&self) -> String {
        self.inner
            .seed()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// The public key in multibase format (`z`-prefixed base58btc).
    ///
    /// Safe to share.  Matches the value stored in `AgentProfile.public_key`
    /// on the node.
    #[wasm_bindgen(getter, js_name = publicKeyMultibase)]
    pub fn public_key_multibase(&self) -> String {
        self.inner.public_key_multibase()
    }

    /// Sign `message` bytes with the agent's private key.
    ///
    /// Returns the raw 64-byte Ed25519 signature as a `Uint8Array`.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.inner.sign(message)
    }
}

/// Parse an agent address string (`did:key:z6Mk…@hostname`) and return a
/// plain JS object with the following fields:
///
/// ```json
/// {
///   "did":          "did:key:z6Mk…",
///   "hostname":     "sweft.example.com",
///   "nodeUrl":      "https://sweft.example.com",
///   "wellKnownUrl": "https://sweft.example.com/.well-known/semanticweft"
/// }
/// ```
///
/// Throws a descriptive string on parse failure.
#[wasm_bindgen(js_name = parseAgentAddress)]
pub fn parse_agent_address(s: &str) -> Result<JsValue, JsValue> {
    setup();
    let addr = semanticweft_agent_core::AgentAddress::parse(s)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let obj = serde_json::json!({
        "did":          addr.did,
        "hostname":     addr.hostname,
        "nodeUrl":      addr.node_url(),
        "wellKnownUrl": addr.well_known_url(),
    });
    Ok(JsValue::from_str(&obj.to_string()))
}

/// Build a node session from a discovery document JSON string and the
/// agent's own DID.  Returns a plain JS object with the following fields:
///
/// ```json
/// {
///   "apiBase":     "https://sweft.example.com/v1",
///   "inboxUrl":    "https://sweft.example.com/v1/agents/did%3A…/inbox",
///   "registerUrl": "https://sweft.example.com/v1/agents/did%3A…",
///   "unitsUrl":    "https://sweft.example.com/v1/units",
///   "peersUrl":    "https://sweft.example.com/v1/peers"
/// }
/// ```
///
/// `discovery_json` is the raw response body from
/// `GET /.well-known/semanticweft`.  Throws on parse failure or if
/// `api_base` is missing from the document.
#[wasm_bindgen(js_name = nodeSessionFromDiscovery)]
pub fn node_session_from_discovery(
    discovery_json: &str,
    own_did: &str,
) -> Result<JsValue, JsValue> {
    setup();
    let session =
        semanticweft_agent_core::NodeSession::from_discovery(discovery_json, own_did)
            .map_err(|e| JsValue::from_str(&e.to_string()))?;

    let obj = serde_json::json!({
        "apiBase":     session.api_base,
        "inboxUrl":    session.inbox_url(),
        "registerUrl": session.register_url(),
        "unitsUrl":    session.units_url(),
        "peersUrl":    session.peers_url(),
    });
    Ok(JsValue::from_str(&obj.to_string()))
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn hex_to_32_bytes(hex: &str) -> Result<[u8; 32], JsValue> {
    if hex.len() != 64 {
        return Err(JsValue::from_str(&format!(
            "seed_hex must be 64 hex characters (32 bytes), got {}",
            hex.len()
        )));
    }
    let mut bytes = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        bytes[i] = (hi << 4) | lo;
    }
    Ok(bytes)
}

fn hex_nibble(b: u8) -> Result<u8, JsValue> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(JsValue::from_str(&format!("invalid hex character: {}", b as char))),
    }
}
