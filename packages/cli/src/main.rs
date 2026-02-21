//! `sweft` — SemanticWeft protocol command-line interface.
//!
//! Provides subcommands for working with Semantic Units locally and against a
//! live node:
//!
//! **Local:**
//! - **`validate`** — check a unit or array of units against the spec.
//! - **`render`** — print a human-readable summary of a unit or graph.
//! - **`new`** — create a new unit with an auto-generated id and timestamp.
//! - **`keygen`** — generate an Ed25519 identity key pair.
//!
//! **Network (requires `--node` or `SWEFT_NODE`):**
//! - **`register`** — register an agent profile on a node.
//! - **`submit`** — submit a unit to a node.
//! - **`fetch`** — retrieve a unit or list of units from a node.
//!
//! All local subcommands read JSON from a file path or from stdin (`-`).
//! Network subcommands authenticate with Ed25519 HTTP Signatures; use
//! `sweft keygen` to create your identity key before first use.

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use semanticweft::{validate_unit, Graph, Reference, RelType, SemanticUnit, Source, UnitType};

/// sweft — SemanticWeft protocol CLI
///
/// Validate and inspect Semantic Units and graphs.
#[derive(Parser)]
#[command(name = "sweft", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate one or more Semantic Units against the spec.
    ///
    /// Reads a JSON file containing either a single unit object or an array of
    /// unit objects. Exits 0 if all units are valid, 1 otherwise.
    ///
    /// Pass `-` as FILE to read from stdin.
    Validate {
        /// Path to a JSON file, or `-` for stdin.
        file: PathBuf,
    },

    /// Render a unit or graph as human-readable text.
    ///
    /// Reads a JSON file containing either a single unit object or an array of
    /// unit objects and prints a structured text summary.
    ///
    /// Pass `-` as FILE to read from stdin.
    Render {
        /// Path to a JSON file, or `-` for stdin.
        file: PathBuf,
    },

    /// Create a new Semantic Unit and print it as JSON.
    ///
    /// Generates a UUIDv7 id and UTC timestamp automatically. The result is
    /// printed to stdout and can be piped directly into `sweft validate` or
    /// redirected to a file.
    ///
    /// Examples:
    ///   sweft new -t assertion -c "PCI DSS applies." -a "agent://analyst-1"
    ///   sweft new -t inference -c "..." -a "agent://x" --confidence 0.8 \
    ///     --ref 019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d:derives-from
    New {
        /// Unit type: assertion | question | inference | challenge | constraint
        #[arg(short = 't', long = "type", value_name = "TYPE")]
        unit_type: UnitType,

        /// The human-readable statement this unit expresses.
        #[arg(short = 'c', long, value_name = "TEXT")]
        content: String,

        /// Identifier of the agent creating this unit.
        #[arg(short = 'a', long, value_name = "ID")]
        author: String,

        /// Degree of belief, 0.0–1.0 (recommended for assertion and inference).
        #[arg(long, value_name = "FLOAT")]
        confidence: Option<f64>,

        /// A condition that must hold for this unit's content to be valid.
        /// Repeat for multiple assumptions: --assumption "..." --assumption "..."
        #[arg(long = "assumption", value_name = "TEXT")]
        assumptions: Vec<String>,

        /// Source citation: a URI or free-form string (e.g. "Smith et al., 2024").
        #[arg(short = 's', long, value_name = "URI_OR_CITATION")]
        source: Option<String>,

        /// A typed reference to another unit: <uuid>:<rel>.
        /// <rel> is one of: supports | rebuts | derives-from | questions | refines
        /// Repeat for multiple references: --ref <uuid>:<rel> --ref <uuid>:<rel>
        #[arg(long = "ref", value_name = "UUID:REL")]
        references: Vec<String>,
    },

    /// Generate an Ed25519 identity key pair.
    ///
    /// Creates a new signing key and saves the 32-byte seed (hex-encoded) to a
    /// key file. The corresponding did:key DID and public key multibase are
    /// printed to stdout so you can share them when registering on a node.
    ///
    /// The default key file path is ~/.config/sweft/identity.key. Pass --out
    /// to save elsewhere (e.g. for multiple identities).
    ///
    /// Examples:
    ///   sweft keygen
    ///   sweft keygen --out ~/.config/sweft/work.key
    Keygen {
        /// Where to write the key file (default: ~/.config/sweft/identity.key).
        /// Can also be set via the SWEFT_KEY environment variable.
        #[arg(long, env = "SWEFT_KEY", value_name = "PATH")]
        out: Option<PathBuf>,
    },

    /// Register an agent profile on a SemanticWeft node.
    ///
    /// Authenticates via Ed25519 HTTP Signature using the key at --key (or
    /// ~/.config/sweft/identity.key). The DID and public key are derived
    /// automatically from the key file. Run `sweft keygen` first.
    ///
    /// Examples:
    ///   sweft register --node https://node.example.com \
    ///     --inbox-url https://node.example.com/v1/agents/did:key:z6Mk.../inbox
    ///   sweft register --node https://node.example.com \
    ///     --inbox-url https://... --key ~/.config/sweft/work.key
    Register {
        /// Base URL of the node (e.g. https://node.example.com).
        /// Can also be set via the SWEFT_NODE environment variable.
        #[arg(long, env = "SWEFT_NODE", value_name = "URL")]
        node: String,

        /// The inbox URL for this agent.
        #[arg(long, value_name = "URL")]
        inbox_url: String,

        /// Optional display name.
        #[arg(long, value_name = "NAME")]
        display_name: Option<String>,

        /// Path to the Ed25519 key file (default: ~/.config/sweft/identity.key).
        /// Can also be set via the SWEFT_KEY environment variable.
        #[arg(long, env = "SWEFT_KEY", value_name = "PATH")]
        key: Option<PathBuf>,
    },

    /// Submit a Semantic Unit to a node.
    ///
    /// Reads a unit from a JSON file (or stdin with `-`) and sends it to the
    /// node via POST /v1/units. Public units need no authentication. For
    /// network or limited visibility units, provide --key so the request can
    /// be signed with an HTTP Signature.
    ///
    /// Examples:
    ///   sweft submit --node https://node.example.com unit.json
    ///   sweft new -t assertion -c "test" -a did:key:z6Mk... | \
    ///     sweft submit --node https://node.example.com -
    Submit {
        /// Base URL of the node (e.g. https://node.example.com).
        /// Can also be set via the SWEFT_NODE environment variable.
        #[arg(long, env = "SWEFT_NODE", value_name = "URL")]
        node: String,

        /// Path to a JSON file containing the unit, or `-` for stdin.
        file: PathBuf,

        /// Path to the Ed25519 key file (required for non-public units).
        /// Can also be set via the SWEFT_KEY environment variable.
        #[arg(long, env = "SWEFT_KEY", value_name = "PATH")]
        key: Option<PathBuf>,
    },

    /// Fetch a unit or a list of units from a node.
    ///
    /// With an ID, retrieves that specific unit (GET /v1/units/{id}).
    /// Without an ID, lists units with optional filters (GET /v1/units).
    /// Provide --key to authenticate and receive network-visibility units.
    ///
    /// Examples:
    ///   sweft fetch --node https://node.example.com <uuid>
    ///   sweft fetch --node https://node.example.com --author did:key:z6Mk...
    ///   sweft fetch --node https://node.example.com --type assertion --limit 10
    Fetch {
        /// Base URL of the node (e.g. https://node.example.com).
        /// Can also be set via the SWEFT_NODE environment variable.
        #[arg(long, env = "SWEFT_NODE", value_name = "URL")]
        node: String,

        /// Unit ID to fetch. If omitted, lists units with optional filters.
        id: Option<String>,

        /// Path to the Ed25519 key file (enables network-unit access).
        /// Can also be set via the SWEFT_KEY environment variable.
        #[arg(long, env = "SWEFT_KEY", value_name = "PATH")]
        key: Option<PathBuf>,

        /// Comma-separated unit types to include (e.g. assertion,inference).
        #[arg(long = "type", value_name = "TYPES")]
        unit_type: Option<String>,

        /// Filter by author DID.
        #[arg(long, value_name = "DID")]
        author: Option<String>,

        /// ISO 8601 lower bound on created_at (e.g. 2026-01-01T00:00:00Z).
        #[arg(long, value_name = "ISO8601")]
        since: Option<String>,

        /// Keyset cursor (unit id of the last seen item).
        #[arg(long, value_name = "UUID")]
        after: Option<String>,

        /// Maximum number of units to return (1–500, default 50).
        #[arg(long, value_name = "N")]
        limit: Option<u32>,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Command::Validate { file } => {
            let json = read_input(&file);
            let units = parse_units(&json);
            let mut all_valid = true;
            for (i, unit) in units.iter().enumerate() {
                if let Err(e) = validate_unit(unit) {
                    if units.len() == 1 {
                        eprintln!("error: {}", e);
                    } else {
                        eprintln!("error in unit {}: {}", i, e);
                    }
                    all_valid = false;
                }
            }
            if all_valid {
                if units.len() == 1 {
                    println!("valid");
                } else {
                    println!("all {} units valid", units.len());
                }
            } else {
                process::exit(1);
            }
        }

        Command::Render { file } => {
            let json = read_input(&file);
            let units = parse_units(&json);
            // A single unit is rendered in full detail; multiple units use the
            // grouped graph summary view.
            if units.len() == 1 {
                print!("{}", semanticweft::render::render_unit(&units[0]));
            } else {
                let graph = Graph::from_units(units);
                print!("{}", semanticweft::render::render_graph(&graph));
            }
        }

        Command::New {
            unit_type,
            content,
            author,
            confidence,
            assumptions,
            source,
            references,
        } => {
            // Parse each --ref argument from "<uuid>:<rel>" into a Reference.
            let parsed_refs = if references.is_empty() {
                None
            } else {
                let mut out = Vec::with_capacity(references.len());
                for raw in &references {
                    let (id_str, rel_str) = raw.split_once(':').unwrap_or_else(|| {
                        fatal(&format!(
                            "invalid --ref {:?}: expected format <uuid>:<rel>",
                            raw
                        ))
                    });
                    let rel = rel_str.parse::<RelType>().unwrap_or_else(|e| fatal(&e));
                    out.push(Reference {
                        id: id_str.to_string(),
                        rel,
                    });
                }
                Some(out)
            };

            let mut unit = SemanticUnit::new(unit_type, content, author);
            unit.confidence = confidence;
            // An empty --assumption list means the field is absent, not present-but-empty.
            unit.assumptions = if assumptions.is_empty() {
                None
            } else {
                Some(assumptions)
            };
            unit.source = source.map(Source::Uri);
            unit.references = parsed_refs;

            // Validate before printing so the user gets a clear error rather than
            // silently producing an invalid unit.
            if let Err(e) = validate_unit(&unit) {
                fatal(&format!("unit is invalid: {}", e));
            }

            println!("{}", serde_json::to_string_pretty(&unit).unwrap());
        }

        Command::Keygen { out } => {
            let path = out.unwrap_or_else(default_key_path);

            // Create parent directories if needed.
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|e| {
                    fatal(&format!("failed to create {}: {e}", parent.display()))
                });
            }

            let signing_key = SigningKey::generate(&mut OsRng);
            let hex = hex_encode(&signing_key.to_bytes());

            fs::write(&path, &hex).unwrap_or_else(|e| {
                fatal(&format!("failed to write key file {}: {e}", path.display()))
            });

            // Set restrictive permissions on Unix so the private key is not
            // world-readable.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .unwrap_or_else(|e| {
                        eprintln!("sweft: warning: could not set key file permissions: {e}")
                    });
            }

            let (did, pubkey_multibase) = derive_did_and_pubkey(&signing_key);
            println!("Key file : {}", path.display());
            println!("DID      : {did}");
            println!("Public key: {pubkey_multibase}");
        }

        Command::Register {
            node,
            inbox_url,
            display_name,
            key,
        } => {
            let signing_key = load_key(key);
            let (did, pubkey_multibase) = derive_did_and_pubkey(&signing_key);

            let node = node.trim_end_matches('/');
            let path = format!("/v1/agents/{}", urlencoded(&did));
            let host = extract_host(node);
            let (date, sig) = http_sign(&signing_key, &did, "post", &path, &host);

            let body = serde_json::json!({
                "did": did,
                "inbox_url": inbox_url,
                "display_name": display_name,
                "public_key": pubkey_multibase,
            });

            let client = reqwest::blocking::Client::new();
            let resp = client
                .post(format!("{node}{path}"))
                .header("host", &host)
                .header("date", &date)
                .header("signature", &sig)
                .json(&body)
                .send()
                .unwrap_or_else(|e| fatal(&format!("request failed: {e}")));

            let status = resp.status();
            let text = resp.text().unwrap_or_default();

            if status.is_success() {
                println!("{text}");
            } else {
                eprintln!("sweft: server returned {status}");
                eprintln!("{text}");
                process::exit(1);
            }
        }

        Command::Submit { node, file, key } => {
            let json = read_input(&file);
            // Validate locally before sending.
            let unit = match serde_json::from_str::<SemanticUnit>(&json) {
                Ok(u) => u,
                Err(e) => fatal(&format!("failed to parse unit: {e}")),
            };
            if let Err(e) = validate_unit(&unit) {
                fatal(&format!("unit is invalid: {e}"));
            }

            let node = node.trim_end_matches('/');
            let path = "/v1/units";
            let host = extract_host(node);

            let client = reqwest::blocking::Client::new();
            let mut builder = client
                .post(format!("{node}{path}"))
                .header("host", &host)
                .json(&unit);

            if let Some(signing_key) = key.map(|p| load_key(Some(p))) {
                let (did, _) = derive_did_and_pubkey(&signing_key);
                let (date, sig) = http_sign(&signing_key, &did, "post", path, &host);
                builder = builder.header("date", &date).header("signature", &sig);
            }

            let resp = builder
                .send()
                .unwrap_or_else(|e| fatal(&format!("request failed: {e}")));

            let status = resp.status();
            let text = resp.text().unwrap_or_default();

            if status.is_success() {
                println!("{text}");
            } else {
                eprintln!("sweft: server returned {status}");
                eprintln!("{text}");
                process::exit(1);
            }
        }

        Command::Fetch {
            node,
            id,
            key,
            unit_type,
            author,
            since,
            after,
            limit,
        } => {
            let node = node.trim_end_matches('/');
            let host = extract_host(node);

            let path = if let Some(ref uid) = id {
                format!("/v1/units/{}", urlencoded(uid))
            } else {
                let mut params: Vec<(&str, String)> = Vec::new();
                if let Some(ref t) = unit_type {
                    params.push(("type", t.clone()));
                }
                if let Some(ref a) = author {
                    params.push(("author", a.clone()));
                }
                if let Some(ref s) = since {
                    params.push(("since", s.clone()));
                }
                if let Some(ref c) = after {
                    params.push(("after", c.clone()));
                }
                if let Some(l) = limit {
                    params.push(("limit", l.to_string()));
                }

                let qs: String = params
                    .iter()
                    .enumerate()
                    .map(|(i, (k, v))| {
                        format!(
                            "{}{}={}",
                            if i == 0 { "?" } else { "&" },
                            k,
                            urlencoded(v)
                        )
                    })
                    .collect();

                format!("/v1/units{qs}")
            };

            let client = reqwest::blocking::Client::new();
            let mut builder = client.get(format!("{node}{path}")).header("host", &host);

            if let Some(signing_key) = key.map(|p| load_key(Some(p))) {
                let (did, _) = derive_did_and_pubkey(&signing_key);
                let (date, sig) = http_sign(&signing_key, &did, "get", &path, &host);
                builder = builder.header("date", &date).header("signature", &sig);
            }

            let resp = builder
                .send()
                .unwrap_or_else(|e| fatal(&format!("request failed: {e}")));

            let status = resp.status();
            let text = resp.text().unwrap_or_default();

            if status.is_success() {
                println!("{text}");
            } else {
                eprintln!("sweft: server returned {status}");
                eprintln!("{text}");
                process::exit(1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Key management helpers
// ---------------------------------------------------------------------------

/// Return the default key file path: `~/.config/sweft/identity.key`.
fn default_key_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config").join("sweft").join("identity.key")
}

/// Load an Ed25519 signing key from a hex-encoded seed file.
///
/// The file must contain exactly 64 hex characters (32-byte seed).
/// Exits with a helpful error if the file is missing or malformed.
fn load_key(path: Option<PathBuf>) -> SigningKey {
    let p = path.unwrap_or_else(default_key_path);
    let hex = fs::read_to_string(&p).unwrap_or_else(|e| {
        fatal(&format!(
            "failed to read key file {}: {e}\nRun `sweft keygen` to create one.",
            p.display()
        ))
    });
    let seed = hex_decode(hex.trim())
        .unwrap_or_else(|e| fatal(&format!("invalid key file {}: {e}", p.display())));
    let arr: [u8; 32] = seed
        .try_into()
        .unwrap_or_else(|_| fatal("key file must contain a 32-byte hex seed (64 hex chars)"));
    SigningKey::from_bytes(&arr)
}

/// Derive the `did:key` DID and public key multibase from a signing key.
fn derive_did_and_pubkey(key: &SigningKey) -> (String, String) {
    let pub_bytes = key.verifying_key().to_bytes();
    let mut multicodec = vec![0xed_u8, 0x01];
    multicodec.extend_from_slice(&pub_bytes);
    let encoded = bs58::encode(&multicodec).into_string();
    (format!("did:key:z{encoded}"), format!("z{encoded}"))
}

/// Build `Date` and `Signature` HTTP header values for an outbound request.
///
/// Mirrors `build_outbound_signature` in the node's auth middleware so that
/// the CLI and node agree on the signing string format.
fn http_sign(key: &SigningKey, did: &str, method: &str, path: &str, host: &str) -> (String, String) {
    let date = httpdate::fmt_http_date(std::time::SystemTime::now());
    let signing_string =
        format!("(request-target): {method} {path}\nhost: {host}\ndate: {date}");
    let sig_bytes = key.sign(signing_string.as_bytes()).to_bytes();
    let sig_encoded = format!("z{}", bs58::encode(sig_bytes).into_string());
    let sig_header = format!(
        r#"keyId="{did}",algorithm="ed25519",headers="(request-target) host date",signature="{sig_encoded}""#
    );
    (date, sig_header)
}

/// Extract the `host` component (no scheme, no path) from a node base URL.
fn extract_host(node_url: &str) -> String {
    let stripped = node_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    stripped.split('/').next().unwrap_or(stripped).to_string()
}

// ---------------------------------------------------------------------------
// General helpers
// ---------------------------------------------------------------------------

/// Percent-encode a string for safe inclusion in a URL path or query string.
fn urlencoded(s: &str) -> String {
    // Encode characters outside the unreserved set (RFC 3986 §2.3).
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Hex-encode a byte slice as a lowercase string.
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decode a lowercase hex string into bytes.
fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if s.len() % 2 != 0 {
        return Err("odd-length hex string".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

/// Read the full contents of a file, or stdin when the path is `"-"`.
fn read_input(path: &PathBuf) -> String {
    if path.to_str() == Some("-") {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .unwrap_or_else(|e| fatal(&format!("failed to read stdin: {}", e)));
        buf
    } else {
        fs::read_to_string(path).unwrap_or_else(|e| {
            fatal(&format!("failed to read {}: {}", path.display(), e))
        })
    }
}

/// Parse a JSON string as either an array of units or a single unit.
///
/// Tries the array form first (covering the common "graph file" case), then
/// falls back to parsing as a single object. Exits with an error message if
/// neither parse succeeds.
fn parse_units(json: &str) -> Vec<SemanticUnit> {
    // Try array first, then single object.
    if let Ok(units) = serde_json::from_str::<Vec<SemanticUnit>>(json) {
        if units.is_empty() {
            fatal("input contains an empty array — nothing to process");
        }
        return units;
    }
    match serde_json::from_str::<SemanticUnit>(json) {
        Ok(unit) => vec![unit],
        Err(e) => fatal(&format!("failed to parse input as a Semantic Unit: {}", e)),
    }
}

/// Print an error message to stderr and exit with code 2.
fn fatal(msg: &str) -> ! {
    eprintln!("sweft: {}", msg);
    process::exit(2);
}
