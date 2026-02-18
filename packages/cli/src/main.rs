//! `sweft` — SemanticWeft protocol command-line interface.
//!
//! Provides three subcommands for working with Semantic Units on the command
//! line:
//!
//! - **`validate`** — check a unit or array of units against the spec.
//! - **`render`** — print a human-readable summary of a unit or graph.
//! - **`new`** — create a new unit with an auto-generated id and timestamp.
//!
//! All subcommands read JSON from a file path or from stdin (`-`).

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
    }
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
