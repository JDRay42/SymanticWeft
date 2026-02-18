//! Human-readable text rendering of [`SemanticUnit`]s and [`Graph`]s.
//!
//! This is the "expression layer" described in the spec: natural language is
//! generated *from* structured units, not used as the source of truth.
//!
//! The output is stable plain text suitable for terminals, logs, or embedding
//! in a human-facing context. It is not a canonical format — implementations
//! may render differently. Only the JSON wire format is normative.

use crate::graph::Graph;
use crate::types::{SemanticUnit, Source, UnitType};

/// Render a single [`SemanticUnit`] as indented plain text.
///
/// ```text
/// [inference] agent-climatesynthesizer  confidence: 0.74
/// "At the observed rate of temperature increase, the 1.5°C threshold will be
///  exceeded before 2035."
///
/// Assumptions:
///   • Current emissions trajectories continue without significant policy intervention.
///   • Climate sensitivity remains within the IPCC AR6 likely range.
///
/// References:
///   derives-from  019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d
///
/// id: 019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6e  created: 2026-02-18T12:02:00Z
/// ```
pub fn render_unit(unit: &SemanticUnit) -> String {
    let mut out = String::new();

    // header line
    let confidence = unit
        .confidence
        .map(|c| format!("  confidence: {:.2}", c))
        .unwrap_or_default();
    out.push_str(&format!(
        "[{}] {}{}",
        unit.unit_type, unit.author, confidence
    ));
    out.push('\n');

    // content (wrapped at 80 chars, second+ lines indented by 1 space)
    out.push_str(&wrap_content(&unit.content, 80));
    out.push('\n');

    // source
    if let Some(source) = &unit.source {
        out.push('\n');
        match source {
            Source::Uri(uri) => out.push_str(&format!("Source: {}", uri)),
            Source::Labeled { label, uri } => match uri {
                Some(u) => out.push_str(&format!("Source: {} <{}>", label, u)),
                None => out.push_str(&format!("Source: {}", label)),
            },
        }
        out.push('\n');
    }

    // assumptions
    if let Some(assumptions) = &unit.assumptions {
        out.push('\n');
        out.push_str("Assumptions:\n");
        for a in assumptions {
            out.push_str(&format!("  • {}\n", a));
        }
    }

    // references
    if let Some(references) = &unit.references {
        out.push('\n');
        out.push_str("References:\n");
        for r in references {
            out.push_str(&format!("  {}  {}\n", r.rel, r.id));
        }
    }

    // extension fields
    if !unit.extensions.is_empty() {
        out.push('\n');
        out.push_str("Extensions:\n");
        let mut keys: Vec<&String> = unit.extensions.keys().collect();
        keys.sort();
        for k in keys {
            out.push_str(&format!("  {}: {}\n", k, unit.extensions[k]));
        }
    }

    // footer
    out.push('\n');
    out.push_str(&format!("id: {}  created: {}\n", unit.id, unit.created_at));

    out
}

/// Render an entire [`Graph`] as a summary grouped by unit type.
///
/// ```text
/// SemanticWeft Graph  3 units
/// ───────────────────────────
///
/// ASSERTIONS (1)
///   [019...] "Global mean surface temperature in 2025 was 1.4°C above baseline."
///
/// INFERENCES (1)
///   [019...] "At the observed rate of temperature increase, the 1.5°C threshold..."
///
/// CHALLENGES (1)
///   [019...] "The 2035 threshold projection assumes linear extrapolation..."
/// ```
pub fn render_graph(graph: &Graph) -> String {
    let total = graph.len();
    let header = format!("SemanticWeft Graph  {} unit{}", total, if total == 1 { "" } else { "s" });
    let rule = "─".repeat(header.len());

    let mut out = format!("{}\n{}\n", header, rule);

    let groups = [
        (UnitType::Assertion, "ASSERTIONS"),
        (UnitType::Question, "QUESTIONS"),
        (UnitType::Inference, "INFERENCES"),
        (UnitType::Challenge, "CHALLENGES"),
        (UnitType::Constraint, "CONSTRAINTS"),
    ];

    for (unit_type, label) in &groups {
        let mut units: Vec<&SemanticUnit> = graph.by_type(unit_type);
        if units.is_empty() {
            continue;
        }
        // Sort by id (UUIDv7 → lexicographic ≈ chronological).
        units.sort_by_key(|u| u.id.as_str());
        out.push('\n');
        out.push_str(&format!("{} ({})\n", label, units.len()));
        for u in units {
            let excerpt = truncate(&u.content, 72);
            out.push_str(&format!("  [{}]  \"{}\"\n", short_id(&u.id), excerpt));
        }
    }

    out
}

// --- helpers -----------------------------------------------------------------

fn wrap_content(content: &str, width: usize) -> String {
    if content.len() <= width {
        return format!("\"{}\"", content);
    }
    let mut result = String::from("\"");
    let mut line_len = 1usize;
    for word in content.split_whitespace() {
        if line_len + word.len() + 1 > width {
            result.push('\n');
            result.push(' ');
            line_len = 1;
        } else if line_len > 1 {
            result.push(' ');
            line_len += 1;
        }
        result.push_str(word);
        line_len += word.len();
    }
    result.push('"');
    result
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        s.to_string()
    } else {
        // truncate at a character boundary
        let boundary = s
            .char_indices()
            .take_while(|(i, _)| *i < max - 1)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(max);
        format!("{}…", &s[..boundary])
    }
}

fn short_id(id: &str) -> &str {
    // first 8 hex chars
    &id[..8.min(id.len())]
}

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UnitType;
    use std::collections::HashMap;

    fn minimal_unit() -> SemanticUnit {
        SemanticUnit {
            id: "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c".into(),
            unit_type: UnitType::Assertion,
            content: "The boiling point of water at sea level is 100°C.".into(),
            created_at: "2026-02-18T12:00:00Z".into(),
            author: "agent-weathersim-v2".into(),
            confidence: None,
            assumptions: None,
            source: None,
            references: None,
            visibility: None,
            audience: None,
            extensions: HashMap::new(),
        }
    }

    #[test]
    fn render_unit_contains_key_fields() {
        let rendered = render_unit(&minimal_unit());
        assert!(rendered.contains("[assertion]"));
        assert!(rendered.contains("agent-weathersim-v2"));
        assert!(rendered.contains("boiling point"));
        assert!(rendered.contains("019526b2"));
    }

    #[test]
    fn render_graph_groups_by_type() {
        let mut g = Graph::new();
        g.add(minimal_unit());
        let rendered = render_graph(&g);
        assert!(rendered.contains("ASSERTIONS (1)"));
        assert!(rendered.contains("1 unit"));
    }
}
