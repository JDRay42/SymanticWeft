//! WebAssembly bindings for the SemanticWeft core library.
//!
//! Exposes [`validate`], [`new_unit`], and [`render`] to JavaScript/TypeScript
//! via `wasm-bindgen`. Compile with `wasm-pack build` to produce an npm-ready
//! package that works in browsers, Node.js, and any other WASM host.
//!
//! # Usage (JavaScript)
//!
//! ```js
//! import init, { validate, new_unit, render } from './semanticweft_wasm.js';
//! await init();
//!
//! // Create a unit.
//! const json = new_unit('assertion', 'CO₂ is rising.', 'did:key:z6Mk...');
//!
//! // Validate it (throws a string on error).
//! validate(json);
//!
//! // Render it as human-readable text.
//! console.log(render(json));
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
