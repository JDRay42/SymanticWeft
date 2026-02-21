//! `sweft` — SemanticWeft protocol command-line interface.
//!
//! Provides subcommands for working with Semantic Units locally and against a
//! live node:
//!
//! **Local:**
//! - **`validate`** — check a unit or array of units against the spec.
//! - **`render`** — print a human-readable summary of a unit or graph.
//! - **`new`** — create a new unit with an auto-generated id and timestamp.
//!
//! **Network (requires `--node` or `SWEFT_NODE`):**
//! - **`register`** — register an agent profile on a node.
//! - **`submit`** — submit a unit to a node.
//! - **`fetch`** — retrieve a unit or list of units from a node.
//!
//! All local subcommands read JSON from a file path or from stdin (`-`).

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
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

    /// Register an agent profile on a SemanticWeft node.
    ///
    /// Sends POST /v1/agents/{did} with the provided profile fields.
    /// Authentication is required: supply a Bearer token via --token or
    /// the SWEFT_TOKEN environment variable.
    ///
    /// Examples:
    ///   sweft register --node https://node.example.com \
    ///     --did did:key:z6Mk... \
    ///     --inbox-url https://node.example.com/v1/agents/did:key:z6Mk.../inbox \
    ///     --token <jwt>
    Register {
        /// Base URL of the node (e.g. https://node.example.com).
        /// Can also be set via the SWEFT_NODE environment variable.
        #[arg(long, env = "SWEFT_NODE", value_name = "URL")]
        node: String,

        /// The DID to register.
        #[arg(long, value_name = "DID")]
        did: String,

        /// The inbox URL for this agent.
        #[arg(long, value_name = "URL")]
        inbox_url: String,

        /// Optional display name.
        #[arg(long, value_name = "NAME")]
        display_name: Option<String>,

        /// Optional Ed25519 public key (multibase-encoded).
        #[arg(long, value_name = "KEY")]
        public_key: Option<String>,

        /// Bearer token for authentication (DID auth).
        /// Can also be set via the SWEFT_TOKEN environment variable.
        #[arg(long, env = "SWEFT_TOKEN", value_name = "TOKEN")]
        token: String,
    },

    /// Submit a Semantic Unit to a node.
    ///
    /// Reads a unit from a JSON file (or stdin with `-`) and sends it to the
    /// node via POST /v1/units. For non-public units, provide an auth token.
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

        /// Bearer token for authentication (required for non-public units).
        /// Can also be set via the SWEFT_TOKEN environment variable.
        #[arg(long, env = "SWEFT_TOKEN", value_name = "TOKEN")]
        token: Option<String>,
    },

    /// Fetch a unit or a list of units from a node.
    ///
    /// With an ID, retrieves that specific unit (GET /v1/units/{id}).
    /// Without an ID, lists units with optional filters (GET /v1/units).
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

        /// Bearer token for authentication (grants access to network units).
        /// Can also be set via the SWEFT_TOKEN environment variable.
        #[arg(long, env = "SWEFT_TOKEN", value_name = "TOKEN")]
        token: Option<String>,

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

        Command::Register {
            node,
            did,
            inbox_url,
            display_name,
            public_key,
            token,
        } => {
            let url = format!("{}/v1/agents/{}", node.trim_end_matches('/'), urlencoded(&did));
            let body = serde_json::json!({
                "did": did,
                "inbox_url": inbox_url,
                "display_name": display_name,
                "public_key": public_key,
            });

            let client = reqwest::blocking::Client::new();
            let mut req = client
                .post(&url)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .json(&body);

            let resp = req
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

        Command::Submit { node, file, token } => {
            let json = read_input(&file);
            // Validate locally before sending.
            let unit = match serde_json::from_str::<SemanticUnit>(&json) {
                Ok(u) => u,
                Err(e) => fatal(&format!("failed to parse unit: {e}")),
            };
            if let Err(e) = validate_unit(&unit) {
                fatal(&format!("unit is invalid: {e}"));
            }

            let url = format!("{}/v1/units", node.trim_end_matches('/'));
            let client = reqwest::blocking::Client::new();
            let mut builder = client
                .post(&url)
                .header("content-type", "application/json")
                .json(&unit);

            if let Some(tok) = token {
                builder = builder.header("authorization", format!("Bearer {tok}"));
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
            token,
            unit_type,
            author,
            since,
            after,
            limit,
        } => {
            let client = reqwest::blocking::Client::new();
            let node = node.trim_end_matches('/');

            let url = if let Some(ref uid) = id {
                format!("{}/v1/units/{}", node, urlencoded(uid))
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

                format!("{}/v1/units{}", node, qs)
            };

            let mut builder = client.get(&url);
            if let Some(tok) = token {
                builder = builder.header("authorization", format!("Bearer {tok}"));
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
