use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use semanticweft::{validate_unit, Graph, SemanticUnit};

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
            if units.len() == 1 {
                print!("{}", semanticweft::render::render_unit(&units[0]));
            } else {
                let graph = Graph::from_units(units);
                print!("{}", semanticweft::render::render_graph(&graph));
            }
        }
    }
}

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

fn fatal(msg: &str) -> ! {
    eprintln!("sweft: {}", msg);
    process::exit(2);
}
