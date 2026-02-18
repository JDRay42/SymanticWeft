//! Spec-conformance validation for [`SemanticUnit`]s.
//!
//! The single public entry point is [`validate_unit`], which checks a unit
//! against every normative rule in `spec/semantic-unit.md §8`. Errors are
//! returned in field order so that the first failure is always predictable and
//! easy to act on.
//!
//! Validation is structural and syntactic. Semantic consistency (e.g. whether
//! a `challenge` unit's content is actually about its referenced unit) is not
//! enforced here and is left to reasoning layers.

use std::sync::LazyLock;

use regex::Regex;
use thiserror::Error;

use crate::types::SemanticUnit;

/// Errors returned when a [`SemanticUnit`] fails conformance validation.
///
/// Variants are ordered to match the field order defined in
/// `spec/semantic-unit.md §8`, so callers can rely on stable error sequencing.
#[derive(Debug, Error, PartialEq)]
pub enum ValidationError {
    #[error("id must be a valid UUIDv7 (RFC 9562), got: {0:?}")]
    InvalidId(String),

    #[error("content must not be empty")]
    EmptyContent,

    #[error("author must not be empty")]
    EmptyAuthor,

    #[error("created_at must be a valid ISO 8601 date-time, got: {0:?}")]
    InvalidTimestamp(String),

    #[error("confidence must be between 0.0 and 1.0 inclusive, got {0}")]
    InvalidConfidence(f64),

    #[error("assumptions must contain at least one item when present")]
    EmptyAssumptions,

    #[error("assumption at index {0} must not be empty")]
    EmptyAssumptionItem(usize),

    #[error("references must contain at least one item when present")]
    EmptyReferences,

    #[error("reference id at index {0} must be a valid UUIDv7, got: {1:?}")]
    InvalidReferenceId(usize, String),

    #[error(
        "extension field {0:?} is invalid; names must match x-<reverse-domain>.<name> \
         (e.g. x-com.example.myfield) and be lowercase"
    )]
    InvalidExtensionFieldName(String),
}

/// Validate a [`SemanticUnit`] against the normative specification (§8).
///
/// Returns `Ok(())` if the unit is fully conformant, or the first
/// [`ValidationError`] found. Errors are returned in field order as defined
/// by the spec.
pub fn validate_unit(unit: &SemanticUnit) -> Result<(), ValidationError> {
    // §3.1 — id must be a valid UUIDv7.
    validate_uuid_v7(&unit.id)
        .map_err(|_| ValidationError::InvalidId(unit.id.clone()))?;

    // §3.3 — content must not be empty.
    if unit.content.is_empty() {
        return Err(ValidationError::EmptyContent);
    }

    // §3.5 — author must not be empty.
    if unit.author.is_empty() {
        return Err(ValidationError::EmptyAuthor);
    }

    // §3.4 — created_at must be a valid ISO 8601 date-time.
    validate_timestamp(&unit.created_at)
        .map_err(|_| ValidationError::InvalidTimestamp(unit.created_at.clone()))?;

    // §4.1 — confidence, if present, must be in [0.0, 1.0].
    if let Some(c) = unit.confidence {
        if !(0.0..=1.0).contains(&c) {
            return Err(ValidationError::InvalidConfidence(c));
        }
    }

    // §4.2 — assumptions array must be non-empty and contain non-empty strings.
    if let Some(assumptions) = &unit.assumptions {
        if assumptions.is_empty() {
            return Err(ValidationError::EmptyAssumptions);
        }
        for (i, a) in assumptions.iter().enumerate() {
            if a.is_empty() {
                return Err(ValidationError::EmptyAssumptionItem(i));
            }
        }
    }

    // §4.4 — references array must be non-empty; each ref id must be UUIDv7.
    if let Some(references) = &unit.references {
        if references.is_empty() {
            return Err(ValidationError::EmptyReferences);
        }
        for (i, r) in references.iter().enumerate() {
            validate_uuid_v7(&r.id)
                .map_err(|_| ValidationError::InvalidReferenceId(i, r.id.clone()))?;
        }
    }

    // §6 — extension field names must match the x-<reverse-domain>.<name> pattern.
    for key in unit.extensions.keys() {
        if !EXTENSION_RE.is_match(key) {
            return Err(ValidationError::InvalidExtensionFieldName(key.clone()));
        }
    }

    Ok(())
}

// --- helpers -----------------------------------------------------------------

/// Returns `Ok(())` if `s` is a valid UUIDv7 string, otherwise `Err(())`.
fn validate_uuid_v7(s: &str) -> Result<(), ()> {
    match uuid::Uuid::parse_str(s) {
        Ok(u) if u.get_version_num() == 7 => Ok(()),
        _ => Err(()),
    }
}

/// Returns `Ok(())` if `s` is a valid RFC 3339 / ISO 8601 date-time string.
fn validate_timestamp(s: &str) -> Result<(), ()> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|_| ())
        .map_err(|_| ())
}

/// Compiled regex for valid extension field names (spec §6):
/// `^x-<reverse-domain>.<name>$` — all lowercase, at least two dot-separated
/// segments after the `x-` prefix.
///
/// Examples of valid names:
/// - `x-com.example.myfield`
/// - `x-org.semanticweft.priority`
static EXTENSION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^x-[a-z0-9]+(\.[a-z0-9]+)+$").expect("invalid extension regex")
});

// --- tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Reference, RelType, SemanticUnit, Source, UnitType};
    use std::collections::HashMap;

    fn minimal() -> SemanticUnit {
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
            extensions: HashMap::new(),
        }
    }

    #[test]
    fn valid_minimal_unit() {
        assert_eq!(validate_unit(&minimal()), Ok(()));
    }

    #[test]
    fn invalid_id_not_uuid() {
        let mut u = minimal();
        u.id = "not-a-uuid".into();
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidId(_))));
    }

    #[test]
    fn invalid_id_wrong_version() {
        let mut u = minimal();
        // UUIDv4
        u.id = "550e8400-e29b-41d4-a716-446655440000".into();
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidId(_))));
    }

    #[test]
    fn empty_content_rejected() {
        let mut u = minimal();
        u.content = String::new();
        assert_eq!(validate_unit(&u), Err(ValidationError::EmptyContent));
    }

    #[test]
    fn empty_author_rejected() {
        let mut u = minimal();
        u.author = String::new();
        assert_eq!(validate_unit(&u), Err(ValidationError::EmptyAuthor));
    }

    #[test]
    fn invalid_timestamp() {
        let mut u = minimal();
        u.created_at = "not-a-date".into();
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidTimestamp(_))));
    }

    #[test]
    fn confidence_out_of_range() {
        let mut u = minimal();
        u.confidence = Some(1.1);
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidConfidence(_))));
        u.confidence = Some(-0.1);
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidConfidence(_))));
    }

    #[test]
    fn confidence_boundary_values_valid() {
        let mut u = minimal();
        u.confidence = Some(0.0);
        assert_eq!(validate_unit(&u), Ok(()));
        u.confidence = Some(1.0);
        assert_eq!(validate_unit(&u), Ok(()));
    }

    #[test]
    fn empty_assumptions_array_rejected() {
        let mut u = minimal();
        u.assumptions = Some(vec![]);
        assert_eq!(validate_unit(&u), Err(ValidationError::EmptyAssumptions));
    }

    #[test]
    fn empty_string_in_assumptions_rejected() {
        let mut u = minimal();
        u.assumptions = Some(vec!["valid".into(), String::new()]);
        assert_eq!(validate_unit(&u), Err(ValidationError::EmptyAssumptionItem(1)));
    }

    #[test]
    fn empty_references_array_rejected() {
        let mut u = minimal();
        u.references = Some(vec![]);
        assert_eq!(validate_unit(&u), Err(ValidationError::EmptyReferences));
    }

    #[test]
    fn invalid_reference_id_rejected() {
        let mut u = minimal();
        u.references = Some(vec![Reference {
            id: "not-a-uuid".into(),
            rel: RelType::Supports,
        }]);
        assert!(matches!(validate_unit(&u), Err(ValidationError::InvalidReferenceId(0, _))));
    }

    #[test]
    fn valid_reference() {
        let mut u = minimal();
        u.references = Some(vec![Reference {
            id: "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d".into(),
            rel: RelType::DerivesFrom,
        }]);
        assert_eq!(validate_unit(&u), Ok(()));
    }

    #[test]
    fn invalid_extension_field_name() {
        let mut u = minimal();
        u.extensions.insert("foo".into(), serde_json::json!("bar"));
        assert!(matches!(
            validate_unit(&u),
            Err(ValidationError::InvalidExtensionFieldName(_))
        ));
    }

    #[test]
    fn valid_extension_field() {
        let mut u = minimal();
        u.extensions
            .insert("x-org.semanticweft.priority".into(), serde_json::json!("high"));
        assert_eq!(validate_unit(&u), Ok(()));
    }

    #[test]
    fn roundtrip_json() {
        let json = r#"{
            "id": "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6c",
            "type": "assertion",
            "content": "The boiling point of water at sea level is 100°C.",
            "created_at": "2026-02-18T12:00:00Z",
            "author": "agent-weathersim-v2"
        }"#;
        let unit: SemanticUnit = serde_json::from_str(json).unwrap();
        assert_eq!(validate_unit(&unit), Ok(()));
        let re = serde_json::to_string(&unit).unwrap();
        let unit2: SemanticUnit = serde_json::from_str(&re).unwrap();
        assert_eq!(unit2.id, unit.id);
    }

    #[test]
    fn roundtrip_labeled_source() {
        let json = r#"{
            "id": "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b6d",
            "type": "assertion",
            "content": "Global mean surface temperature in 2025 was 1.4°C above baseline.",
            "created_at": "2026-02-18T12:01:00Z",
            "author": "agent-climatesynthesizer",
            "confidence": 0.91,
            "source": {
                "label": "WMO Global Climate Report 2025",
                "uri": "https://wmo.int/reports/global-climate-2025"
            }
        }"#;
        let unit: SemanticUnit = serde_json::from_str(json).unwrap();
        assert_eq!(validate_unit(&unit), Ok(()));
        assert!(matches!(unit.source, Some(Source::Labeled { .. })));
    }

    #[test]
    fn roundtrip_extension_field() {
        let json = r#"{
            "id": "019526b2-f68a-7c3e-a0b4-1d2e3f4a5b70",
            "type": "constraint",
            "content": "All temperature values must be in degrees Celsius.",
            "created_at": "2026-02-18T12:04:00Z",
            "author": "agent-graphsupervisor",
            "x-org.semanticweft.priority": "high"
        }"#;
        let unit: SemanticUnit = serde_json::from_str(json).unwrap();
        assert_eq!(validate_unit(&unit), Ok(()));
        assert_eq!(
            unit.extensions.get("x-org.semanticweft.priority"),
            Some(&serde_json::json!("high"))
        );
    }
}
